//! Copying files out of a Linux filesystem into a Windows folder (spec §6.2).

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::fs::{Fs, Kind, Timestamp, join};
use crate::sanitize::sanitize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Copy under "name (2).ext" (default).
    KeepBoth,
    Skip,
    /// Replace files; merge into existing folders.
    Overwrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Copied,
    /// Copied under a different name: sanitized or de-duplicated.
    Renamed {
        to: String,
    },
    Skipped {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportItem {
    /// Linux path, lossily decoded for display.
    pub source: String,
    pub dest: Option<PathBuf>,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub items: Vec<ReportItem>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub files: u64,
    pub bytes: u64,
    /// Linux path being copied, lossily decoded.
    pub current: String,
}

const CHUNK: usize = 1 << 20;
const MAX_DEPTH: usize = 1024;

/// Copies `sources` (absolute Linux paths) into `dest_dir`, recursing into folders.
/// Per-file failures are recorded and the copy goes on; `cancel` stops it between chunks.
pub fn copy_out(
    fs: &dyn Fs,
    sources: &[Vec<u8>],
    dest_dir: &Path,
    conflict: Conflict,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(&Progress),
) -> Report {
    let mut copier = Copier {
        fs,
        conflict,
        cancel,
        on_progress,
        progress: Progress::default(),
        report: Report::default(),
    };
    let mut taken = Taken::new(dest_dir);
    for src in sources {
        if copier.cancelled() {
            break;
        }
        if src.iter().all(|&b| b == b'/') {
            // The root has no name of its own: copy what it contains.
            copier.copy_children(src, dest_dir, 0);
        } else {
            copier.copy_entry(src, dest_dir, &mut taken, 0);
        }
    }
    copier.report
}

/// Names in one destination folder, compared the way Windows does (case-insensitively).
struct Taken {
    existing: HashSet<String>,
    created: HashSet<String>,
}

impl Taken {
    fn new(dir: &Path) -> Self {
        let existing = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_uppercase())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            existing,
            created: HashSet::new(),
        }
    }

    fn is_free(&self, name: &str) -> bool {
        let key = name.to_uppercase();
        !self.existing.contains(&key) && !self.created.contains(&key)
    }

    /// "name (2).ext", "name (3).ext", ... — the first one not taken.
    fn unique(&self, name: &str, is_dir: bool) -> String {
        let (stem, ext) = match name.rfind('.') {
            Some(dot) if dot > 0 && !is_dir => name.split_at(dot),
            _ => (name, ""),
        };
        (2u64..)
            .map(|n| sanitize(format!("{stem} ({n}){ext}").as_bytes()))
            .find(|candidate| self.is_free(candidate))
            .unwrap_or_else(|| name.to_string())
    }
}

/// Where an entry goes: the final name, and whether it reuses an existing item (Overwrite).
enum Plan {
    Use { name: String, reuse: bool },
    Skip(String),
}

struct Copier<'a> {
    fs: &'a dyn Fs,
    conflict: Conflict,
    cancel: &'a AtomicBool,
    on_progress: &'a mut dyn FnMut(&Progress),
    progress: Progress,
    report: Report,
}

impl Copier<'_> {
    fn cancelled(&mut self) -> bool {
        if self.cancel.load(Ordering::Relaxed) {
            self.report.cancelled = true;
        }
        self.report.cancelled
    }

    fn record(&mut self, source: String, dest: Option<PathBuf>, outcome: Outcome) {
        self.report.items.push(ReportItem {
            source,
            dest,
            outcome,
        });
    }

    fn plan(&self, raw_name: &[u8], is_dir: bool, taken: &Taken) -> Plan {
        let wanted = sanitize(raw_name);
        let key = wanted.to_uppercase();
        if taken.created.contains(&key) {
            // Two Linux names became one Windows name ("File" / "file"): always keep both.
            return Plan::Use {
                name: taken.unique(&wanted, is_dir),
                reuse: false,
            };
        }
        if !taken.existing.contains(&key) {
            return Plan::Use {
                name: wanted,
                reuse: false,
            };
        }
        match self.conflict {
            Conflict::KeepBoth => Plan::Use {
                name: taken.unique(&wanted, is_dir),
                reuse: false,
            },
            Conflict::Skip => Plan::Skip("already exists in the destination".into()),
            Conflict::Overwrite => Plan::Use {
                name: wanted,
                reuse: true,
            },
        }
    }

    fn copy_entry(&mut self, src: &[u8], dest_dir: &Path, taken: &mut Taken, depth: usize) {
        let shown = String::from_utf8_lossy(src).into_owned();
        self.progress.current.clone_from(&shown);
        if depth > MAX_DEPTH {
            return self.record(
                shown,
                None,
                Outcome::Failed {
                    reason: "folders nested too deeply".into(),
                },
            );
        }
        let entry = match self.fs.stat(src) {
            Ok(entry) => entry,
            Err(e) => {
                return self.record(
                    shown,
                    None,
                    Outcome::Failed {
                        reason: e.to_string(),
                    },
                );
            }
        };
        match entry.kind {
            Kind::Symlink => {
                let target = self
                    .fs
                    .read_link(src)
                    .map(|t| String::from_utf8_lossy(&t).into_owned());
                let reason = format!(
                    "symbolic link -> {}",
                    target.unwrap_or_else(|e| e.to_string())
                );
                return self.record(shown, None, Outcome::Skipped { reason });
            }
            Kind::Other => {
                return self.record(
                    shown,
                    None,
                    Outcome::Skipped {
                        reason: "device, FIFO or socket".into(),
                    },
                );
            }
            Kind::File | Kind::Dir => {}
        }
        let is_dir = entry.kind == Kind::Dir;
        let (name, reuse) = match self.plan(&entry.name, is_dir, taken) {
            Plan::Use { name, reuse } => (name, reuse),
            Plan::Skip(reason) => return self.record(shown, None, Outcome::Skipped { reason }),
        };
        taken.created.insert(name.to_uppercase());
        let dest = dest_dir.join(&name);
        let renamed = name != String::from_utf8_lossy(&entry.name);
        let ok = if renamed {
            Outcome::Renamed { to: name }
        } else {
            Outcome::Copied
        };

        if is_dir {
            if !reuse && let Err(e) = std::fs::create_dir(&dest) {
                return self.record(
                    shown,
                    Some(dest),
                    Outcome::Failed {
                        reason: e.to_string(),
                    },
                );
            }
            self.record(shown, Some(dest.clone()), ok);
            self.copy_children(src, &dest, depth);
        } else {
            let outcome = match self.copy_file(src, &dest, reuse, entry.mtime) {
                Ok(true) => ok,
                Ok(false) => Outcome::Skipped {
                    reason: "cancelled".into(),
                },
                Err(e) => Outcome::Failed {
                    reason: e.to_string(),
                },
            };
            self.record(shown, Some(dest), outcome);
        }
    }

    /// Copies the entries of folder `src` into `dest`.
    fn copy_children(&mut self, src: &[u8], dest: &Path, depth: usize) {
        let mut children = match self.fs.read_dir(src) {
            Ok(children) => children,
            Err(e) => {
                let source = String::from_utf8_lossy(src).into_owned();
                return self.record(
                    source,
                    Some(dest.to_path_buf()),
                    Outcome::Failed {
                        reason: e.to_string(),
                    },
                );
            }
        };
        children.sort_by(|a, b| a.name.cmp(&b.name));
        let mut taken = Taken::new(dest);
        for child in children {
            if self.cancelled() {
                return;
            }
            self.copy_entry(&join(src, &child.name), dest, &mut taken, depth + 1);
        }
    }

    /// `Ok(false)` when cancelled; the partial file is removed.
    fn copy_file(
        &mut self,
        src: &[u8],
        dest: &Path,
        replace: bool,
        mtime: Option<Timestamp>,
    ) -> io::Result<bool> {
        let mut reader = self
            .fs
            .open(src)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let mut options = OpenOptions::new();
        options.write(true);
        if replace {
            options.create(true).truncate(true)
        } else {
            options.create_new(true)
        };
        let mut file = options.open(dest)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if self.cancelled() {
                drop(file);
                std::fs::remove_file(dest)?;
                return Ok(false);
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    drop(file);
                    let _ = std::fs::remove_file(dest); // keep the read error, not a cleanup error
                    return Err(e);
                }
            };
            file.write_all(buf.get(..n).unwrap_or_default())?;
            self.progress.bytes += n as u64;
            (self.on_progress)(&self.progress);
        }
        if let Some(time) = mtime.and_then(system_time) {
            // NTFS cannot store some Linux times (e.g. before 1601); the data is what matters.
            let _ = file.set_modified(time);
        }
        self.progress.files += 1;
        (self.on_progress)(&self.progress);
        Ok(true)
    }
}

fn system_time(t: Timestamp) -> Option<SystemTime> {
    let base = if t.secs >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(t.secs.unsigned_abs()))?
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(t.secs.unsigned_abs()))?
    };
    base.checked_add(Duration::from_nanos(u64::from(t.nanos)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{Entry, FsInfo};
    use std::collections::BTreeMap;

    /// In-memory filesystem: path -> (entry, file content or link target).
    struct MemFs {
        info: FsInfo,
        nodes: BTreeMap<Vec<u8>, (Entry, Vec<u8>)>,
    }

    impl MemFs {
        fn new() -> Self {
            let mut fs = Self {
                info: FsInfo::default(),
                nodes: BTreeMap::new(),
            };
            fs.add(b"/", Kind::Dir, b"");
            fs
        }

        fn add(&mut self, path: &[u8], kind: Kind, data: &[u8]) {
            let name = path.rsplit(|&b| b == b'/').next().unwrap().to_vec();
            let mtime = Some(Timestamp {
                secs: 981_173_106,
                nanos: 0,
            }); // 2001-02-03T04:05:06Z
            let entry = Entry {
                name,
                kind,
                size: data.len() as u64,
                mtime,
                mode: 0o644,
                uid: 0,
                gid: 0,
            };
            self.nodes.insert(path.to_vec(), (entry, data.to_vec()));
        }

        fn get(&self, path: &[u8]) -> crate::Result<&(Entry, Vec<u8>)> {
            self.nodes.get(path).ok_or(crate::Error::NotFound)
        }
    }

    impl Fs for MemFs {
        fn info(&self) -> &FsInfo {
            &self.info
        }
        fn read_dir(&self, path: &[u8]) -> crate::Result<Vec<Entry>> {
            Ok(self
                .nodes
                .iter()
                .filter(|(p, _)| {
                    let parent = &p[..p.iter().rposition(|&b| b == b'/').unwrap_or(0).max(1)];
                    p.as_slice() != b"/" && parent == path
                })
                .map(|(_, (e, _))| e.clone())
                .collect())
        }
        fn stat(&self, path: &[u8]) -> crate::Result<Entry> {
            Ok(self.get(path)?.0.clone())
        }
        fn read_link(&self, path: &[u8]) -> crate::Result<Vec<u8>> {
            Ok(self.get(path)?.1.clone())
        }
        fn open(&self, path: &[u8]) -> crate::Result<Box<dyn Read + '_>> {
            Ok(Box::new(io::Cursor::new(self.get(path)?.1.clone())))
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuxread-copy-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(fs: &MemFs, sources: &[&[u8]], dest: &Path, conflict: Conflict) -> Report {
        let sources: Vec<Vec<u8>> = sources.iter().map(|s| s.to_vec()).collect();
        copy_out(
            fs,
            &sources,
            dest,
            conflict,
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }

    #[test]
    fn copies_a_tree_with_contents_and_times() {
        let mut fs = MemFs::new();
        fs.add(b"/docs", Kind::Dir, b"");
        fs.add(b"/docs/a.txt", Kind::File, b"alpha");
        fs.add(b"/docs/sub", Kind::Dir, b"");
        fs.add(b"/docs/sub/b.bin", Kind::File, &[7u8; 3 * CHUNK + 5]);
        let dest = temp_dir("tree");
        let report = run(&fs, &[b"/docs"], &dest, Conflict::KeepBoth);
        assert!(
            report.items.iter().all(|i| i.outcome == Outcome::Copied),
            "{:?}",
            report.items
        );
        assert_eq!(std::fs::read(dest.join("docs/a.txt")).unwrap(), b"alpha");
        assert_eq!(
            std::fs::read(dest.join("docs/sub/b.bin")).unwrap().len(),
            3 * CHUNK + 5
        );
        let mtime = std::fs::metadata(dest.join("docs/a.txt"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            mtime.duration_since(UNIX_EPOCH).unwrap().as_secs(),
            981_173_106
        );
    }

    #[test]
    fn copying_the_root_copies_its_contents() {
        let mut fs = MemFs::new();
        fs.add(b"/a.txt", Kind::File, b"alpha");
        fs.add(b"/sub", Kind::Dir, b"");
        let dest = temp_dir("root");
        let report = run(&fs, &[b"/"], &dest, Conflict::KeepBoth);
        assert!(
            report.items.iter().all(|i| i.outcome == Outcome::Copied),
            "{:?}",
            report.items
        );
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"alpha");
        assert!(dest.join("sub").is_dir());
    }

    #[test]
    fn sanitized_and_case_colliding_names_are_renamed() {
        let mut fs = MemFs::new();
        fs.add(b"/w", Kind::Dir, b"");
        fs.add(b"/w/File", Kind::File, b"1");
        fs.add(b"/w/a:b", Kind::File, b"2");
        fs.add(b"/w/file", Kind::File, b"3");
        let dest = temp_dir("names");
        let report = run(&fs, &[b"/w"], &dest, Conflict::KeepBoth);
        let outcome = |src: &str| {
            report
                .items
                .iter()
                .find(|i| i.source == src)
                .unwrap()
                .outcome
                .clone()
        };
        assert_eq!(outcome("/w/File"), Outcome::Copied);
        assert_eq!(outcome("/w/a:b"), Outcome::Renamed { to: "a_b".into() });
        assert_eq!(
            outcome("/w/file"),
            Outcome::Renamed {
                to: "file (2)".into()
            }
        );
        assert_eq!(std::fs::read(dest.join("w/file (2)")).unwrap(), b"3");
    }

    #[test]
    fn conflict_policies_with_an_existing_file() {
        let mut fs = MemFs::new();
        fs.add(b"/note.txt", Kind::File, b"new");
        for (policy, expect_file, expect_content) in [
            (Conflict::KeepBoth, "note (2).txt", "new"),
            (Conflict::Skip, "note.txt", "old"),
            (Conflict::Overwrite, "note.txt", "new"),
        ] {
            let dest = temp_dir(&format!("{policy:?}"));
            std::fs::write(dest.join("note.txt"), "old").unwrap();
            run(&fs, &[b"/note.txt"], &dest, policy);
            assert_eq!(
                std::fs::read_to_string(dest.join(expect_file)).unwrap(),
                expect_content,
                "{policy:?}"
            );
        }
    }

    #[test]
    fn links_and_special_files_are_reported_not_created() {
        let mut fs = MemFs::new();
        fs.add(b"/ln", Kind::Symlink, b"../target");
        fs.add(b"/fifo", Kind::Other, b"");
        let dest = temp_dir("special");
        let report = run(&fs, &[b"/ln", b"/fifo"], &dest, Conflict::KeepBoth);
        assert_eq!(
            report.items[0].outcome,
            Outcome::Skipped {
                reason: "symbolic link -> ../target".into()
            }
        );
        assert_eq!(
            report.items[1].outcome,
            Outcome::Skipped {
                reason: "device, FIFO or socket".into()
            }
        );
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn cancel_removes_the_partial_file() {
        let mut fs = MemFs::new();
        fs.add(b"/big.bin", Kind::File, &vec![1u8; 4 * CHUNK]);
        let dest = temp_dir("cancel");
        let cancel = AtomicBool::new(false);
        let report = copy_out(
            &fs,
            &[b"/big.bin".to_vec()],
            &dest,
            Conflict::KeepBoth,
            &cancel,
            &mut |p| {
                if p.bytes > 0 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(report.cancelled);
        assert_eq!(
            report.items[0].outcome,
            Outcome::Skipped {
                reason: "cancelled".into()
            }
        );
        assert!(!dest.join("big.bin").exists());
    }

    #[test]
    fn same_names_from_different_folders_are_kept_apart() {
        let mut fs = MemFs::new();
        fs.add(b"/a", Kind::Dir, b"");
        fs.add(b"/a/x.txt", Kind::File, b"from a");
        fs.add(b"/b", Kind::Dir, b"");
        fs.add(b"/b/x.txt", Kind::File, b"from b");
        let dest = temp_dir("same-names");
        run(&fs, &[b"/a/x.txt", b"/b/x.txt"], &dest, Conflict::KeepBoth);
        assert_eq!(std::fs::read(dest.join("x.txt")).unwrap(), b"from a");
        assert_eq!(std::fs::read(dest.join("x (2).txt")).unwrap(), b"from b");
    }

    #[test]
    fn paths_longer_than_260_characters_are_copied() {
        let mut fs = MemFs::new();
        let mut path = Vec::new();
        for i in 0..30 {
            path.extend_from_slice(format!("/folder-number-{i:02}").as_bytes());
            fs.add(&path, Kind::Dir, b"");
        }
        let file = [path.as_slice(), b"/leaf.txt"].concat();
        fs.add(&file, Kind::File, b"deep");
        let dest = temp_dir("long-paths");
        let report = run(&fs, &[b"/folder-number-00"], &dest, Conflict::KeepBoth);
        assert!(
            report.items.iter().all(|i| i.outcome == Outcome::Copied),
            "{:?}",
            report.items.last()
        );
        let copied = report.items.last().unwrap().dest.clone().unwrap();
        assert!(copied.as_os_str().len() > 260);
        assert_eq!(std::fs::read(copied).unwrap(), b"deep");
    }

    #[test]
    fn an_unwritable_destination_fails_each_item_without_stopping() {
        let mut fs = MemFs::new();
        fs.add(b"/one.txt", Kind::File, b"1");
        fs.add(b"/two.txt", Kind::File, b"2");
        let dir = temp_dir("unwritable");
        let not_a_folder = dir.join("plain-file");
        std::fs::write(&not_a_folder, "x").unwrap();
        let report = run(
            &fs,
            &[b"/one.txt", b"/two.txt"],
            &not_a_folder,
            Conflict::KeepBoth,
        );
        assert_eq!(report.items.len(), 2);
        assert!(
            report
                .items
                .iter()
                .all(|i| matches!(i.outcome, Outcome::Failed { .. })),
            "{:?}",
            report.items
        );
    }

    #[test]
    fn negative_and_far_future_times_convert() {
        assert!(
            system_time(Timestamp {
                secs: -31_536_000,
                nanos: 5
            })
            .is_some()
        );
        assert!(
            system_time(Timestamp {
                secs: 2_147_484_648,
                nanos: 0
            })
            .is_some()
        );
    }
}
