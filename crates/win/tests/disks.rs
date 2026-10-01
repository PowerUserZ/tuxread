//! Disk listing and reading on the machine running the tests.
#![cfg(windows)]

use std::path::Path;

use tuxread_core::dev::BlockDev;
use tuxread_win::disk::{WinDisk, list_disks};

#[test]
fn every_machine_lists_at_least_one_disk_without_admin_rights() {
    let disks = list_disks();
    assert!(!disks.is_empty());
    for d in &disks {
        assert!(d.size > 0, "{d:?}");
        assert!([512, 4096].contains(&d.logical_sector), "{d:?}");
        assert!(d.physical_sector >= d.logical_sector, "{d:?}");
    }
    let mut numbers: Vec<_> = disks.iter().map(|d| d.number).collect();
    numbers.dedup();
    assert_eq!(numbers.len(), disks.len());
    eprintln!("{disks:#?}");
}

#[test]
fn a_file_read_through_windisk_matches_its_bytes_at_any_offset() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/data/tiny-ext4.img");
    let expected = std::fs::read(&path).unwrap();
    let disk = WinDisk::open_path(&path).unwrap();
    assert_eq!(disk.len(), expected.len() as u64);
    assert_eq!(disk.sector_size(), 512);
    for (offset, len) in [
        (0usize, 512usize),
        (1, 1),
        (1023, 3000),
        (524_287, 1),
        (100, 200_000),
    ] {
        let mut buf = vec![0u8; len];
        disk.read_exact_at(offset as u64, &mut buf).unwrap();
        assert_eq!(buf, expected[offset..offset + len], "offset {offset}");
    }
    let mut past_end = [0u8; 2];
    assert!(disk.read_exact_at(524_287, &mut past_end).is_err());
}
