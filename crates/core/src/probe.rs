//! Builds the tree a user sees under a disk or image: partitions, then what each holds.

use std::sync::Arc;

use crate::dev::{BlockDev, SliceDev};
use crate::fs::ext::ExtFs;
use crate::fs::{Fs, FsInfo};
use crate::ident::{self, ExtInfo, Ident};
use crate::part::{self, DiskId, Partition, Table, TypeCode};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// NTFS, FAT, exFAT, ReFS, BitLocker.
    WindowsCanOpen,
    /// One of those in a partition whose type Windows does not mount (one made for Linux,
    /// say): Windows shows nothing until the type changes.
    WindowsSkips(TypeFix),
    /// Supported in a later TuxRead version (LUKS, LVM, Btrfs, XFS in v0.1).
    Later,
    NotSupported(String),
    Unrecognized,
    Error(String),
}

/// The status in words, as the CLI and the diagnostics print it.
impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::WindowsCanOpen => f.write_str("Windows can open this"),
            Status::WindowsSkips(fix) => write!(
                f,
                "Windows does not mount it: its partition type is {}",
                fix.type_name
            ),
            Status::Later => f.write_str("supported in a later version"),
            Status::NotSupported(why) => write!(f, "not supported: {why}"),
            Status::Unrecognized => f.write_str("unrecognized"),
            Status::Error(e) => write!(f, "error: {e}"),
        }
    }
}

/// What makes Windows mount a partition it skips: the partition at `offset` (bytes) on the
/// disk `disk` gets the type `to` instead of `from`. Only the type changes, not the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeFix {
    pub disk: DiskId,
    pub offset: u64,
    pub from: TypeCode,
    pub to: TypeCode,
    /// `from` as the tree shows it ("Linux filesystem").
    pub type_name: String,
}

/// A browsable filesystem.
#[derive(Clone)]
pub struct Volume {
    pub dev: Arc<dyn BlockDev>,
    pub info: FsInfo,
}

impl Volume {
    /// Opens the filesystem on the calling thread (filesystem objects are not `Send`).
    pub fn open(&self) -> Result<Box<dyn Fs>> {
        Ok(Box::new(ExtFs::open(self.dev.clone(), self.info.clone())?))
    }
}

#[derive(Clone)]
pub enum NodeKind {
    /// `type_name` is the partition type ("Linux filesystem", "Basic data", "type 0x8E").
    Partition {
        number: u32,
        type_name: String,
    },
    Volume(Volume),
    Detected {
        name: String,
        status: Status,
    },
}

#[derive(Clone)]
pub struct Node {
    pub label: String,
    pub size: u64,
    pub kind: NodeKind,
    pub children: Vec<Node>,
}

/// Children of a source. Order: GPT, then a filesystem on the whole device,
/// then MBR, then "the whole device is one unrecognized region".
pub fn probe(dev: Arc<dyn BlockDev>) -> Vec<Node> {
    if let Some(table) = part::read_gpt(dev.as_ref()) {
        return partition_nodes(&dev, &table);
    }
    let whole = content_node(dev.clone());
    // md metadata 0.90 sits at the end of its partition, so a last partition that reaches the
    // end of the disk makes the whole disk look like a RAID member: a valid MBR wins then.
    let raid = matches!(
        &whole.kind,
        NodeKind::Detected { status: Status::NotSupported(why), .. } if why == ident::RAID_MEMBER
    );
    if !raid
        && !matches!(
            &whole.kind,
            NodeKind::Detected {
                status: Status::Unrecognized,
                ..
            }
        )
    {
        return vec![whole];
    }
    match part::read_mbr(dev.as_ref(), 512) {
        Ok(Some(table)) => partition_nodes(&dev, &table),
        Ok(None) => vec![whole],
        Err(_) if raid => vec![whole],
        Err(e) => vec![detected(
            "Partition table",
            dev.len(),
            Status::Error(e.to_string()),
        )],
    }
}

/// Volume and detected nodes in depth-first order: what the CLI numbers #1, #2, ...
pub fn leaves(nodes: &[Node]) -> Vec<&Node> {
    let mut out = Vec::new();
    for node in nodes {
        match node.kind {
            NodeKind::Partition { .. } => out.extend(leaves(&node.children)),
            _ => out.push(node),
        }
    }
    out
}

fn partition_nodes(dev: &Arc<dyn BlockDev>, table: &Table) -> Vec<Node> {
    table
        .partitions
        .iter()
        .map(|p| {
            let mut child = match SliceDev::new(dev.clone(), p.start, p.len) {
                Ok(slice) => content_node(Arc::new(slice)),
                Err(e) => detected("Unreadable", p.len, Status::Error(e.to_string())),
            };
            if let NodeKind::Detected { name, status } = &mut child.kind
                && *status == Status::WindowsCanOpen
                && part::hides_from_windows(p.type_code)
            {
                *status = Status::WindowsSkips(TypeFix {
                    disk: table.disk_id,
                    offset: p.start,
                    from: p.type_code,
                    to: part::windows_type(table.kind, name),
                    type_name: p.type_name.clone(),
                });
            }
            Node {
                label: partition_label(p),
                size: p.len,
                kind: NodeKind::Partition {
                    number: p.number,
                    type_name: p.type_name.clone(),
                },
                children: vec![child],
            }
        })
        .collect()
}

fn partition_label(p: &Partition) -> String {
    if p.name.is_empty() {
        format!("Partition {} ({})", p.number, p.type_name)
    } else {
        format!("Partition {} ({}, \"{}\")", p.number, p.type_name, p.name)
    }
}

fn content_node(dev: Arc<dyn BlockDev>) -> Node {
    let size = dev.len();
    let ident = match ident::identify(dev.as_ref()) {
        Ok(ident) => ident,
        Err(e) => return detected("Unreadable", size, Status::Error(e.to_string())),
    };
    match ident {
        Ident::Ext(info) => ext_node(dev, info),
        Ident::Btrfs => detected("Btrfs", size, Status::Later),
        Ident::Xfs => detected("XFS", size, Status::Later),
        Ident::Luks { version } => {
            detected(&format!("LUKS{version} encrypted"), size, Status::Later)
        }
        Ident::LvmMember => detected("LVM2 member", size, Status::Later),
        Ident::Windows(name) => detected(name, size, Status::WindowsCanOpen),
        Ident::Other(name) => detected(name, size, Status::NotSupported(name.to_string())),
        Ident::Unknown => detected("Unrecognized", size, Status::Unrecognized),
    }
}

fn ext_node(dev: Arc<dyn BlockDev>, ext: ExtInfo) -> Node {
    let size = dev.len();
    if !ext.unsupported.is_empty() {
        let why = format!("{} with {}", ext.flavor, ext.unsupported.join(", "));
        return detected(ext.flavor, size, Status::NotSupported(why));
    }
    let used = ext
        .blocks
        .saturating_sub(ext.free_blocks)
        .saturating_mul(ext.block_size);
    let info = FsInfo {
        fs_type: ext.flavor.to_string(),
        label: ext.label,
        uuid: ext.uuid,
        size,
        used,
    };
    let volume = Volume { dev, info };
    // Loading once here turns an unreadable filesystem into a labelled node instead of a later failure.
    match volume.open() {
        Ok(_) => Node {
            label: volume_label(&volume.info),
            size,
            kind: NodeKind::Volume(volume),
            children: vec![],
        },
        Err(Error::Unsupported(why)) => detected(ext.flavor, size, Status::NotSupported(why)),
        Err(e) => detected(ext.flavor, size, Status::Error(e.to_string())),
    }
}

fn volume_label(info: &FsInfo) -> String {
    if info.label.is_empty() {
        info.fs_type.clone()
    } else {
        format!("{} \"{}\"", info.fs_type, info.label)
    }
}

fn detected(name: &str, size: u64, status: Status) -> Node {
    Node {
        label: name.to_string(),
        size,
        kind: NodeKind::Detected {
            name: name.to_string(),
            status,
        },
        children: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::MemDev;

    #[test]
    fn a_raid_member_last_partition_does_not_hide_the_mbr() {
        // md metadata 0.90 sits 64 KiB before the end of its partition. When the last
        // partition reaches the end of the disk, the disk's own end shows the same superblock.
        let len = 4usize << 20;
        let mut disk = vec![0u8; len];
        for (i, sys, start, sectors) in [(0, 0x83u8, 2048u32, 2048u32), (1, 0xFD, 4096, 4096)] {
            let off = 446 + 16 * i;
            disk[off + 4] = sys;
            disk[off + 8..off + 12].copy_from_slice(&start.to_le_bytes());
            disk[off + 12..off + 16].copy_from_slice(&sectors.to_le_bytes());
        }
        disk[510..512].copy_from_slice(&[0x55, 0xAA]);
        disk[len - 65536..len - 65532].copy_from_slice(&0xA92B_4EFCu32.to_le_bytes());
        let nodes = probe(Arc::new(MemDev(disk)));
        let labels: Vec<_> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(nodes.len(), 2, "{labels:?}");
    }

    #[test]
    fn empty_device_is_one_unrecognized_leaf() {
        let nodes = probe(Arc::new(MemDev(vec![0u8; 4096])));
        let leaves = leaves(&nodes);
        assert_eq!(leaves.len(), 1);
        assert!(matches!(
            &leaves[0].kind,
            NodeKind::Detected {
                status: Status::Unrecognized,
                ..
            }
        ));
    }

    #[test]
    fn a_partition_past_the_end_of_a_truncated_image_is_an_error_leaf() {
        // MBR partition at 1 MiB of 4 MiB, but the image stops at 3 MiB.
        let mut disk = vec![0u8; 3 * 1024 * 1024];
        disk[446 + 4] = 0x83;
        disk[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
        disk[446 + 12..446 + 16].copy_from_slice(&8192u32.to_le_bytes());
        disk[510..512].copy_from_slice(&[0x55, 0xAA]);
        let nodes = probe(Arc::new(MemDev(disk)));
        let leaves = leaves(&nodes);
        assert_eq!(leaves.len(), 1);
        assert!(matches!(
            &leaves[0].kind,
            NodeKind::Detected {
                status: Status::Error(_),
                ..
            }
        ));
    }

    #[test]
    fn mbr_partitions_get_their_content_identified() {
        let mut disk = vec![0u8; 2 * 1024 * 1024];
        disk[0x1B8..0x1BC].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        // One Linux partition at sector 2048 (1 MiB) of 2048 sectors, holding an NTFS signature.
        disk[446 + 4] = 0x83;
        disk[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
        disk[446 + 12..446 + 16].copy_from_slice(&2048u32.to_le_bytes());
        disk[510..512].copy_from_slice(&[0x55, 0xAA]);
        disk[1024 * 1024 + 3..1024 * 1024 + 11].copy_from_slice(b"NTFS    ");
        let nodes = probe(Arc::new(MemDev(disk)));
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].label, "Partition 1 (Linux)");
        assert!(matches!(
            &nodes[0].kind,
            NodeKind::Partition { number: 1, type_name } if type_name == "Linux"
        ));
        let leaves = leaves(&nodes);
        // Windows mounts NTFS only in a partition typed for it (0x07), so it skips this one.
        assert!(matches!(
            &leaves[0].kind,
            NodeKind::Detected {
                status: Status::WindowsSkips(fix),
                ..
            } if *fix == TypeFix {
                disk: DiskId::Mbr(0x1234_5678),
                offset: 1024 * 1024,
                from: TypeCode::Mbr(0x83),
                to: TypeCode::Mbr(0x07),
                type_name: "Linux".into(),
            }
        ));
    }

    const DISK_GUID: [u8; 16] = [7; 16];
    const LINUX: [u8; 16] = [
        0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D,
        0xE4,
    ];
    const EFI: [u8; 16] = [
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9,
        0x3B,
    ];

    /// The status of what a GPT disk's one partition (type `ty`, at 1 MiB) holds: `content` at its start.
    fn gpt_status(ty: [u8; 16], content: &[(usize, &[u8])]) -> Status {
        let mut cursor = std::io::Cursor::new(vec![0u8; 4 << 20]);
        let mut gpt = gptman::GPT::new_from(&mut cursor, 512, DISK_GUID).unwrap();
        gpt[1] = gptman::GPTPartitionEntry {
            partition_type_guid: ty,
            unique_partition_guid: [1; 16],
            starting_lba: 2048,
            ending_lba: 4095,
            attribute_bits: 0,
            partition_name: "primary".into(),
        };
        gpt.write_into(&mut cursor).unwrap();
        let mut disk = cursor.into_inner();
        for (at, bytes) in content {
            disk[(1 << 20) + at..(1 << 20) + at + bytes.len()].copy_from_slice(bytes);
        }
        let nodes = probe(Arc::new(MemDev(disk)));
        match &leaves(&nodes)[0].kind {
            NodeKind::Detected { status, .. } => status.clone(),
            _ => panic!("not a detected leaf"),
        }
    }

    /// Linux tools give a partition the Linux type even when it is then formatted exFAT, and
    /// Windows then shows nothing (seen on a real USB disk). The fix names that partition by the
    /// disk's GUID and its offset.
    #[test]
    fn a_windows_filesystem_in_a_linux_partition_is_skipped_by_windows() {
        let exfat: &[(usize, &[u8])] = &[(3, b"EXFAT   ")];
        assert_eq!(
            gpt_status(LINUX, exfat),
            Status::WindowsSkips(TypeFix {
                disk: DiskId::Gpt(DISK_GUID),
                offset: 1 << 20,
                from: TypeCode::Gpt(LINUX),
                to: TypeCode::Gpt(part::BASIC_DATA),
                type_name: "Linux filesystem".into(),
            })
        );
        assert_eq!(gpt_status(part::BASIC_DATA, exfat), Status::WindowsCanOpen);
        // Windows' own system partitions stay as they are: no fix is offered for them.
        let fat: &[(usize, &[u8])] = &[(54, b"FAT16   "), (510, &[0x55, 0xAA])];
        assert_eq!(gpt_status(EFI, fat), Status::WindowsCanOpen);
    }

    /// In an MBR, FAT needs the FAT32 (LBA) type; NTFS and exFAT share 0x07.
    #[test]
    fn an_mbr_fix_picks_the_type_for_the_filesystem() {
        let mut disk = vec![0u8; 2 * 1024 * 1024];
        disk[446 + 4] = 0x83;
        disk[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
        disk[446 + 12..446 + 16].copy_from_slice(&2048u32.to_le_bytes());
        disk[510..512].copy_from_slice(&[0x55, 0xAA]);
        disk[(1 << 20) + 82..(1 << 20) + 90].copy_from_slice(b"FAT32   ");
        disk[(1 << 20) + 510..(1 << 20) + 512].copy_from_slice(&[0x55, 0xAA]);
        let nodes = probe(Arc::new(MemDev(disk)));
        assert!(matches!(
            &leaves(&nodes)[0].kind,
            NodeKind::Detected { status: Status::WindowsSkips(fix), .. } if fix.to == TypeCode::Mbr(0x0C)
        ));
    }
}
