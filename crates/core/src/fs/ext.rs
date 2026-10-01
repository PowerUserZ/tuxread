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
        check_geometry(dev.as_ref())?;
        let fs = Ext4::load(Box::new(DevReader(dev))).map_err(map_err)?;
        Ok(Self { fs, info })
    }
}

/// Most block groups we load. ponytail: ext4-view keeps every group descriptor in memory
/// (16 bytes each), so this caps that at 64 MiB, which is 512 TiB with 4 KiB blocks. Larger
/// volumes need ext4-view to load descriptors lazily.
const MAX_GROUPS: u64 = 1 << 22;

/// The kernel's size checks, made before ext4-view sizes anything from the superblock.
/// Anything else wrong with the superblock is left for ext4-view to report.
fn check_geometry(dev: &dyn BlockDev) -> Result<()> {
    let mut sb = [0u8; 1024];
    dev.read_exact_at(1024, &mut sb)?;
    let field = |offset: usize| {
        sb.get(offset..offset + 4)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u32::from_le_bytes)
    };
    let log_block_size = field(0x18);
    let per_group = u64::from(field(0x20));
    if log_block_size > 6 || per_group == 0 {
        return Ok(());
    }
    let block_size = 1024u64 << log_block_size;
    // Like `ext4_blocks_count`: the high half only counts with the 64bit feature.
    let hi = if field(0x60) & 0x80 != 0 {
        u64::from(field(0x150))
    } else {
        0
    };
    let blocks = hi << 32 | u64::from(field(0x04));
    let device_blocks = dev.len() / block_size;
    if blocks > device_blocks {
        return Err(Error::Corrupt(format!(
            "block count {blocks} exceeds the size of the device ({device_blocks} blocks)"
        )));
    }
    let groups = blocks
        .saturating_sub(u64::from(field(0x14)))
        .div_ceil(per_group);
    if groups > MAX_GROUPS {
        return Err(Error::Unsupported(format!(
            "{groups} block groups; TuxRead reads up to {MAX_GROUPS}"
        )));
    }
    Ok(())
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
            // A damaged inode must not hide the rest of the folder: list its name, and let
            // stat and open report the damage for that one entry.
            out.push(match item.metadata() {
                Ok(meta) => entry(name.to_vec(), &meta),
                Err(_) => Entry {
                    name: name.to_vec(),
                    kind: item.file_type().map_or(Kind::Other, kind),
                    size: 0,
                    mtime: None,
                    mode: 0,
                    uid: 0,
                    gid: 0,
                    ino: 0,
                },
            });
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
