//! Recognizes what a region holds by its on-disk signatures.

use std::io;

use crate::dev::BlockDev;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ident {
    Ext(ExtInfo),
    Btrfs,
    Xfs,
    Luks {
        version: u16,
    },
    LvmMember,
    /// Filesystems Windows can open by itself.
    Windows(&'static str),
    /// Recognized, but TuxRead cannot read it.
    Other(&'static str),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtInfo {
    /// "ext2", "ext3" or "ext4".
    pub flavor: &'static str,
    /// Features present that ext4-view refuses, by their e2fsprogs names.
    pub unsupported: Vec<&'static str>,
    pub label: String,
    pub uuid: String,
    pub block_size: u64,
    pub blocks: u64,
    pub free_blocks: u64,
}

/// Bytes read from the start of a region; covers every signature below except the RAID tail ones.
const HEAD: u64 = 256 * 1024;

const RAID_MAGIC: u32 = 0xA92B_4EFC;

/// What a Linux md RAID member is called; `probe` looks for it.
pub const RAID_MEMBER: &str = "Linux RAID member";
const ZFS_UBERBLOCK_MAGIC: u64 = 0x00BA_B10C;

pub fn identify(dev: &dyn BlockDev) -> io::Result<Ident> {
    let mut head = vec![0u8; dev.len().min(HEAD) as usize];
    dev.read_exact_at(0, &mut head)?;
    let at = |off: usize, magic: &[u8]| head.get(off..off + magic.len()) == Some(magic);

    if at(0, b"LUKS\xBA\xBE") {
        return Ok(Ident::Luks {
            version: be16(&head, 6).unwrap_or(0),
        });
    }
    if (0..4).any(|s| at(s * 512, b"LABELONE") && at(s * 512 + 24, b"LVM2 001")) {
        return Ok(Ident::LvmMember);
    }
    // RAID members also contain a filesystem at the start (metadata 0.90 and 1.0), so check them first.
    if le32(&head, 0) == Some(RAID_MAGIC)
        || le32(&head, 4096) == Some(RAID_MAGIC)
        || raid_tail(dev)?
    {
        return Ok(Ident::Other(RAID_MEMBER));
    }
    if at(0, b"XFSB") {
        return Ok(Ident::Xfs);
    }
    if at(0x1_0040, b"_BHRfS_M") {
        return Ok(Ident::Btrfs);
    }
    if le16(&head, 1080) == Some(0xEF53)
        && let Some(info) = head.get(1024..2048).and_then(ext_info)
    {
        return Ok(Ident::Ext(info));
    }
    if le32(&head, 1024) == Some(0xF2F5_2010) {
        return Ok(Ident::Other("f2fs"));
    }
    if [b"ReIsErFs".as_slice(), b"ReIsEr2Fs", b"ReIsEr3Fs"]
        .iter()
        .any(|m| at(0x1_0034, m))
    {
        return Ok(Ident::Other("ReiserFS"));
    }
    if at(0x8000, b"JFS1") {
        return Ok(Ident::Other("JFS"));
    }
    if [4096usize, 8192, 16384, 65536]
        .iter()
        .any(|p| at(p - 10, b"SWAPSPACE2") || at(p - 10, b"SWAP-SPACE"))
    {
        return Ok(Ident::Other("Linux swap"));
    }
    if at(3, b"NTFS    ") {
        return Ok(Ident::Windows("NTFS"));
    }
    if at(3, b"EXFAT   ") {
        return Ok(Ident::Windows("exFAT"));
    }
    if at(3, b"ReFS\0\0\0\0") {
        return Ok(Ident::Windows("ReFS"));
    }
    if at(3, b"-FVE-FS-") {
        return Ok(Ident::Windows("BitLocker"));
    }
    if at(510, &[0x55, 0xAA]) && (at(54, b"FAT1") || at(54, b"FAT     ") || at(82, b"FAT32   ")) {
        return Ok(Ident::Windows("FAT"));
    }
    if at(32, b"NXSB") {
        return Ok(Ident::Other("APFS"));
    }
    if at(1024, b"H+") || at(1024, b"HX") {
        return Ok(Ident::Other("HFS+"));
    }
    let uberblock = |k: usize| {
        let b: [u8; 8] = head
            .get(0x2_0000 + k * 1024..0x2_0008 + k * 1024)?
            .try_into()
            .ok()?;
        Some(
            u64::from_le_bytes(b) == ZFS_UBERBLOCK_MAGIC
                || u64::from_be_bytes(b) == ZFS_UBERBLOCK_MAGIC,
        )
    };
    if (0..128).any(|k| uberblock(k) == Some(true)) {
        return Ok(Ident::Other("ZFS member"));
    }
    Ok(Ident::Unknown)
}

/// RAID metadata 0.90 and 1.0 sit near the end of the member.
fn raid_tail(dev: &dyn BlockDev) -> io::Result<bool> {
    let len = dev.len();
    let v090 = (len / 65536).checked_sub(1).map(|n| n * 65536);
    let v10 = (len / 512).checked_sub(16).map(|s| (s & !7) * 512);
    for off in [v090, v10].into_iter().flatten() {
        if off >= HEAD && off + 4 <= len {
            let mut b = [0u8; 4];
            dev.read_exact_at(off, &mut b)?;
            if u32::from_le_bytes(b) == RAID_MAGIC {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

const INCOMPAT_EXTENTS: u32 = 0x40;
const INCOMPAT_64BIT: u32 = 0x80;
const INCOMPAT_FLEX_BG: u32 = 0x200;
const COMPAT_HAS_JOURNAL: u32 = 0x4;
/// huge_file, gdt_csum, dir_nlink, extra_isize, metadata_csum.
const RO_COMPAT_EXT4_ONLY: u32 = 0x8 | 0x10 | 0x20 | 0x40 | 0x400;

/// Incompatible features ext4-view refuses to mount, with their e2fsprogs names.
const UNSUPPORTED_INCOMPAT: [(u32, &str); 9] = [
    (0x1, "compression"),
    (0x8, "journal_dev"),
    (0x10, "meta_bg"),
    (0x100, "mmp"),
    (0x400, "ea_inode"),
    (0x1000, "dirdata"),
    (0x4000, "largedir"),
    (0x8000, "inline_data"),
    (0x2_0000, "casefold"),
];

/// Parses the 1 KiB ext superblock.
fn ext_info(sb: &[u8]) -> Option<ExtInfo> {
    let compat = le32(sb, 0x5C)?;
    let incompat = le32(sb, 0x60)?;
    let ro_compat = le32(sb, 0x64)?;
    let block_size = 1024u64
        .checked_shl(le32(sb, 0x18)?)
        .filter(|&b| b <= 65536)?;
    let is_64bit = incompat & INCOMPAT_64BIT != 0;
    let hi = |off| {
        if is_64bit {
            le32(sb, off).map(u64::from)
        } else {
            Some(0)
        }
    };
    let blocks = u64::from(le32(sb, 0x04)?) | (hi(0x150)? << 32);
    let free_blocks = u64::from(le32(sb, 0x0C)?) | (hi(0x158)? << 32);
    let flavor = if incompat & (INCOMPAT_EXTENTS | INCOMPAT_64BIT | INCOMPAT_FLEX_BG) != 0
        || ro_compat & RO_COMPAT_EXT4_ONLY != 0
    {
        "ext4"
    } else if compat & COMPAT_HAS_JOURNAL != 0 {
        "ext3"
    } else {
        "ext2"
    };
    let unsupported = UNSUPPORTED_INCOMPAT
        .iter()
        .filter(|(bit, _)| incompat & bit != 0)
        .map(|&(_, n)| n)
        .collect();
    let uuid = sb.get(0x68..0x78)?;
    let uuid = uuid
        .iter()
        .enumerate()
        .fold(String::new(), |mut s, (i, b)| {
            if matches!(i, 4 | 6 | 8 | 10) {
                s.push('-');
            }
            s.push_str(&format!("{b:02x}"));
            s
        });
    let label = sb.get(0x78..0x88)?;
    let label =
        String::from_utf8_lossy(label.split(|&b| b == 0).next().unwrap_or_default()).into_owned();
    Some(ExtInfo {
        flavor,
        unsupported,
        label,
        uuid,
        block_size,
        blocks,
        free_blocks,
    })
}

fn le16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn be16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::MemDev;

    fn put(buf: &mut [u8], off: usize, bytes: &[u8]) {
        buf[off..off + bytes.len()].copy_from_slice(bytes);
    }

    fn ident_of(build: impl FnOnce(&mut Vec<u8>)) -> Ident {
        let mut disk = vec![0u8; 1024 * 1024];
        build(&mut disk);
        identify(&MemDev(disk)).unwrap()
    }

    fn ext_sb(disk: &mut [u8], compat: u32, incompat: u32, ro_compat: u32) {
        put(disk, 1080, &0xEF53u16.to_le_bytes());
        put(disk, 1024 + 0x18, &2u32.to_le_bytes()); // 4 KiB blocks
        put(disk, 1024 + 0x04, &1000u32.to_le_bytes());
        put(disk, 1024 + 0x0C, &400u32.to_le_bytes());
        put(disk, 1024 + 0x5C, &compat.to_le_bytes());
        put(disk, 1024 + 0x60, &incompat.to_le_bytes());
        put(disk, 1024 + 0x64, &ro_compat.to_le_bytes());
        put(disk, 1024 + 0x78, b"root\0");
    }

    #[test]
    fn ext_flavors_and_unsupported_features() {
        let Ident::Ext(e2) = ident_of(|d| ext_sb(d, 0, 0x2, 0)) else {
            panic!()
        };
        assert_eq!(
            (e2.flavor, e2.block_size, e2.blocks, e2.free_blocks),
            ("ext2", 4096, 1000, 400)
        );
        assert_eq!(e2.label, "root");
        let Ident::Ext(e3) = ident_of(|d| ext_sb(d, COMPAT_HAS_JOURNAL, 0x2, 0)) else {
            panic!()
        };
        assert_eq!(e3.flavor, "ext3");
        let Ident::Ext(e4) =
            ident_of(|d| ext_sb(d, COMPAT_HAS_JOURNAL, 0x2 | 0x40 | 0x8000 | 0x2_0000, 0))
        else {
            panic!()
        };
        assert_eq!(e4.flavor, "ext4");
        assert_eq!(e4.unsupported, vec!["inline_data", "casefold"]);
    }

    #[test]
    fn container_and_foreign_signatures() {
        assert_eq!(
            ident_of(|d| put(d, 0, b"LUKS\xBA\xBE\x00\x02")),
            Ident::Luks { version: 2 }
        );
        assert_eq!(
            ident_of(|d| {
                put(d, 512, b"LABELONE");
                put(d, 512 + 24, b"LVM2 001")
            }),
            Ident::LvmMember
        );
        assert_eq!(ident_of(|d| put(d, 0, b"XFSB")), Ident::Xfs);
        assert_eq!(ident_of(|d| put(d, 0x1_0040, b"_BHRfS_M")), Ident::Btrfs);
        assert_eq!(ident_of(|d| put(d, 3, b"NTFS    ")), Ident::Windows("NTFS"));
        assert_eq!(
            ident_of(|d| put(d, 4086, b"SWAPSPACE2")),
            Ident::Other("Linux swap")
        );
        assert_eq!(
            ident_of(|d| put(d, 1024, &0xF2F5_2010u32.to_le_bytes())),
            Ident::Other("f2fs")
        );
        assert_eq!(
            ident_of(|d| put(d, 4096, &RAID_MAGIC.to_le_bytes())),
            Ident::Other("Linux RAID member")
        );
        assert_eq!(ident_of(|_| {}), Ident::Unknown);
    }

    #[test]
    fn raid_metadata_at_the_end_wins_over_a_filesystem_at_the_start() {
        let id = ident_of(|d| {
            ext_sb(d, 0, 0x2, 0);
            let len = d.len();
            put(d, len - 65536, &RAID_MAGIC.to_le_bytes()); // metadata 0.90
        });
        assert_eq!(id, Ident::Other("Linux RAID member"));
    }

    #[test]
    fn tiny_devices_do_not_error() {
        assert_eq!(identify(&MemDev(vec![0u8; 100])).unwrap(), Ident::Unknown);
    }
}
