//! Inputs found by fuzzing. A counting allocator catches allocations sized from untrusted
//! on-disk counts: Windows may grant a huge reservation, where libFuzzer's limit would not.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tuxread_core::dev::MemDev;
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

#[test]
fn huge_block_group_count_is_refused_without_reserving_memory() {
    // Fuzz OOM 5ec148c8: a 2 KB ext image claiming 1.6 billion block groups made
    // ext4-view reserve 24 GiB before reading the first descriptor.
    let image = include_bytes!("data/fuzz-probe-oom-block-groups.img");
    let nodes = probe::probe(Arc::new(MemDev(image.to_vec())));
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
    let largest = LARGEST.load(Ordering::Relaxed);
    assert!(largest < 64 << 20, "largest allocation: {largest} bytes");
}
