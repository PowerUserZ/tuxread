//! TuxRead engine: read-only access to Linux filesystems inside disks and images.
//!
//! Nothing in this crate can write to a source: [`dev::BlockDev`] has no write method.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod dev;
pub mod error;

pub use error::{Error, Result};
