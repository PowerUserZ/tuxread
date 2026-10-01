use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::dev::{BlockDev, check_range};

/// Size of one cached chunk. A multiple of every sector size we meet (512, 4096).
pub const CHUNK: u64 = 1 << 20;
/// Chunks kept per source (64 MiB). Defaults; tune with measurements.
pub const CAPACITY: usize = 64;

/// Sits directly on a source. Every read it issues starts at a multiple of
/// [`CHUNK`] and covers a whole chunk (shorter only at the end of the device),
/// so sources that demand sector-aligned I/O are always satisfied, and many
/// small reads from a filesystem become a few large ones.
pub struct CachedDev {
    inner: Arc<dyn BlockDev>,
    lru: Mutex<Lru>,
}

#[derive(Default)]
struct Lru {
    chunks: HashMap<u64, Arc<Vec<u8>>>,
    order: VecDeque<u64>,
}

impl Lru {
    fn get(&mut self, index: u64) -> Option<Arc<Vec<u8>>> {
        let hit = self.chunks.get(&index)?.clone();
        // ponytail: O(n) reorder over at most CAPACITY entries; a linked LRU if CAPACITY grows.
        self.order.retain(|&i| i != index);
        self.order.push_back(index);
        Some(hit)
    }

    fn insert(&mut self, index: u64, data: Arc<Vec<u8>>) {
        if self.chunks.insert(index, data).is_none() {
            self.order.push_back(index);
        }
        while self.order.len() > CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.chunks.remove(&old);
            }
        }
    }
}

impl CachedDev {
    pub fn new(inner: Arc<dyn BlockDev>) -> Self {
        Self {
            inner,
            lru: Mutex::new(Lru::default()),
        }
    }

    fn lru(&self) -> MutexGuard<'_, Lru> {
        self.lru
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn chunk(&self, index: u64) -> io::Result<Arc<Vec<u8>>> {
        if let Some(hit) = self.lru().get(index) {
            return Ok(hit);
        }
        let start = index * CHUNK;
        let len = CHUNK.min(self.inner.len().saturating_sub(start));
        let mut data = vec![0u8; len as usize];
        // The lock is not held during I/O; two threads may fetch the same chunk once each.
        self.inner.read_exact_at(start, &mut data)?;
        let data = Arc::new(data);
        self.lru().insert(index, data.clone());
        Ok(data)
    }
}

impl BlockDev for CachedDev {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        check_range(offset, buf.len(), self.len())?;
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done as u64;
            let chunk = self.chunk(pos / CHUNK)?;
            let (Some(src), Some(dst)) = (chunk.get((pos % CHUNK) as usize..), buf.get_mut(done..))
            else {
                return Err(io::ErrorKind::UnexpectedEof.into());
            };
            let n = src.len().min(dst.len());
            let (Some(src), Some(dst)) = (src.get(..n), dst.get_mut(..n)) else {
                return Err(io::ErrorKind::UnexpectedEof.into());
            };
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            dst.copy_from_slice(src);
            done += n;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::MemDev;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Rejects any read whose offset or length is not a multiple of `sector`,
    /// like `\\.\PhysicalDriveN` does.
    struct AlignedOnlyDev {
        data: MemDev,
        sector: u64,
        reads: AtomicUsize,
    }

    impl BlockDev for AlignedOnlyDev {
        fn len(&self) -> u64 {
            self.data.len()
        }
        fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            if !offset.is_multiple_of(self.sector)
                || !(buf.len() as u64).is_multiple_of(self.sector)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unaligned read",
                ));
            }
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.data.read_exact_at(offset, buf)
        }
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[test]
    fn unaligned_reads_succeed_on_512_and_4096_sector_devices() {
        for sector in [512u64, 4096] {
            let len = 3 * CHUNK as usize + 3 * sector as usize;
            let bytes = pattern(len);
            let dev = CachedDev::new(Arc::new(AlignedOnlyDev {
                data: MemDev(bytes.clone()),
                sector,
                reads: AtomicUsize::new(0),
            }));
            // Odd offsets and lengths, a read spanning a chunk boundary, and the device tail.
            for (offset, n) in [
                (1usize, 3usize),
                (1023, 2050),
                (CHUNK as usize - 5, 10),
                (len - 7, 7),
            ] {
                let mut buf = vec![0u8; n];
                dev.read_exact_at(offset as u64, &mut buf).unwrap();
                assert_eq!(
                    buf,
                    bytes[offset..offset + n],
                    "sector {sector}, offset {offset}"
                );
            }
            let mut buf = [0u8; 8];
            assert!(dev.read_exact_at(len as u64 - 7, &mut buf).is_err());
        }
    }

    #[test]
    fn concurrent_readers_get_the_right_bytes() {
        let bytes = Arc::new(pattern(8 * CHUNK as usize));
        let dev = Arc::new(CachedDev::new(Arc::new(MemDev(bytes.to_vec()))));
        let threads: Vec<_> = (0..8u64)
            .map(|t| {
                let (dev, bytes) = (dev.clone(), bytes.clone());
                std::thread::spawn(move || {
                    for i in 0..200u64 {
                        let offset = ((t * 7919 + i * 104_729) % (7 * CHUNK)) as usize;
                        let mut buf = vec![0u8; 4096 + (i as usize % 3000)];
                        dev.read_exact_at(offset as u64, &mut buf).unwrap();
                        assert_eq!(buf, bytes[offset..offset + buf.len()]);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }

    #[test]
    fn repeated_reads_hit_the_cache() {
        let inner = Arc::new(AlignedOnlyDev {
            data: MemDev(pattern(2 * CHUNK as usize)),
            sector: 512,
            reads: AtomicUsize::new(0),
        });
        let dev = CachedDev::new(inner.clone());
        let mut buf = [0u8; 16];
        for _ in 0..100 {
            dev.read_exact_at(100, &mut buf).unwrap();
        }
        assert_eq!(inner.reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn capacity_is_bounded() {
        let dev = CachedDev::new(Arc::new(MemDev(pattern((CAPACITY + 10) * CHUNK as usize))));
        let mut buf = [0u8; 1];
        for i in 0..(CAPACITY as u64 + 10) {
            dev.read_exact_at(i * CHUNK, &mut buf).unwrap();
        }
        assert_eq!(dev.lru().chunks.len(), CAPACITY);
    }
}
