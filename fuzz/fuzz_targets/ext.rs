#![no_main]
//! ext4-view through our adapter, skipping signature checks so mutated images reach it.

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use tuxread_core::dev::MemDev;
use tuxread_core::fs::ext::ExtFs;
use tuxread_core::fs::FsInfo;

fuzz_target!(|data: &[u8]| {
    if let Ok(fs) = ExtFs::open(Arc::new(MemDev(data.to_vec())), FsInfo::default()) {
        tuxread_fuzz::walk(&fs);
    }
});
