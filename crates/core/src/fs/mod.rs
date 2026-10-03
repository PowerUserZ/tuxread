//! The filesystem interface shared by every reader.

pub mod ext;

use std::io::Read;

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    /// Device node, FIFO or socket.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub secs: i64,
    pub nanos: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Raw name bytes; Linux names need not be UTF-8.
    pub name: Vec<u8>,
    pub kind: Kind,
    pub size: u64,
    pub mtime: Option<Timestamp>,
    /// Permission bits only (`0o7777`).
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// Inode number, 0 when unknown. Tells hard links and directory loops apart.
    pub ino: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FsInfo {
    /// "ext2", "ext3", "ext4", ...
    pub fs_type: String,
    pub label: String,
    pub uuid: String,
    pub size: u64,
    pub used: u64,
}

/// A mounted, read-only filesystem. Paths are absolute `/`-separated Linux byte paths.
///
/// Deliberately not `Send`: ext4-view's `Ext4` is `Rc`-based, so an instance lives on
/// the thread that opened it.
pub trait Fs {
    fn info(&self) -> &FsInfo;
    /// Entries of a directory, without `.` and `..`.
    fn read_dir(&self, path: &[u8]) -> Result<Vec<Entry>>;
    /// Metadata of `path` itself; a final symlink is not followed.
    fn stat(&self, path: &[u8]) -> Result<Entry>;
    fn read_link(&self, path: &[u8]) -> Result<Vec<u8>>;
    fn open(&self, path: &[u8]) -> Result<Box<dyn Read + '_>>;
}

/// `dir` + `/` + `name`, without doubling the separator at the root.
pub fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut path = dir.to_vec();
    if path.last() != Some(&b'/') {
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}

#[cfg(test)]
mod tests {
    #[test]
    fn join_paths() {
        assert_eq!(super::join(b"/", b"etc"), b"/etc");
        assert_eq!(super::join(b"/etc", b"hosts"), b"/etc/hosts");
    }
}
