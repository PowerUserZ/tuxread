//! The TuxRead app's backend: the commands the window calls (spec §5.5). Every command is
//! async and does its work on a blocking thread, so the window never freezes.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod diagnostics;
pub mod display;
pub mod error;
pub mod jobs;
pub mod sources;
pub mod worker;

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::{Manager, State, WebviewWindow};
use tuxread_core::copy::Conflict;

use crate::error::{CmdResult, Code, CommandError};
use crate::jobs::{JobEvent, Jobs, ReportItemView};
use crate::sources::{App, SourceView, SourcesView};
use crate::worker::{ListEvent, PAGE, Properties};

struct Shared {
    app: App,
    jobs: Jobs,
}

type Ctx<'a> = State<'a, Arc<Shared>>;

/// Runs `f` on a blocking thread with the shared state.
async fn blocking<R: Send + 'static>(
    ctx: Ctx<'_>,
    f: impl FnOnce(&Shared) -> CmdResult<R> + Send + 'static,
) -> CmdResult<R> {
    let shared = Arc::clone(&ctx);
    tauri::async_runtime::spawn_blocking(move || f(&shared))
        .await
        .map_err(|e| CommandError::new(Code::Other, e.to_string()))?
}

#[tauri::command]
async fn list_sources(ctx: Ctx<'_>) -> CmdResult<SourcesView> {
    blocking(ctx, |s| Ok(s.app.list_sources())).await
}

#[tauri::command]
async fn open_image(ctx: Ctx<'_>, path: String) -> CmdResult<SourceView> {
    blocking(ctx, move |s| s.app.open_image(&PathBuf::from(path))).await
}

#[tauri::command]
async fn open_disk(ctx: Ctx<'_>, window: WebviewWindow, number: u32) -> CmdResult<SourceView> {
    // The UAC prompt belongs to this window, so it opens in front of it.
    let owner = window.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    blocking(ctx, move |s| s.app.open_disk(number, owner)).await
}

#[tauri::command]
async fn list_dir(
    ctx: Ctx<'_>,
    volume: u32,
    dir: u32,
    on_event: Channel<ListEvent>,
) -> CmdResult<usize> {
    blocking(ctx, move |s| {
        let slot = s.app.volume(volume)?;
        let result = slot.worker().and_then(|w| {
            w.call(move |session| {
                session.list(dir, PAGE, &mut |event| {
                    let _ = on_event.send(event);
                })
            })?
        });
        result.map_err(|e| s.app.explain(&slot, e))
    })
    .await
}

#[tauri::command]
async fn stat(ctx: Ctx<'_>, volume: u32, entry: u32) -> CmdResult<Properties> {
    blocking(ctx, move |s| {
        let slot = s.app.volume(volume)?;
        let result = slot
            .worker()
            .and_then(|w| w.call(move |session| session.properties(entry))?);
        result.map_err(|e| s.app.explain(&slot, e))
    })
    .await
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ConflictArg {
    KeepBoth,
    Skip,
    Overwrite,
}

impl From<ConflictArg> for Conflict {
    fn from(c: ConflictArg) -> Self {
        match c {
            ConflictArg::KeepBoth => Conflict::KeepBoth,
            ConflictArg::Skip => Conflict::Skip,
            ConflictArg::Overwrite => Conflict::Overwrite,
        }
    }
}

#[tauri::command]
async fn copy(
    ctx: Ctx<'_>,
    volume: u32,
    entries: Vec<u32>,
    dest: String,
    conflict: ConflictArg,
    on_event: Channel<JobEvent>,
) -> CmdResult<u32> {
    let shared = Arc::clone(&ctx);
    blocking(ctx, move |s| {
        let slot = s.app.volume(volume)?;
        let paths = slot
            .worker()
            .and_then(|w| w.call(move |session| session.paths(&entries))?)
            .map_err(|e| s.app.explain(&slot, e))?;
        let on = Arc::clone(&slot);
        s.jobs.start(
            slot.volume.clone(),
            paths,
            PathBuf::from(dest),
            conflict.into(),
            move |e| shared.app.explain(&on, e),
            move |event| {
                let _ = on_event.send(event);
            },
        )
    })
    .await
}

#[tauri::command]
async fn cancel_job(ctx: Ctx<'_>, job: u32) -> CmdResult<()> {
    blocking(ctx, move |s| s.jobs.cancel(job)).await
}

#[tauri::command]
async fn job_report(ctx: Ctx<'_>, job: u32) -> CmdResult<Vec<ReportItemView>> {
    blocking(ctx, move |s| s.jobs.report(job)).await
}

#[tauri::command]
async fn save_report(ctx: Ctx<'_>, job: u32, path: String) -> CmdResult<()> {
    blocking(ctx, move |s| s.jobs.save_report(job, &PathBuf::from(path))).await
}

#[tauri::command]
async fn diagnostics(ctx: Ctx<'_>) -> CmdResult<String> {
    blocking(ctx, |s| Ok(diagnostics::diagnostics(&s.app))).await
}

pub fn run() {
    let logs = tauri_plugin_log::Builder::new()
        .target(tauri_plugin_log::Target::new(
            tauri_plugin_log::TargetKind::LogDir {
                file_name: Some("tuxread".into()),
            },
        ))
        .max_file_size(1 << 20)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
        .level(log::LevelFilter::Info)
        .build();
    let result = tauri::Builder::default()
        .plugin(logs)
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(Arc::new(Shared {
                app: App::default(),
                jobs: Jobs::default(),
            }));
            log::info!("TuxRead {} started", env!("CARGO_PKG_VERSION"));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sources,
            open_image,
            open_disk,
            list_dir,
            stat,
            copy,
            cancel_job,
            job_report,
            save_report,
            diagnostics
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        log::error!("TuxRead failed: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    /// One version everywhere (spec §9): Cargo.toml gives it to both executables' version
    /// resources, the installer and the diagnostics; package.json gives it to About.
    #[test]
    fn the_version_comes_from_cargo_toml_alone() {
        let package: serde_json::Value =
            serde_json::from_str(include_str!("../../package.json")).unwrap();
        assert_eq!(package["version"], env!("CARGO_PKG_VERSION"));
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert!(
            config.get("version").is_none(),
            "tauri.conf.json must not set its own version; Tauri then uses Cargo.toml's"
        );
    }
}
