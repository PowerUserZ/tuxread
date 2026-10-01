//! ext2/3/4 through the `ext4-view` crate.

use std::io::Read;
use std::sync::Arc;

use ext4_view::{Ext4, Ext4Error, Ext4Read, FileType, Metadata, Path};

use super::{Entry, Fs, FsInfo, Kind, Timestamp};
use crate::dev::BlockDev;
use crate::{Error, Result};

struct DevReader(Arc<dyn BlockDev>);

impl Ext4Read for DevReader {
    fn read(
        &mut self,
        start_byte: u64,
        dst: &mut [u8],
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.0
            .read_exact_at(start_byte, dst)
            .map_err(|e| Box::new(e) as _)
    }
}

pub struct ExtFs {
    fs: Ext4,
    info: FsInfo,
}

impl ExtFs {
    /// Loads the filesystem; the journal, if dirty, is replayed in memory only.
    pub fn open(dev: Arc<dyn BlockDev>, info: FsInfo) -> Result<Self> {
        let fs = Ext4::load(Box::new(DevReader(dev))).map_err(map_err)?;
        Ok(Self { fs, info })
    }
}

fn path(p: &[u8]) -> Result<Path<'_>> {
    Path::try_from(p).map_err(|e| Error::Other(format!("invalid path: {e}")))
}

fn kind(t: FileType) -> Kind {
    match t {
        FileType::Regular => Kind::File,
        FileType::Directory => Kind::Dir,
        FileType::Symlink => Kind::Symlink,
        _ => Kind::Other,
    }
}

fn entry(name: Vec<u8>, m: &Metadata) -> Entry {
    let t = m.modified();
    Entry {
        name,
        kind: kind(m.file_type()),
        size: m.len(),
        mtime: Some(Timestamp {
            secs: t.seconds(),
            nanos: t.nanoseconds(),
        }),
        mode: u32::from(m.mode()) & 0o7777,
        uid: m.uid(),
        gid: m.gid(),
        ino: u64::from(m.inode()),
    }
}

fn map_err(e: Ext4Error) -> Error {
    match e {
        Ext4Error::NotFound => Error::NotFound,
        Ext4Error::NotADirectory => Error::NotADirectory,
        Ext4Error::Incompatible(i) => Error::Unsupported(i.to_string()),
        Ext4Error::Encrypted => Error::Unsupported("encrypted file or directory".into()),
        Ext4Error::Corrupt(c) => Error::Corrupt(c.to_string()),
        other => Error::Other(other.to_string()),
    }
}

impl Fs for ExtFs {
    fn info(&self) -> &FsInfo {
        &self.info
    }

    fn read_dir(&self, dir: &[u8]) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        for item in self.fs.read_dir(path(dir)?).map_err(map_err)? {
            let item = item.map_err(map_err)?;
            let name = item.file_name();
            let name: &[u8] = name.as_ref();
            if name == b"." || name == b".." {
                continue;
            }
            let meta = item.metadata().map_err(map_err)?;
            out.push(entry(name.to_vec(), &meta));
        }
        Ok(out)
    }

    fn stat(&self, p: &[u8]) -> Result<Entry> {
        let meta = self.fs.symlink_metadata(path(p)?).map_err(map_err)?;
        let name = p.rsplit(|&b| b == b'/').next().unwrap_or_default();
        Ok(entry(name.to_vec(), &meta))
    }

    fn read_link(&self, p: &[u8]) -> Result<Vec<u8>> {
        let target = self.fs.read_link(path(p)?).map_err(map_err)?;
        Ok(target.as_ref().to_vec())
    }

    fn open(&self, p: &[u8]) -> Result<Box<dyn Read + '_>> {
        Ok(Box::new(self.fs.open(path(p)?).map_err(map_err)?))
    }
}
