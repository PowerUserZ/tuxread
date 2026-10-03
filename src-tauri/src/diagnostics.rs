//! "Copy diagnostics" (spec §6.3): what TuxRead sees, without file names, for a GitHub issue.

use std::fmt::Write;

use tuxread_core::probe::{Node, NodeKind, Status};
use tuxread_win::disk::list_disks;

use crate::sources::{App, lock};

pub fn diagnostics(app: &App) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "TuxRead {} ({} build) diagnostics (no file names, labels or paths)",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH
    );
    let _ = writeln!(
        out,
        "elevated: {}, disk helper running: {}",
        tuxread_win::is_elevated(),
        app.helper_running()
    );
    for d in list_disks() {
        let _ = writeln!(
            out,
            "disk:{}  {}  {}  {}, {}/{}-byte sectors",
            d.number,
            d.model,
            human_size(d.size),
            d.bus,
            d.logical_sector,
            d.physical_sector
        );
    }
    let registry = lock(&app.registry);
    for o in &registry.disks {
        let _ = writeln!(out, "opened disk:{}", o.key);
        nodes(&mut out, &o.nodes, 1);
    }
    for o in &registry.images {
        let _ = writeln!(out, "opened image #{}  {}", o.key, human_size(o.size));
        nodes(&mut out, &o.nodes, 1);
    }
    out
}

fn nodes(out: &mut String, nodes: &[Node], depth: usize) {
    for node in nodes {
        let size = human_size(node.size);
        let line = match &node.kind {
            NodeKind::Partition { number, type_name } => {
                format!("Partition {number} ({type_name})  {size}")
            }
            NodeKind::Volume(v) => format!("{} volume  {size}", v.info.fs_type),
            NodeKind::Detected { name, status } => {
                format!("{name}  {size}  {}", status_text(status))
            }
        };
        let _ = writeln!(out, "{}{line}", "  ".repeat(depth));
        self::nodes(out, &node.children, depth + 1);
    }
}

fn status_text(status: &Status) -> String {
    match status {
        Status::WindowsCanOpen => "Windows can open this".into(),
        Status::Later => "supported in a later version".into(),
        Status::NotSupported(why) => format!("not supported: {why}"),
        Status::Unrecognized => "unrecognized".into(),
        Status::Error(e) => format!("error: {e}"),
    }
}

fn human_size(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    match units.get(unit) {
        Some(u) if unit > 0 => format!("{value:.1} {u}"),
        _ => format!("{bytes} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::tests::tiny_image;

    #[test]
    fn diagnostics_describe_what_is_open_without_names_or_paths() {
        let app = App::default();
        app.open_image(&tiny_image()).unwrap();
        let text = diagnostics(&app);
        assert!(text.contains("opened image #1  512.0 KiB"), "{text}");
        assert!(text.contains("  ext4 volume  512.0 KiB"), "{text}");
        assert!(text.contains("disk:0 "), "{text}");
        assert!(!text.contains("tiny-ext4"), "{text}");
        assert!(!text.contains(r"\"), "{text}");
    }

    /// The architecture is the build's (an x64 build also runs on ARM64 Windows), so it is
    /// named next to the version, not as if it were the Windows edition.
    #[test]
    fn the_architecture_is_named_as_the_build_s() {
        let text = diagnostics(&App::default());
        let first = format!(
            "TuxRead {} ({} build) diagnostics",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH
        );
        assert!(text.starts_with(&first), "{text}");
        assert!(
            text.lines().nth(1).unwrap().starts_with("elevated: "),
            "{text}"
        );
    }
}
