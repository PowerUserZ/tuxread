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

pub mod display;
pub mod error;
pub mod worker;

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
        .setup(|_| {
            log::info!("TuxRead {} started", env!("CARGO_PKG_VERSION"));
            Ok(())
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        log::error!("TuxRead failed: {e}");
        std::process::exit(1);
    }
}
