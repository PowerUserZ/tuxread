//! GPT and MBR partition tables.

use std::io::{self, Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::dev::BlockDev;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    Gpt,
    Mbr,
}

/// A partition's type as its table stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCode {
    Gpt([u8; 16]),
    Mbr(u8),
}

/// The disk's own id in its partition table; Windows reports the same (`Get-Disk`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskId {
    /// The GPT disk GUID.
    Gpt([u8; 16]),
    /// The MBR disk signature.
    Mbr(u32),
}

/// The GPT partition type Windows mounts filesystems from: Basic data.
pub const BASIC_DATA: [u8; 16] = [
    0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7,
];

/// True for the type Linux tools give a new partition (GPT "Linux filesystem", MBR 0x83), which
/// Windows does not mount. Only this one: other types that Windows skips hide a filesystem on
/// purpose (dynamic disks, Storage Spaces, OEM recovery) or belong to another owner, and
/// changing them would let Windows mount, and perhaps "repair", what that owner manages.
pub fn hides_from_windows(code: TypeCode) -> bool {
    match code {
        TypeCode::Gpt(guid) => guid_string(&guid) == "0FC63DAF-8483-4772-8E79-3D69D8477DE4",
        TypeCode::Mbr(sys) => sys == 0x83,
    }
}

/// The type that makes Windows mount `fs` (as `Ident::Windows` names it) in a `kind` table.
pub fn windows_type(kind: TableKind, fs: &str) -> TypeCode {
    match kind {
        TableKind::Gpt => TypeCode::Gpt(BASIC_DATA),
        TableKind::Mbr if fs == "FAT" => TypeCode::Mbr(0x0C),
        TableKind::Mbr => TypeCode::Mbr(0x07),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// Number as Linux shows it; MBR logical partitions start at 5.
    pub number: u32,
    /// Byte offset on the device.
    pub start: u64,
    /// Length in bytes.
    pub len: u64,
    pub type_name: String,
    pub type_code: TypeCode,
    /// GPT partition name; empty for MBR.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub kind: TableKind,
    pub disk_id: DiskId,
    pub sector_size: u64,
    pub partitions: Vec<Partition>,
}

/// `Read + Seek` over a [`BlockDev`], for parsers that want a stream.
pub struct DevCursor<'a> {
    dev: &'a dyn BlockDev,
    pos: u64,
}

impl<'a> DevCursor<'a> {
    pub fn new(dev: &'a dyn BlockDev) -> Self {
        Self { dev, pos: 0 }
    }
}

impl Read for DevCursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.dev.len().saturating_sub(self.pos);
        let n = (buf.len() as u64).min(remaining) as usize;
        let Some(dst) = buf.get_mut(..n) else {
            return Ok(0);
        };
        self.dev.read_exact_at(self.pos, dst)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for DevCursor<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.dev.len().checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos =
            new.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"))?;
        Ok(self.pos)
    }
}

/// Runs a third-party parser; a panic on malformed input becomes `None`.
fn guarded<T>(parse: impl FnOnce() -> Option<T>) -> Option<T> {
    catch_unwind(AssertUnwindSafe(parse)).ok().flatten()
}

/// A valid GPT (512- or 4096-byte sectors, CRC-checked, backup header as fallback), if any.
pub fn read_gpt(dev: &dyn BlockDev) -> Option<Table> {
    if !gpt_headers_fit(dev) {
        return None;
    }
    let gpt = guarded(|| gptman::GPT::find_from(&mut DevCursor::new(dev)).ok())?;
    let ss = gpt.sector_size;
    let partitions = gpt
        .iter()
        .filter(|(_, e)| e.is_used())
        .filter_map(|(number, e)| {
            let sectors = e.ending_lba.checked_sub(e.starting_lba)?.checked_add(1)?;
            Some(Partition {
                number,
                start: e.starting_lba.checked_mul(ss)?,
                len: sectors.checked_mul(ss)?,
                type_name: gpt_type_name(&e.partition_type_guid),
                type_code: TypeCode::Gpt(e.partition_type_guid),
                name: e.partition_name.as_str().to_string(),
            })
        })
        .collect();
    Some(Table {
        kind: TableKind::Gpt,
        disk_id: DiskId::Gpt(gpt.header.disk_guid),
        sector_size: ss,
        partitions,
    })
}

/// gptman reserves memory for a header's entry count before reading the entries, and a
/// failed allocation aborts the process: it is not a panic `guarded` can catch. So every
/// valid header gptman may read (primary and backup, 512- and 4096-byte sectors) must, like
/// in the kernel's efi.c, use 128-byte entries and a table of at most 4 MiB.
fn gpt_headers_fit(dev: &dyn BlockDev) -> bool {
    [512u64, 4096].into_iter().all(|ss| {
        let backup = (dev.len() / ss).saturating_sub(1) * ss;
        [ss, backup].into_iter().all(|at| {
            let header = guarded(|| {
                let mut cursor = DevCursor::new(dev);
                cursor.seek(SeekFrom::Start(at)).ok()?;
                gptman::GPTHeader::read_from(&mut cursor).ok()
            });
            header.is_none_or(|h| {
                h.size_of_partition_entry == 128
                    && u64::from(h.number_of_partition_entries) * 128 <= 4 << 20
            })
        })
    })
}

/// An MBR partition table (logical partitions included, extended containers left out).
/// `Ok(None)`: no plausible MBR. `Err`: a protective MBR whose GPT is missing or damaged.
pub fn read_mbr(dev: &dyn BlockDev, sector_size: u32) -> Result<Option<Table>> {
    // Filesystem boot sectors also end in 55 AA; their "entries" are code, so start 0 gives them
    // away. Checked before mbrman: 0.6.1 panics when every used entry starts at sector 0.
    let mut sector0 = [0u8; 512];
    if dev.read_exact_at(0, &mut sector0).is_err()
        || sector0[446..510]
            .as_chunks::<16>()
            .0
            .iter()
            .any(|e| e[4] != 0 && e[8..12] == [0; 4])
    {
        return Ok(None);
    }
    let Some(mbr) = guarded(|| mbrman::MBR::read_from(&mut DevCursor::new(dev), sector_size).ok())
    else {
        return Ok(None);
    };
    if mbr.iter().any(|(_, p)| p.sys == 0xEE) {
        return Err(Error::Corrupt(
            "protective MBR found, but the GPT is missing or damaged".into(),
        ));
    }
    let used: Vec<_> = mbr
        .iter()
        .filter(|(_, p)| p.is_used() && !p.is_extended())
        .collect();
    if used.is_empty() || used.iter().any(|(_, p)| p.sectors == 0) {
        return Ok(None);
    }
    let ss = u64::from(sector_size);
    let partitions = used
        .into_iter()
        .map(|(number, p)| Partition {
            number: number as u32,
            start: u64::from(p.starting_lba) * ss,
            len: u64::from(p.sectors) * ss,
            type_name: mbr_type_name(p.sys),
            type_code: TypeCode::Mbr(p.sys),
            name: String::new(),
        })
        .collect();
    Ok(Some(Table {
        kind: TableKind::Mbr,
        disk_id: DiskId::Mbr(u32::from_le_bytes(mbr.header.disk_signature)),
        sector_size: ss,
        partitions,
    }))
}

/// GUID in the usual text form; the first three fields are stored little-endian.
pub fn guid_string(g: &[u8; 16]) -> String {
    let [a0, a1, a2, a3, b0, b1, c0, c1, d @ ..] = *g;
    let [d0, d1, d2, d3, d4, d5, d6, d7] = d;
    format!(
        "{a3:02X}{a2:02X}{a1:02X}{a0:02X}-{b1:02X}{b0:02X}-{c1:02X}{c0:02X}-{d0:02X}{d1:02X}-{d2:02X}{d3:02X}{d4:02X}{d5:02X}{d6:02X}{d7:02X}"
    )
}

fn gpt_type_name(g: &[u8; 16]) -> String {
    let guid = guid_string(g);
    let name = match guid.as_str() {
        "C12A7328-F81F-11D2-BA4B-00A0C93EC93B" => "EFI system",
        "21686148-6449-6E6F-744E-656564454649" => "BIOS boot",
        "E3C9E316-0B5C-4DB8-817D-F92DF00215AE" => "Microsoft reserved",
        "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7" => "Basic data",
        "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC" => "Windows recovery",
        "0FC63DAF-8483-4772-8E79-3D69D8477DE4" => "Linux filesystem",
        "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709" => "Linux root (x86-64)",
        "B921B045-1DF0-41C3-AF44-4C6F280D3FAE" => "Linux root (ARM64)",
        "933AC7E1-2EB4-4F13-B844-0E14E2AEF915" => "Linux home",
        "BC13C2FF-59E6-4262-A352-B275FD6F7172" => "Linux extended boot",
        "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F" => "Linux swap",
        "E6D6D379-F507-44C2-A23C-238F2A3DF928" => "Linux LVM",
        "CA7D7CCB-63ED-4C53-861C-1742536059CC" => "Linux LUKS",
        "A19D880F-05FC-4D3B-A006-743F0F84911E" => "Linux RAID",
        _ => return guid,
    };
    name.to_string()
}

fn mbr_type_name(sys: u8) -> String {
    let name = match sys {
        0x83 => "Linux",
        0x82 => "Linux swap",
        0x8E => "Linux LVM",
        0xFD => "Linux RAID",
        0x07 => "NTFS/exFAT",
        0x0B | 0x0C => "FAT32",
        0x04 | 0x06 | 0x0E => "FAT16",
        0x27 => "Windows recovery",
        0xEF => "EFI system",
        _ => return format!("type 0x{sys:02X}"),
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::MemDev;

    #[test]
    fn basic_data_is_the_windows_type() {
        assert_eq!(
            guid_string(&BASIC_DATA),
            "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"
        );
    }

    fn put(buf: &mut [u8], off: usize, bytes: &[u8]) {
        buf[off..off + bytes.len()].copy_from_slice(bytes);
    }

    /// Classic MBR: entry `i` (0-based) of the primary table.
    fn mbr_entry(buf: &mut [u8], i: usize, sys: u8, start: u32, sectors: u32) {
        let off = 446 + 16 * i;
        put(buf, off + 4, &[sys]);
        put(buf, off + 8, &start.to_le_bytes());
        put(buf, off + 12, &sectors.to_le_bytes());
        put(buf, 510, &[0x55, 0xAA]);
    }

    #[test]
    fn mbr_with_one_extended_and_two_logical_partitions() {
        let mut disk = vec![0u8; 64 * 512];
        mbr_entry(&mut disk, 0, 0x83, 2, 10); // sectors 2..12
        mbr_entry(&mut disk, 1, 0x05, 20, 40); // extended 20..60
        // EBR at sector 20: logical at +2 (8 sectors), link to next EBR at relative 20.
        let ebr1 = 20 * 512;
        mbr_entry(&mut disk[ebr1..ebr1 + 512], 0, 0x83, 2, 8);
        mbr_entry(&mut disk[ebr1..ebr1 + 512], 1, 0x05, 20, 10);
        // EBR at sector 40: logical at +2 (6 sectors), end of chain.
        let ebr2 = 40 * 512;
        mbr_entry(&mut disk[ebr2..ebr2 + 512], 0, 0x83, 2, 6);

        let table = read_mbr(&MemDev(disk), 512).unwrap().unwrap();
        let got: Vec<_> = table
            .partitions
            .iter()
            .map(|p| (p.number, p.start / 512, p.len / 512))
            .collect();
        assert_eq!(got, vec![(1, 2, 10), (5, 22, 8), (6, 42, 6)]);
        assert_eq!(table.partitions[0].type_name, "Linux");
    }

    #[test]
    fn protective_mbr_without_gpt_is_an_error() {
        let mut disk = vec![0u8; 64 * 512];
        mbr_entry(&mut disk, 0, 0xEE, 1, 63);
        assert!(read_gpt(&MemDev(disk.clone())).is_none());
        assert!(matches!(
            read_mbr(&MemDev(disk), 512),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn boot_sector_garbage_is_not_an_mbr() {
        let mut disk = vec![0u8; 64 * 512];
        put(&mut disk, 510, &[0x55, 0xAA]); // signature but no entries
        assert_eq!(read_mbr(&MemDev(disk.clone()), 512).unwrap(), None);
        mbr_entry(&mut disk, 0, 0x83, 0, 10); // starts at sector 0
        assert_eq!(read_mbr(&MemDev(disk), 512).unwrap(), None);
    }

    /// Runs `f` and reports whether anything inside it panicked, even a panic caught by `guarded`
    /// (libFuzzer aborts on those, so they must not happen).
    fn panicked_inside<T>(f: impl FnOnce() -> T) -> (T, bool) {
        use std::cell::Cell;
        thread_local!(static PANICKED: Cell<bool> = const { Cell::new(false) });
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                PANICKED.set(true);
                previous(info);
            }));
        });
        PANICKED.set(false);
        let out = f();
        (out, PANICKED.get())
    }

    #[test]
    fn entries_at_sector_zero_never_reach_mbrman() {
        // Fuzz crash 85b9adeb: mbrman 0.6.1 panics (find_alignment) when every used entry starts at 0.
        let mut disk = vec![0u8; 727];
        mbr_entry(&mut disk, 2, 0x04, 0, 0);
        let (result, panicked) = panicked_inside(|| read_mbr(&MemDev(disk), 512));
        assert_eq!(result.unwrap(), None);
        assert!(!panicked, "mbrman panicked inside read_mbr");
    }

    #[test]
    fn guid_text_form() {
        // EFI system partition type as stored on disk.
        let raw = [
            0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E,
            0xC9, 0x3B,
        ];
        assert_eq!(guid_string(&raw), "C12A7328-F81F-11D2-BA4B-00A0C93EC93B");
        assert_eq!(gpt_type_name(&raw), "EFI system");
    }
}
