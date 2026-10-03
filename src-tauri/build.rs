/// TuxRead's own commands. Tauri checks an app's commands only when they are declared here
/// (spec §5.8); capabilities/main.json then grants exactly these.
const COMMANDS: &[&str] = &[
    "list_sources",
    "open_image",
    "open_disk",
    "list_dir",
    "stat",
    "copy",
    "cancel_job",
    "job_report",
    "save_report",
    "diagnostics",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build script");
}
