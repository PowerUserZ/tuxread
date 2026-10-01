//! Reads a small real ext4 image kept in the repository, so the whole read path is
//! tested on every platform, without the corpus that needs Linux to build.
//! The image holds a.txt, dir/b.txt and link -> a.txt.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use tuxread_core::Error;
use tuxread_core::dev::{BlockDev, FileDev};
use tuxread_core::fs::ext::ExtFs;
use tuxread_core::fs::{Fs, FsInfo, Kind};

fn open_tiny() -> Box<dyn Fs> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiny-ext4.img");
    let dev: Arc<dyn BlockDev> = Arc::new(FileDev::open(&path).unwrap());
    Box::new(ExtFs::open(dev, FsInfo::default()).unwrap())
}

#[test]
fn lists_reads_and_follows_links() {
    let fs = open_tiny();
    let mut names: Vec<String> = fs
        .read_dir(b"/")
        .unwrap()
        .into_iter()
        .map(|e| String::from_utf8(e.name).unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "dir", "link", "lost+found"]);

    let mut text = String::new();
    fs.open(b"/dir/b.txt")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "beta\n");
    assert_eq!(fs.read_link(b"/link").unwrap(), b"a.txt");
    assert_eq!(fs.stat(b"/link").unwrap().kind, Kind::Symlink);
}

#[test]
fn wrong_kinds_and_missing_paths_are_errors() {
    let fs = open_tiny();
    assert!(matches!(fs.read_dir(b"/a.txt"), Err(Error::NotADirectory)));
    assert!(matches!(fs.stat(b"/missing"), Err(Error::NotFound)));
}
