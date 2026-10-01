//! Inputs found by fuzzing and review. A counting allocator catches allocations sized from
//! untrusted on-disk counts: Windows may grant a huge reservation, where libFuzzer's limit
//! would not.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tuxread_core::dev::{BlockDev, MemDev};
use tuxread_core::fs::FsInfo;
use tuxread_core::fs::ext::ExtFs;
use tuxread_core::part;
use tuxread_core::probe::{self, NodeKind, Status};

static LARGEST: AtomicUsize = AtomicUsize::new(0);

struct RecordLargest;

unsafe impl GlobalAlloc for RecordLargest {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: RecordLargest = RecordLargest;

/// Runs `f` with no other test measuring, and returns the largest single allocation.
fn largest_allocation<T>(f: impl FnOnce() -> T) -> (T, usize) {
    static ALONE: Mutex<()> = Mutex::new(());
    let _alone = ALONE.lock().unwrap_or_else(|e| e.into_inner());
    LARGEST.store(0, Ordering::Relaxed);
    let out = f();
    (out, LARGEST.load(Ordering::Relaxed))
}

/// A large device that reads as `head` followed by zeros, without holding the zeros.
struct Sparse {
    head: Vec<u8>,
    len: u64,
}

impl BlockDev for Sparse {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        tuxread_core::dev::check_range(offset, buf.len(), self.len)?;
        buf.fill(0);
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.head.len());
        let src = &self.head[start..];
        let n = src.len().min(buf.len());
        buf[..n].copy_from_slice(&src[..n]);
        Ok(())
    }
}

#[test]
fn huge_block_group_count_is_refused_without_reserving_memory() {
    // Fuzz OOM 5ec148c8: a 2 KB ext image claiming 1.6 billion block groups made
    // ext4-view reserve 24 GiB before reading the first descriptor.
    let image = include_bytes!("data/fuzz-probe-oom-block-groups.img");
    let (nodes, largest) = largest_allocation(|| probe::probe(Arc::new(MemDev(image.to_vec()))));
    let leaves = probe::leaves(&nodes);
    assert!(
        matches!(
            leaves[0].kind,
            NodeKind::Detected {
                status: Status::Error(_),
                ..
            }
        ),
        "{}",
        leaves[0].label
    );
    assert!(largest < 64 << 20, "largest allocation: {largest} bytes");
}

#[test]
fn block_groups_that_fit_the_device_are_still_capped() {
    // Review finding: a superblock consistent with a large device can still claim millions
    // of tiny block groups, and ext4-view keeps every group descriptor in memory.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiny-ext4.img");
    let mut head = std::fs::read(path).unwrap();
    let sb = 1024;
    let blocks: u32 = 1 << 25; // 1 KiB blocks: a 32 GiB filesystem
    head[sb + 0x04..sb + 0x08].copy_from_slice(&blocks.to_le_bytes());
    head[sb + 0x20..sb + 0x24].copy_from_slice(&4u32.to_le_bytes()); // blocks per group
    let ro_compat = u32::from_le_bytes(head[sb + 0x64..sb + 0x68].try_into().unwrap());
    let ro_compat = ro_compat & !0x400; // no metadata_csum, so the edit needs no checksum
    head[sb + 0x64..sb + 0x68].copy_from_slice(&ro_compat.to_le_bytes());
    let dev = Sparse {
        head,
        len: u64::from(blocks) * 1024,
    };

    let (opened, largest) =
        largest_allocation(|| ExtFs::open(Arc::new(dev), FsInfo::default()).is_ok());
    assert!(!opened, "8 million block groups were loaded");
    assert!(largest < 64 << 20, "largest allocation: {largest} bytes");
}

/// CRC-32 (ISO-HDLC), as GPT headers use.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn a_gpt_header_claiming_millions_of_entries_reserves_nothing() {
    // Review finding: gptman reserves memory for the header's entry count before reading
    // the entries. Forging the header CRC is easy, and a failed allocation aborts.
    let mut disk = vec![0u8; 2048];
    let header = &mut disk[512..512 + 92];
    header[0..8].copy_from_slice(b"EFI PART");
    header[8..12].copy_from_slice(&[0, 0, 1, 0]);
    header[12..16].copy_from_slice(&92u32.to_le_bytes());
    header[24..32].copy_from_slice(&1u64.to_le_bytes()); // this header's LBA
    header[72..80].copy_from_slice(&2u64.to_le_bytes()); // entries start at LBA 2
    header[80..84].copy_from_slice(&0x0800_0000u32.to_le_bytes()); // 134 million entries
    header[84..88].copy_from_slice(&128u32.to_le_bytes());
    let crc = crc32(header);
    header[16..20].copy_from_slice(&crc.to_le_bytes());

    let (table, largest) = largest_allocation(|| part::read_gpt(&MemDev(disk)));
    assert!(table.is_none());
    assert!(largest < 64 << 20, "largest allocation: {largest} bytes");
}
