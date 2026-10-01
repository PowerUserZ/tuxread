//! Reads at any offset from devices that accept only aligned reads (spec §5.6).

use std::io;

use tuxread_core::dev::check_range;

/// Buffer address alignment for raw reads; 4 KiB covers every sector size in use.
pub const ALIGN: usize = 4096;
/// Largest single raw read. It is also the helper's limit (spec §5.7).
pub const MAX_IO: usize = 4 << 20;

/// A zeroed buffer whose usable bytes start on an `ALIGN` boundary.
pub struct AlignedBuf {
    storage: Vec<u8>,
    start: usize,
    len: usize,
}

impl AlignedBuf {
    pub fn new(len: usize) -> Self {
        let storage = vec![0u8; len + ALIGN];
        // `min` only matters if `align_offset` ever gave up; the read would then fail
        // as unaligned instead of going out of bounds.
        let start = storage.as_ptr().align_offset(ALIGN).min(ALIGN);
        Self {
            storage,
            start,
            len,
        }
    }

    /// The first `n` usable bytes (at most the length given to `new`).
    pub fn first_mut(&mut self, n: usize) -> &mut [u8] {
        let n = n.min(self.len);
        self.storage
            .get_mut(self.start..self.start + n)
            .unwrap_or_default()
    }
}

/// Fills `buf` with the bytes at `offset` of a `dev_len`-byte device whose `raw` reads
/// accept only `sector`-aligned offsets and lengths into `ALIGN`-aligned buffers.
/// Each raw read is at most `MAX_IO` bytes and never extends past `dev_len`.
pub fn read_aligned(
    offset: u64,
    buf: &mut [u8],
    sector: u64,
    dev_len: u64,
    raw: &mut dyn FnMut(u64, &mut [u8]) -> io::Result<()>,
) -> io::Result<()> {
    check_range(offset, buf.len(), dev_len)?;
    if !(512..=65536).contains(&sector) || !sector.is_power_of_two() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unusable sector size {sector}"),
        ));
    }
    if buf.is_empty() {
        return Ok(());
    }
    let end = offset + buf.len() as u64;
    let stop = end.div_ceil(sector).saturating_mul(sector).min(dev_len);
    let max = MAX_IO as u64 / sector * sector;
    let mut pos = offset / sector * sector;
    let mut bounce = AlignedBuf::new((stop - pos).min(max) as usize);
    while pos < stop {
        let n = (stop - pos).min(max);
        let chunk = bounce.first_mut(n as usize);
        raw(pos, chunk)?;
        // Copy out the part of [pos, pos + n) that the caller asked for.
        let lo = pos.max(offset);
        let hi = (pos + n).min(end);
        let src = chunk.get((lo - pos) as usize..(hi - pos) as usize);
        let dst = buf.get_mut((lo - offset) as usize..(hi - offset) as usize);
        match (src, dst) {
            (Some(src), Some(dst)) => dst.copy_from_slice(src),
            _ => return Err(io::Error::other("aligned read out of range")),
        }
        pos += n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device that, like a raw Windows disk, refuses reads whose offset, length or
    /// buffer address is not aligned.
    struct Strict {
        data: Vec<u8>,
        sector: u64,
        calls: usize,
    }

    impl Strict {
        fn new(len: usize, sector: u64) -> Self {
            let data = (0..len).map(|i| (i % 251) as u8).collect();
            Self {
                data,
                sector,
                calls: 0,
            }
        }

        fn raw(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            self.calls += 1;
            let aligned = offset.is_multiple_of(self.sector)
                && (buf.len() as u64).is_multiple_of(self.sector)
                && (buf.as_ptr() as usize).is_multiple_of(ALIGN);
            if !aligned || buf.len() > MAX_IO {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            let start = offset as usize;
            buf.copy_from_slice(&self.data[start..start + buf.len()]);
            Ok(())
        }
    }

    #[test]
    fn unaligned_reads_are_served_from_aligned_ones() {
        for sector in [512u64, 4096] {
            let mut dev = Strict::new(64 * 1024, sector);
            let len = dev.data.len() as u64;
            for (offset, n) in [
                (0u64, 1usize),
                (1, 511),
                (510, 4),
                (4095, 2),
                (100, 20_000),
                (65_535, 1),
            ] {
                let mut buf = vec![0u8; n];
                read_aligned(offset, &mut buf, sector, len, &mut |o, b| dev.raw(o, b)).unwrap();
                let start = offset as usize;
                assert_eq!(
                    buf,
                    dev.data[start..start + n],
                    "sector {sector}, offset {offset}"
                );
            }
        }
    }

    #[test]
    fn large_reads_are_split_into_bounded_raw_reads() {
        let mut dev = Strict::new(9 << 20, 4096);
        let mut buf = vec![0u8; (9 << 20) - 3];
        read_aligned(1, &mut buf, 4096, 9 << 20, &mut |o, b| dev.raw(o, b)).unwrap();
        assert_eq!(buf[..], dev.data[1..(9 << 20) - 2]);
        assert_eq!(dev.calls, 3);
    }

    #[test]
    fn reads_past_the_end_are_refused_before_any_raw_read() {
        let mut dev = Strict::new(64 * 1024, 512);
        let mut buf = vec![0u8; 2];
        let err =
            read_aligned(65_535, &mut buf, 512, 65_536, &mut |o, b| dev.raw(o, b)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(dev.calls, 0);
    }

    #[test]
    fn a_zero_sector_size_is_an_error_not_a_crash() {
        let mut buf = [0u8; 4];
        let err = read_aligned(0, &mut buf, 0, 4096, &mut |_, _| Ok(())).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
