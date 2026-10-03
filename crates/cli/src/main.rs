//! `tuxread-cli`: the engine from a terminal, for development and bug reports.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tuxread_core::cache::CachedDev;
use tuxread_core::copy::{Conflict, Outcome, copy_out};
use tuxread_core::dev::{BlockDev, FileDev};
use tuxread_core::display::display_name;
use tuxread_core::fs::{Entry, Kind, join};
use tuxread_core::probe::{self, Node, NodeKind, Volume};

const USAGE: &str = "usage:
  tuxread-cli disks
  tuxread-cli probe <source>
  tuxread-cli ls <source> <volume> [path]
  tuxread-cli cp <source> <volume> <path> <dest-folder>

<source> is an image file, or disk:N for a disk listed by `disks`. Reading a disk
needs administrator rights: from a normal console, Windows asks once (UAC).
<volume> is the #number printed by `probe`.";

fn main() -> ExitCode {
    let all: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if let Some(helper_args) = tuxread_win::helper::parse_args(&all) {
        // Started by `open_disk` below, elevated: serve disks to the parent, then exit.
        return match tuxread_win::helper::run(&helper_args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    let args: Vec<&str> = all.iter().skip(1).map(String::as_str).collect();
    let result = match args.as_slice() {
        ["disks"] => disks_cmd(),
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

#[derive(Debug, PartialEq, Eq)]
enum Source<'a> {
    Image(&'a Path),
    Disk(u32),
}

fn parse_source(src: &str) -> Result<Source<'_>, String> {
    match src.strip_prefix("disk:") {
        Some(n) => n
            .parse()
            .map(Source::Disk)
            .map_err(|_| format!("bad disk number: {src}")),
        None => Ok(Source::Image(Path::new(src))),
    }
}

fn open_source(src: &str) -> Result<Vec<Node>, String> {
    let dev: Arc<dyn BlockDev> = match parse_source(src)? {
        Source::Image(path) => Arc::new(FileDev::open(path).map_err(|e| format!("{src}: {e}"))?),
        Source::Disk(n) => open_disk(n).map_err(|e| format!("{src}: {e}"))?,
    };
    Ok(probe::probe(Arc::new(CachedDev::new(dev))))
}

/// Elevated: reads the disk directly. Otherwise starts this program again as the disk
/// helper, through one UAC prompt, and reads through it.
#[cfg(windows)]
fn open_disk(n: u32) -> Result<Arc<dyn BlockDev>, String> {
    use tuxread_win::disk::{WinDisk, list_disks};
    use tuxread_win::helper;
    // Listing disks needs no admin rights, so a wrong number never costs a UAC prompt.
    if !list_disks().iter().any(|d| d.number == n) {
        return Err(format!("no disk {n} (`tuxread-cli disks` lists them)"));
    }
    if tuxread_win::is_elevated() {
        return Ok(Arc::new(WinDisk::open(n).map_err(|e| e.to_string())?));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = helper::launch(&exe, &[], true, 0).map_err(|e| e.to_string())?;
    Ok(Arc::new(helper.open_disk(n).map_err(|e| e.to_string())?))
}

#[cfg(not(windows))]
fn open_disk(_: u32) -> Result<Arc<dyn BlockDev>, String> {
    Err("disk sources are only available on Windows".into())
}

#[cfg(windows)]
fn disks_cmd() -> Result<(), String> {
    for d in tuxread_win::disk::list_disks() {
        println!(
            "disk:{}  {}  [{}]  {}, {}-byte sectors",
            d.number,
            display_name(d.model.as_bytes()),
            human_size(d.size),
            d.bus,
            d.logical_sector
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn disks_cmd() -> Result<(), String> {
    Err("listing disks is only available on Windows".into())
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
        NodeKind::Detected { status, .. } => {
            Err(format!("volume #{n} cannot be browsed: {}", status))
        }
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
        let label = display_name(node.label.as_bytes());
        match &node.kind {
            NodeKind::Partition { .. } => println!("{indent}{label} [{size}]"),
            NodeKind::Volume(_) => {
                *counter += 1;
                println!("{indent}#{counter} {label} [{size}]");
            }
            NodeKind::Detected { status, .. } => {
                *counter += 1;
                println!(
                    "{indent}#{counter} {label} [{size}] - {}",
                    display_name(status.to_string().as_bytes())
                );
            }
        }
        print_nodes(&node.children, depth + 1, counter);
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
        let target = (e.kind == Kind::Symlink)
            .then(|| fs.read_link(&join(path.as_bytes(), &e.name)).ok())
            .flatten();
        println!("{}", ls_line(&e, target.as_deref()));
    }
    Ok(())
}

/// One line of `ls`, like `ls -l`. Names from the image go through `display_name`, so they
/// cannot send escape sequences to the console.
fn ls_line(e: &Entry, link: Option<&[u8]>) -> String {
    let kind = match e.kind {
        Kind::Dir => 'd',
        Kind::File => '-',
        Kind::Symlink => 'l',
        Kind::Other => '?',
    };
    let time = e.mtime.map(|t| format_time(t.secs)).unwrap_or_default();
    let mut name = display_name(&e.name);
    if let Some(target) = link {
        name = format!("{name} -> {}", display_name(target));
    }
    format!(
        "{kind} {:04o} {:>5}:{:<5} {time} {:>12} {name}",
        e.mode, e.uid, e.gid, e.size
    )
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
                println!("renamed  {} -> {to}", display_name(item.source.as_bytes()));
            }
            Outcome::Skipped { reason } => {
                skipped += 1;
                let (source, reason) = (item.source.as_bytes(), reason.as_bytes());
                println!(
                    "skipped  {} ({})",
                    display_name(source),
                    display_name(reason)
                );
            }
            Outcome::Failed { reason } => {
                failed += 1;
                let (source, reason) = (item.source.as_bytes(), reason.as_bytes());
                println!(
                    "FAILED   {} ({})",
                    display_name(source),
                    display_name(reason)
                );
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
    fn sources_are_images_or_disk_numbers() {
        assert_eq!(parse_source("disk:3"), Ok(Source::Disk(3)));
        assert_eq!(
            parse_source("disk.img"),
            Ok(Source::Image(Path::new("disk.img")))
        );
        assert_eq!(
            parse_source(r"C:\images\disk.raw"),
            Ok(Source::Image(Path::new(r"C:\images\disk.raw")))
        );
        assert!(parse_source("disk:").is_err());
        assert!(parse_source("disk:-1").is_err());
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(64 * 1024 * 1024), "64.0 MiB");
    }

    #[test]
    fn names_from_an_image_cannot_drive_the_terminal() {
        // An escape sequence in a name could retitle or clear the console, or worse.
        let entry = tuxread_core::fs::Entry {
            name: b"evil\x1b]0;owned\x07.txt".to_vec(),
            kind: Kind::Symlink,
            size: 9,
            mtime: None,
            mode: 0o777,
            uid: 0,
            gid: 0,
            ino: 12,
        };
        let line = ls_line(&entry, Some(b"\x1b[2J"));
        assert!(!line.contains('\x1b'), "{line:?}");
        assert!(
            line.ends_with("evil⟨U+001B⟩]0;owned⟨U+0007⟩.txt -> ⟨U+001B⟩[2J"),
            "{line}"
        );
    }
}
