//! Windows physical disks and the elevated disk helper (spec §5.6, §5.7).
//!
//! All `unsafe` code of TuxRead lives in `sys`, one small wrapper per Win32 call, so that
//! `tuxread-core` stays free of it and keeps building on Linux.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]

pub mod align;
pub mod proto;

#[cfg(windows)]
pub mod disk;
#[cfg(windows)]
mod sys;
