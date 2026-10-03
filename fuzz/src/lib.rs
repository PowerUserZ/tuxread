//! Shared by the fuzz targets: touch everything a user could touch in a filesystem.

use std::io::Read;

use tuxread_core::fs::{join, Fs, Kind};

/// Lists, stats, follows and reads entries. The entry budget keeps corrupt, cyclic
/// directory trees from turning into timeouts.
pub fn walk(fs: &dyn Fs) {
    let mut budget = 2_000usize;
    let mut dirs = vec![b"/".to_vec()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs.read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let path = join(&dir, &entry.name);
            let _ = fs.stat(&path);
            match entry.kind {
                Kind::Dir => dirs.push(path),
                Kind::Symlink => {
                    let _ = fs.read_link(&path);
                }
                Kind::File => {
                    if let Ok(mut reader) = fs.open(&path) {
                        let mut buf = [0u8; 4096];
                        for _ in 0..16 {
                            if !matches!(reader.read(&mut buf), Ok(n) if n > 0) {
                                break;
                            }
                        }
                    }
                }
                Kind::Other => {}
            }
        }
    }
}
