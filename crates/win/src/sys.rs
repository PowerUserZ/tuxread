//! The only `unsafe` code in TuxRead: one small wrapper per Win32 call.
//! Everything std can do (opening devices and pipes, reading at an offset) goes through std.

use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::ptr;

use windows_sys::Win32::System::IO::DeviceIoControl;

/// `DeviceIoControl` with a byte input, returning at most `out_len` output bytes.
pub fn ioctl(file: &File, code: u32, input: &[u8], out_len: usize) -> io::Result<Vec<u8>> {
    let mut out = vec![0u8; out_len];
    let mut returned = 0u32;
    // SAFETY: the handle stays open while `file` is borrowed, both buffers outlive this
    // synchronous call, and their real lengths are passed alongside them.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            input.as_ptr().cast(),
            input.len() as u32,
            out.as_mut_ptr().cast(),
            out_len as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    out.truncate(returned as usize);
    Ok(out)
}
