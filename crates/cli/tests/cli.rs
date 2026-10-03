//! tuxread-cli as a user runs it. The checks need Windows disks, so the file is Windows-only.
#![cfg(windows)]

use std::process::Command;

/// A disk number that does not exist is reported at once, before any UAC prompt for the helper.
#[test]
fn a_disk_that_does_not_exist_is_named_without_asking_for_admin() {
    let out = Command::new(env!("CARGO_BIN_EXE_tuxread-cli"))
        .args(["probe", "disk:4000"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("disk:4000: no disk 4000 (`tuxread-cli disks` lists them)"),
        "{stderr}"
    );
}
