//! Physical disks: listing them without admin rights, and reading `\\.\PhysicalDrive<N>`
//! (spec §5.6).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::path::Path;

use tuxread_core::dev::BlockDev;
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
use windows_sys::Win32::System::Ioctl::{
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    StorageAccessAlignmentProperty, StorageDeviceProperty,
};

use crate::align::read_aligned;
use crate::sys;

/// Disk numbers probed by `list_disks`. ponytail: probing `\\.\PhysicalDrive0..=255` finds
/// gaps and late numbers without SetupDi; a machine with more disk numbers needs SetupDi.
const MAX_DISKS: u32 = 256;

/// What Windows tells a normal user about a disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInfo {
    /// N in `\\.\PhysicalDrive<N>`. Numbers can have gaps and change across reboots.
    pub number: u32,
    /// Vendor and product, as the device reports them.
    pub model: String,
    pub size: u64,
    pub bus: &'static str,
    pub logical_sector: u32,
    pub physical_sector: u32,
}

pub fn disk_path(number: u32) -> String {
    format!(r"\\.\PhysicalDrive{number}")
}

/// Every disk Windows knows, in number order. Needs no admin rights.
/// Disks without media (an empty card reader) are left out.
pub fn list_disks() -> Vec<DiskInfo> {
    (0..MAX_DISKS).filter_map(|n| describe(n).ok()).collect()
}

fn describe(number: u32) -> io::Result<DiskInfo> {
    // Access 0 is enough for the IOCTLs below and needs no admin rights.
    let file = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(disk_path(number))?;
    let (size, geometry_sector) = geometry(&file)?;
    let (logical_sector, physical_sector) =
        alignment(&file).unwrap_or((geometry_sector, geometry_sector));
    let descriptor = ioctl_property(&file, StorageDeviceProperty, 1024)?;
    let (model, bus) = parse_device_descriptor(&descriptor);
    Ok(DiskInfo {
        number,
        model,
        size,
        bus,
        logical_sector,
        physical_sector,
    })
}

/// Disk size and the sector size from `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX`.
fn geometry(file: &File) -> io::Result<(u64, u32)> {
    let out = sys::ioctl(file, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], 256)?;
    // DISK_GEOMETRY (24 bytes, BytesPerSector at 20), then DiskSize at 24.
    match (le_u32(&out, 20), le_u64(&out, 24)) {
        (Some(sector), Some(size)) => Ok((size, sector)),
        _ => Err(io::Error::other("short disk geometry")),
    }
}

/// Logical and physical sector sizes from `StorageAccessAlignmentProperty`.
fn alignment(file: &File) -> io::Result<(u32, u32)> {
    let out = ioctl_property(file, StorageAccessAlignmentProperty, 64)?;
    // STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR: BytesPerLogicalSector at 16, physical at 20.
    match (le_u32(&out, 16), le_u32(&out, 20)) {
        (Some(logical), Some(physical)) if logical > 0 => Ok((logical, physical)),
        _ => Err(io::Error::other("short alignment descriptor")),
    }
}

fn ioctl_property(file: &File, property: i32, out_len: usize) -> io::Result<Vec<u8>> {
    // STORAGE_PROPERTY_QUERY: PropertyId, QueryType, AdditionalParameters[1] (padded).
    let mut query = [0u8; 12];
    query[..4].copy_from_slice(&property.to_le_bytes());
    query[4..8].copy_from_slice(&PropertyStandardQuery.to_le_bytes());
    sys::ioctl(file, IOCTL_STORAGE_QUERY_PROPERTY, &query, out_len)
}

/// Model ("vendor product") and bus name from a STORAGE_DEVICE_DESCRIPTOR.
fn parse_device_descriptor(d: &[u8]) -> (String, &'static str) {
    let text_at = |field: usize| -> String {
        let Some(offset) = le_u32(d, field).filter(|&o| o != 0) else {
            return String::new();
        };
        let bytes = d.get(offset as usize..).unwrap_or_default();
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(bytes.get(..end).unwrap_or_default())
            .trim()
            .to_string()
    };
    let model = [text_at(12), text_at(16)]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (model, bus_name(le_u32(d, 28).unwrap_or(0)))
}

/// STORAGE_BUS_TYPE values (ntddstor.h).
fn bus_name(bus: u32) -> &'static str {
    match bus {
        1 => "SCSI",
        2 => "ATAPI",
        3 => "ATA",
        4 => "IEEE 1394",
        6 => "Fibre Channel",
        7 => "USB",
        8 => "RAID",
        9 => "iSCSI",
        10 => "SAS",
        11 => "SATA",
        12 => "SD",
        13 => "MMC",
        14 => "Virtual",
        15 => "Virtual disk file",
        16 => "Storage Spaces",
        17 => "NVMe",
        18 => "SCM",
        19 => "UFS",
        _ => "Other",
    }
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le_u64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// A disk (or any file) read through aligned raw reads, as Windows requires for disks.
/// Opened read-only, sharing read and write so mounted disks stay usable.
pub struct WinDisk {
    file: File,
    len: u64,
    sector: u32,
}

impl WinDisk {
    /// `\\.\PhysicalDrive<number>`; needs admin rights.
    pub fn open(number: u32) -> io::Result<Self> {
        Self::open_path(Path::new(&disk_path(number)))
    }

    /// A device path, or a plain file (treated as a disk with 512-byte sectors).
    pub fn open_path(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(path)?;
        let (len, sector) = match geometry(&file) {
            Ok((len, geometry_sector)) => {
                let sector = alignment(&file).map_or(geometry_sector, |(logical, _)| logical);
                (len, sector)
            }
            Err(_) if file.metadata()?.is_file() => (file.metadata()?.len(), 512),
            Err(e) => return Err(e),
        };
        Ok(Self { file, len, sector })
    }

    pub fn sector_size(&self) -> u32 {
        self.sector
    }
}

impl BlockDev for WinDisk {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        read_aligned(
            offset,
            buf,
            u64::from(self.sector),
            self.len,
            &mut |pos, chunk| read_full_at(&self.file, pos, chunk),
        )
    }
}

/// `seek_read` until `buf` is full; it issues ReadFile with an explicit offset, so
/// threads sharing the handle do not race on a file pointer.
fn read_full_at(file: &File, mut pos: u64, mut buf: &mut [u8]) -> io::Result<()> {
    while !buf.is_empty() {
        match file.seek_read(buf, pos) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buf = std::mem::take(&mut buf).get_mut(n..).unwrap_or_default();
                pos += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_descriptors_give_model_and_bus() {
        let mut d = vec![0u8; 64];
        d[12..16].copy_from_slice(&40u32.to_le_bytes()); // vendor offset
        d[16..20].copy_from_slice(&48u32.to_le_bytes()); // product offset
        d[28..32].copy_from_slice(&7u32.to_le_bytes()); // USB
        d[40..46].copy_from_slice(b"Kings\0");
        d[48..56].copy_from_slice(b"DT 64G \0");
        assert_eq!(
            parse_device_descriptor(&d),
            ("Kings DT 64G".to_string(), "USB")
        );
        // Offsets past the end, or zero, give no text rather than a crash.
        d[12..16].copy_from_slice(&9999u32.to_le_bytes());
        d[16..20].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(parse_device_descriptor(&d), (String::new(), "USB"));
        assert_eq!(parse_device_descriptor(&[]), (String::new(), "Other"));
    }
}
