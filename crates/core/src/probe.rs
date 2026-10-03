//! Builds the tree a user sees under a disk or image: partitions, then what each holds.

use std::sync::Arc;

use crate::dev::{BlockDev, SliceDev};
use crate::fs::ext::ExtFs;
use crate::fs::{Fs, FsInfo};
use crate::ident::{self, ExtInfo, Ident};
use crate::part::{self, Partition, Table};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// NTFS, FAT, exFAT, ReFS, BitLocker.
    WindowsCanOpen,
    /// Supported in a later TuxRead version (LUKS, LVM, Btrfs, XFS in v0.1).
    Later,
    NotSupported(String),
    Unrecognized,
    Error(String),
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
            let child = match SliceDev::new(dev.clone(), p.start, p.len) {
                Ok(slice) => content_node(Arc::new(slice)),
                Err(e) => detected("Unreadable", p.len, Status::Error(e.to_string())),
            };
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
        assert!(matches!(
            &leaves[0].kind,
            NodeKind::Detected {
                status: Status::WindowsCanOpen,
                ..
            }
        ));
    }
}
