use std::io;
use std::path::Path;
use std::sync::Arc;

/// Read-only random access to the bytes of a disk, partition, image or container.
/// There is deliberately no write method anywhere in the stack.
pub trait BlockDev: Send + Sync {
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fills `buf` completely or returns an error. Never a short read.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

/// `UnexpectedEof` unless `[offset, offset + len)` lies inside a device of `dev_len` bytes.
pub fn check_range(offset: u64, len: usize, dev_len: u64) -> io::Result<()> {
    match offset.checked_add(len as u64) {
        Some(end) if end <= dev_len => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "read of {len} bytes at {offset} goes past the end of the device ({dev_len} bytes)"
            ),
        )),
    }
}

/// An image file, opened read-only.
pub struct FileDev {
    file: std::fs::File,
    len: u64,
}

impl FileDev {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

impl BlockDev for FileDev {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        check_range(offset, buf.len(), self.len)?;
        read_file_at(&self.file, offset, buf)
    }
}

#[cfg(windows)]
fn read_file_at(file: &std::fs::File, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buf = std::mem::take(&mut buf)
                    .get_mut(n..)
                    .ok_or_else(|| io::Error::other("read returned more bytes than requested"))?;
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_file_at(file: &std::fs::File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

/// A window `[start, start + len)` onto another device, e.g. a partition.
pub struct SliceDev {
    inner: Arc<dyn BlockDev>,
    start: u64,
    len: u64,
}

impl SliceDev {
    pub fn new(inner: Arc<dyn BlockDev>, start: u64, len: u64) -> io::Result<Self> {
        match start.checked_add(len) {
            Some(end) if end <= inner.len() => Ok(Self { inner, start, len }),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("region at {start} of {len} bytes extends past the end of the device"),
            )),
        }
    }
}

impl BlockDev for SliceDev {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        check_range(offset, buf.len(), self.len)?;
        self.inner.read_exact_at(self.start + offset, buf)
    }
}

/// An in-memory device, for tests and fuzzing.
pub struct MemDev(pub Vec<u8>);

impl BlockDev for MemDev {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        check_range(offset, buf.len(), self.len())?;
        let start = offset as usize;
        let src = self
            .0
            .get(start..start + buf.len())
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_dev_reads_exact_range_and_rejects_past_end() {
        let dev = MemDev((0u8..=9).collect());
        let mut buf = [0u8; 3];
        dev.read_exact_at(7, &mut buf).unwrap();
        assert_eq!(buf, [7, 8, 9]);
        let err = dev.read_exact_at(8, &mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn slice_dev_offsets_reads_and_checks_bounds() {
        let base: Arc<dyn BlockDev> = Arc::new(MemDev((0u8..100).collect()));
        let slice = SliceDev::new(base.clone(), 10, 20).unwrap();
        assert_eq!(slice.len(), 20);
        let mut buf = [0u8; 2];
        slice.read_exact_at(18, &mut buf).unwrap();
        assert_eq!(buf, [28, 29]);
        assert!(slice.read_exact_at(19, &mut buf).is_err());
        assert!(SliceDev::new(base, 90, 11).is_err());
    }

    #[test]
    fn file_dev_reads_from_file() {
        let path = std::env::temp_dir().join(format!("tuxread-filedev-{}.bin", std::process::id()));
        std::fs::write(&path, (0u8..=255).collect::<Vec<_>>()).unwrap();
        let dev = FileDev::open(&path).unwrap();
        let mut buf = [0u8; 4];
        dev.read_exact_at(252, &mut buf).unwrap();
        assert_eq!(buf, [252, 253, 254, 255]);
        assert!(dev.read_exact_at(253, &mut buf).is_err());
        drop(dev);
        std::fs::remove_file(path).unwrap();
    }
}
