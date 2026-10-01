# Vendored ext4-view

Source: https://github.com/nicholasbishop/ext4-view-rs, tag `ext4-view-v1.0.0`
(commit 8facbf4), exported with `git archive`. License: MIT OR Apache-2.0
(see LICENSE-MIT, LICENSE-APACHE). Used through `[patch.crates-io]` in the
workspace `Cargo.toml`.

Changes (found by the TuxRead corpus, spec §7):

1. `src/timestamp.rs`: decode the inode time base field as signed 32-bit, like
   the kernel. 1.0.0 rejected every time before 1970 or after 2038.
   Regression test: `tuxread_patch_tests` in the same file.
2. `src/iters/file_blocks/block_map.rs`: a zero indirect block index is a hole
   for the whole level. 1.0.0 read block 0 and failed on sparse ext2/ext3 files.
3. `src/block_group.rs` (found by the `probe` fuzz target): the descriptor list
   grows as descriptors are read. 1.0.0 reserved capacity for the superblock's
   block group count, so a 2 KB corrupt image asked for 24 GiB.
   Regression test: `crates/core/tests/fuzz_regressions.rs`.
4. `src/metadata.rs`, `src/inode.rs`: `Metadata::inode()` returns the inode
   number, so the copy engine can recognise a folder loop in a damaged
   filesystem. Test: `copy::tests::a_folder_that_contains_itself_is_skipped`.

All four are to be proposed upstream as small, hand-written pull requests (the
project requires the Google CLA). Delete this directory and the patch entry
once a release contains all four changes.
