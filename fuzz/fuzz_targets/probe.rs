#![no_main]
//! Whole probe chain on arbitrary bytes: tables, signatures, then every volume found.

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use tuxread_core::dev::MemDev;
use tuxread_core::probe::{self, NodeKind};

fuzz_target!(|data: &[u8]| {
    let nodes = probe::probe(Arc::new(MemDev(data.to_vec())));
    for leaf in probe::leaves(&nodes) {
        if let NodeKind::Volume(volume) = &leaf.kind
            && let Ok(fs) = volume.open()
        {
            tuxread_fuzz::walk(fs.as_ref());
        }
    }
});
