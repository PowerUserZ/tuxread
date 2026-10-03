//! Copy jobs (spec §5.5, §6.2). Each runs on its own thread with its own filesystem instance
//! over the volume's shared device, so browsing stays responsive during long copies.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tuxread_core::copy::{Conflict, Outcome, Progress, Report, copy_out};
use tuxread_core::probe::Volume;

use crate::display::display_name;
use crate::error::{CmdResult, Code, CommandError};
use crate::sources::lock;

/// Progress reaches the window at most 10 times a second (spec §5.5).
pub const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Lets an event through at most once per `every`.
pub struct Throttle {
    every: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(every: Duration) -> Self {
        Self { every, last: None }
    }

    pub fn ready(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if now.saturating_duration_since(last) < self.every => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum JobEvent {
    Progress {
        files: u64,
        bytes: u64,
        current: String,
    },
    /// The last event of a job. `error` is set when the copy could not start at all.
    Finished {
        copied: usize,
        renamed: usize,
        skipped: usize,
        failed: usize,
        cancelled: bool,
        error: Option<CommandError>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportItemView {
    pub source: String,
    pub dest: Option<String>,
    /// "copied", "renamed", "skipped" or "failed".
    pub outcome: &'static str,
    /// The new name, or why it was skipped or failed.
    pub detail: Option<String>,
}

struct Job {
    cancel: Arc<AtomicBool>,
    dest: PathBuf,
    report: Option<Report>,
}

#[derive(Default)]
pub struct Jobs {
    next: AtomicU32,
    jobs: Arc<Mutex<HashMap<u32, Job>>>,
}

impl Jobs {
    /// Starts copying `sources` (raw paths on `volume`) into `dest`; `send` receives the
    /// job's events, ending with `Finished`. Returns the job's number.
    pub fn start(
        &self,
        volume: Volume,
        sources: Vec<Vec<u8>>,
        dest: PathBuf,
        conflict: Conflict,
        send: impl Fn(JobEvent) + Send + 'static,
    ) -> CmdResult<u32> {
        // Checked here, not per file: otherwise a typo fails every file, and a relative path
        // would land wherever the app happens to run.
        if !dest.is_absolute() || !dest.is_dir() {
            return Err(CommandError::new(
                Code::NotADirectory,
                format!(
                    "the destination is not an existing folder: {}",
                    dest.display()
                ),
            ));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        lock(&self.jobs).insert(
            id,
            Job {
                cancel: Arc::clone(&cancel),
                dest: dest.clone(),
                report: None,
            },
        );
        let jobs = Arc::clone(&self.jobs);
        std::thread::Builder::new()
            .name("copy".into())
            .spawn(move || {
                let (report, error) = match volume.open() {
                    Ok(fs) => {
                        let mut throttle = Throttle::new(PROGRESS_EVERY);
                        let mut on_progress = |p: &Progress| {
                            if throttle.ready(Instant::now()) {
                                send(progress_event(p));
                            }
                        };
                        let report = copy_out(
                            fs.as_ref(),
                            &sources,
                            &dest,
                            conflict,
                            &cancel,
                            &mut on_progress,
                        );
                        (report, None)
                    }
                    Err(e) => (Report::default(), Some(CommandError::from(e))),
                };
                let finished = finished_event(&report, error);
                // The report is kept before `Finished` goes out, so the window can ask for it.
                if let Some(job) = lock(&jobs).get_mut(&id) {
                    job.report = Some(report);
                }
                send(finished);
            })?;
        Ok(id)
    }

    pub fn cancel(&self, id: u32) -> CmdResult<()> {
        let jobs = lock(&self.jobs);
        let job = jobs.get(&id).ok_or_else(|| no_job(id))?;
        job.cancel.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn report(&self, id: u32) -> CmdResult<Vec<ReportItemView>> {
        self.with_report(id, |_, report| report.items.iter().map(item_view).collect())
    }

    pub fn save_report(&self, id: u32, path: &Path) -> CmdResult<()> {
        let text = self.with_report(id, report_text)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    fn with_report<R>(&self, id: u32, f: impl FnOnce(&Path, &Report) -> R) -> CmdResult<R> {
        let jobs = lock(&self.jobs);
        let job = jobs.get(&id).ok_or_else(|| no_job(id))?;
        let report = job
            .report
            .as_ref()
            .ok_or_else(|| CommandError::new(Code::Other, "the copy is still running"))?;
        Ok(f(&job.dest, report))
    }
}

fn no_job(id: u32) -> CommandError {
    CommandError::new(Code::NotFound, format!("no copy job #{id}"))
}

fn progress_event(p: &Progress) -> JobEvent {
    JobEvent::Progress {
        files: p.files,
        bytes: p.bytes,
        current: display_name(p.current.as_bytes()),
    }
}

/// Copied, renamed, skipped and failed items.
fn counts(report: &Report) -> [usize; 4] {
    let mut counts = [0; 4];
    for item in &report.items {
        let i = match item.outcome {
            Outcome::Copied => 0,
            Outcome::Renamed { .. } => 1,
            Outcome::Skipped { .. } => 2,
            Outcome::Failed { .. } => 3,
        };
        if let Some(n) = counts.get_mut(i) {
            *n += 1;
        }
    }
    counts
}

fn finished_event(report: &Report, error: Option<CommandError>) -> JobEvent {
    let [copied, renamed, skipped, failed] = counts(report);
    JobEvent::Finished {
        copied,
        renamed,
        skipped,
        failed,
        cancelled: report.cancelled,
        error,
    }
}

fn item_view(item: &tuxread_core::copy::ReportItem) -> ReportItemView {
    let (outcome, detail) = match &item.outcome {
        Outcome::Copied => ("copied", None),
        Outcome::Renamed { to } => ("renamed", Some(to.clone())),
        Outcome::Skipped { reason } => ("skipped", Some(reason.clone())),
        Outcome::Failed { reason } => ("failed", Some(reason.clone())),
    };
    ReportItemView {
        source: display_name(item.source.as_bytes()),
        dest: item.dest.as_ref().map(|d| d.display().to_string()),
        outcome,
        detail: detail.map(|d| display_name(d.as_bytes())),
    }
}

/// The report as text for a file (spec §6.2), with Windows line endings.
pub fn report_text(dest: &Path, report: &Report) -> String {
    let [copied, renamed, skipped, failed] = counts(report);
    let mut out = String::new();
    let _ = write!(
        out,
        "TuxRead copy report\r\nDestination: {}\r\n{copied} copied, {renamed} renamed, {skipped} skipped, {failed} failed{}\r\n\r\n",
        dest.display(),
        if report.cancelled { " (cancelled)" } else { "" }
    );
    for item in report.items.iter().map(item_view) {
        let detail = item.detail.unwrap_or_default();
        let line = match item.outcome {
            "copied" => format!("Copied   {}", item.source),
            "renamed" => format!("Renamed  {} -> {detail}", item.source),
            "skipped" => format!("Skipped  {} ({detail})", item.source),
            _ => format!("Failed   {} ({detail})", item.source),
        };
        out.push_str(&line);
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::sources::App;
    use crate::worker::tests::tiny_image;

    fn tiny_volume(app: &App) -> Volume {
        let view = app.open_image(&tiny_image()).unwrap();
        let id = view.nodes[0].volume.unwrap();
        app.volume(id).unwrap().volume.clone()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuxread-job-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn progress_is_let_through_at_most_once_per_interval() {
        let mut throttle = Throttle::new(PROGRESS_EVERY);
        let t0 = Instant::now();
        assert!(throttle.ready(t0));
        assert!(!throttle.ready(t0 + Duration::from_millis(99)));
        assert!(throttle.ready(t0 + Duration::from_millis(100)));
        assert!(!throttle.ready(t0 + Duration::from_millis(150)));
    }

    #[test]
    fn a_job_copies_reports_and_ends_with_finished() {
        let app = App::default();
        let jobs = Jobs::default();
        let dest = temp_dir("copy");
        let (tx, rx) = mpsc::channel();
        let id = jobs
            .start(
                tiny_volume(&app),
                vec![b"/".to_vec()],
                dest.clone(),
                Conflict::KeepBoth,
                move |e| {
                    let _ = tx.send(e);
                },
            )
            .unwrap();
        let events: Vec<JobEvent> = rx.iter().collect();
        assert_eq!(
            events.last().unwrap(),
            &JobEvent::Finished {
                copied: 4,
                renamed: 0,
                skipped: 1,
                failed: 0,
                cancelled: false,
                error: None
            }
        );
        assert_eq!(std::fs::read(dest.join("dir/b.txt")).unwrap(), b"beta\n");
        let report = jobs.report(id).unwrap();
        let link = report.iter().find(|i| i.source == "/link").unwrap();
        assert_eq!(
            (link.outcome, link.detail.as_deref()),
            ("skipped", Some("symbolic link -> a.txt"))
        );
        let saved = dest.join("report.txt");
        jobs.save_report(id, &saved).unwrap();
        let text = std::fs::read_to_string(&saved).unwrap();
        assert!(
            text.contains("4 copied, 0 renamed, 1 skipped, 0 failed\r\n"),
            "{text}"
        );
        assert!(
            text.contains("Skipped  /link (symbolic link -> a.txt)\r\n"),
            "{text}"
        );
    }

    #[test]
    fn a_cancelled_job_says_so_and_unknown_jobs_are_not_found() {
        let app = App::default();
        let jobs = Jobs::default();
        let (tx, rx) = mpsc::channel();
        // The first progress event waits until the job has been cancelled.
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let wait = Arc::clone(&gate);
        let id = jobs
            .start(
                tiny_volume(&app),
                vec![b"/".to_vec()],
                temp_dir("cancel"),
                Conflict::KeepBoth,
                move |e| {
                    let _guard = wait.lock();
                    let _ = tx.send(e);
                },
            )
            .unwrap();
        jobs.cancel(id).unwrap();
        drop(held);
        let last = rx.iter().last().unwrap();
        assert!(
            matches!(
                last,
                JobEvent::Finished {
                    cancelled: true,
                    ..
                }
            ),
            "{last:?}"
        );
        assert_eq!(jobs.cancel(999).unwrap_err().code, Code::NotFound);
        assert_eq!(jobs.report(999).unwrap_err().code, Code::NotFound);
    }

    #[test]
    fn the_destination_must_be_an_existing_folder() {
        let app = App::default();
        let jobs = Jobs::default();
        let dir = temp_dir("dest");
        let file = dir.join("a file");
        std::fs::write(&file, b"x").unwrap();
        for dest in [dir.join("missing"), file, PathBuf::from("relative")] {
            let err = jobs
                .start(
                    tiny_volume(&app),
                    vec![b"/".to_vec()],
                    dest.clone(),
                    Conflict::KeepBoth,
                    |_| {},
                )
                .unwrap_err();
            assert_eq!(err.code, Code::NotADirectory, "{}", dest.display());
        }
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "nothing was created"
        );
    }
}
