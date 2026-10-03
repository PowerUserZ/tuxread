//! `tuxread-cli`: the engine from a terminal, for development and bug reports.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tuxread_core::cache::CachedDev;
use tuxread_core::copy::{Conflict, Outcome, copy_out};
use tuxread_core::dev::FileDev;
use tuxread_core::fs::{Kind, join};
use tuxread_core::probe::{self, Node, NodeKind, Status, Volume};

const USAGE: &str = "usage:
  tuxread-cli probe <image>
  tuxread-cli ls <image> <volume> [path]
  tuxread-cli cp <image> <volume> <path> <dest-folder>

<volume> is the #number printed by `probe`.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["probe", src] => probe_cmd(src),
        ["ls", src, vol] => ls_cmd(src, vol, "/"),
        ["ls", src, vol, path] => ls_cmd(src, vol, path),
        ["cp", src, vol, path, dest] => cp_cmd(src, vol, path, dest),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open_source(src: &str) -> Result<Vec<Node>, String> {
    let file = FileDev::open(Path::new(src)).map_err(|e| format!("{src}: {e}"))?;
    Ok(probe::probe(Arc::new(CachedDev::new(Arc::new(file)))))
}

fn volume(nodes: &[Node], number: &str) -> Result<Volume, String> {
    let n: usize = number
        .trim_start_matches('#')
        .parse()
        .map_err(|_| format!("bad volume number: {number}"))?;
    let leaves = probe::leaves(nodes);
    let node = n
        .checked_sub(1)
        .and_then(|i| leaves.get(i))
        .ok_or(format!("no volume #{n}"))?;
    match &node.kind {
        NodeKind::Volume(v) => Ok(v.clone()),
        NodeKind::Detected { status, .. } => Err(format!(
            "volume #{n} cannot be browsed: {}",
            status_text(status)
        )),
        NodeKind::Partition { .. } => Err(format!("#{n} is a partition")),
    }
}

fn probe_cmd(src: &str) -> Result<(), String> {
    let nodes = open_source(src)?;
    println!("{src}");
    print_nodes(&nodes, 1, &mut 0);
    Ok(())
}

fn print_nodes(nodes: &[Node], depth: usize, counter: &mut usize) {
    for node in nodes {
        let indent = "  ".repeat(depth);
        let size = human_size(node.size);
        match &node.kind {
            NodeKind::Partition { .. } => println!("{indent}{} [{size}]", node.label),
            NodeKind::Volume(_) => {
                *counter += 1;
                println!("{indent}#{counter} {} [{size}]", node.label);
            }
            NodeKind::Detected { status, .. } => {
                *counter += 1;
                println!(
                    "{indent}#{counter} {} [{size}] - {}",
                    node.label,
                    status_text(status)
                );
            }
        }
        print_nodes(&node.children, depth + 1, counter);
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

fn ls_cmd(src: &str, vol: &str, path: &str) -> Result<(), String> {
    let fs = volume(&open_source(src)?, vol)?
        .open()
        .map_err(|e| e.to_string())?;
    let mut entries = fs
        .read_dir(path.as_bytes())
        .map_err(|e| format!("{path}: {e}"))?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    for e in entries {
        let kind = match e.kind {
            Kind::Dir => 'd',
            Kind::File => '-',
            Kind::Symlink => 'l',
            Kind::Other => '?',
        };
        let time = e.mtime.map(|t| format_time(t.secs)).unwrap_or_default();
        let mut name = String::from_utf8_lossy(&e.name).into_owned();
        if e.kind == Kind::Symlink
            && let Ok(target) = fs.read_link(&join(path.as_bytes(), &e.name))
        {
            name = format!("{name} -> {}", String::from_utf8_lossy(&target));
        }
        println!(
            "{kind} {:04o} {:>5}:{:<5} {time} {:>12} {name}",
            e.mode, e.uid, e.gid, e.size
        );
    }
    Ok(())
}

fn cp_cmd(src: &str, vol: &str, path: &str, dest: &str) -> Result<(), String> {
    let fs = volume(&open_source(src)?, vol)?
        .open()
        .map_err(|e| e.to_string())?;
    let report = copy_out(
        fs.as_ref(),
        &[path.as_bytes().to_vec()],
        Path::new(dest),
        Conflict::KeepBoth,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    let (mut copied, mut renamed, mut skipped, mut failed) = (0, 0, 0, 0);
    for item in &report.items {
        match &item.outcome {
            Outcome::Copied => copied += 1,
            Outcome::Renamed { to } => {
                renamed += 1;
                println!("renamed  {} -> {to}", item.source);
            }
            Outcome::Skipped { reason } => {
                skipped += 1;
                println!("skipped  {} ({reason})", item.source);
            }
            Outcome::Failed { reason } => {
                failed += 1;
                println!("FAILED   {} ({reason})", item.source);
            }
        }
    }
    println!("{copied} copied, {renamed} renamed, {skipped} skipped, {failed} failed");
    if failed > 0 {
        Err(format!("{failed} item(s) failed"))
    } else {
        Ok(())
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
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

/// UTC "YYYY-MM-DD HH:MM" for Unix seconds, including dates before 1970.
fn format_time(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times_before_and_after_the_epoch() {
        assert_eq!(format_time(0), "1970-01-01 00:00");
        assert_eq!(format_time(981_173_106), "2001-02-03 04:05");
        assert_eq!(format_time(-60), "1969-12-31 23:59");
        assert_eq!(format_time(2_147_483_648), "2038-01-19 03:14");
        assert_eq!(format_time(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(64 * 1024 * 1024), "64.0 MiB");
    }
}
