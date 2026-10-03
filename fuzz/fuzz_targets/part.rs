#![no_main]
//! GPT and MBR parsing (gptman, mbrman and our own checks) on arbitrary bytes.

use libfuzzer_sys::fuzz_target;
use tuxread_core::dev::MemDev;
use tuxread_core::part;

fuzz_target!(|data: &[u8]| {
    let dev = MemDev(data.to_vec());
    let _ = part::read_gpt(&dev);
    let _ = part::read_mbr(&dev, 512);
    let _ = part::read_mbr(&dev, 4096);
});
