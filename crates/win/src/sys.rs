//! The only `unsafe` code in TuxRead: one small wrapper per Win32 call.
//! Everything std can do (opening devices and pipes, reading at an offset) goes through std.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;

use windows_sys::Win32::Foundation::{
    ERROR_NO_DATA, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE, LocalFree, STATUS_SUCCESS,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    INFINITE, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, WaitForSingleObject,
};

/// A NUL-terminated UTF-16 copy of `s`.
fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// The string at `p`, up to its NUL, then `LocalFree`s it.
///
/// # Safety
/// `p` must be a NUL-terminated UTF-16 string allocated with `LocalAlloc`.
unsafe fn take_local_string(p: *mut u16) -> String {
    let mut len = 0;
    // SAFETY: the caller guarantees a NUL terminator, so every read is in bounds.
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` units were just read and are initialized.
    let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) });
    // SAFETY: the caller guarantees `p` came from `LocalAlloc`; it is not used again.
    unsafe { LocalFree(p.cast()) };
    s
}

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

/// 16 bytes from the system's cryptographic random generator, as hex.
pub fn random_hex() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    // SAFETY: the buffer and its length match; no algorithm handle is needed with
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG.
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != STATUS_SUCCESS {
        return Err(io::Error::other(format!(
            "BCryptGenRandom failed: {status:#x}"
        )));
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn process_token(process: HANDLE) -> io::Result<OwnedHandle> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` is a valid process handle and `token` receives a new handle.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken succeeded, so `token` is a handle we now own.
    Ok(unsafe { OwnedHandle::from_raw_handle(token) })
}

/// A process we may query and wait for.
pub struct Process(OwnedHandle);

impl Process {
    pub fn open(pid: u32) -> io::Result<Self> {
        // SAFETY: plain call; the result is checked for null below.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcess succeeded, so we own this handle.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    /// The full path of the process's executable.
    pub fn image_path(&self) -> io::Result<OsString> {
        let mut buf = vec![0u16; 32 * 1024];
        let mut len = buf.len() as u32;
        // SAFETY: `len` holds the buffer's capacity in characters, as the call requires.
        let ok = unsafe {
            QueryFullProcessImageNameW(
                self.0.as_raw_handle(),
                PROCESS_NAME_WIN32,
                buf.as_mut_ptr(),
                &mut len,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(OsString::from_wide(
            buf.get(..len as usize).unwrap_or_default(),
        ))
    }

    /// The process owner's SID in string form ("S-1-5-21-...").
    pub fn user_sid(&self) -> io::Result<String> {
        let token = process_token(self.0.as_raw_handle())?;
        // A TOKEN_USER followed by the SID it points to; `u64`s keep it aligned.
        let mut buf = [0u64; 64];
        let mut returned = 0u32;
        // SAFETY: the buffer and its byte size match.
        let ok = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buf.as_mut_ptr().cast(),
                size_of_val(&buf) as u32,
                &mut returned,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success the buffer starts with an initialized, aligned TOKEN_USER.
        let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
        let mut text: *mut u16 = ptr::null_mut();
        // SAFETY: `user.User.Sid` points into `buf`, which is still alive.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: ConvertSidToStringSidW returns a LocalAlloc'd, NUL-terminated string.
        Ok(unsafe { take_local_string(text) })
    }

    /// Blocks until the process exits.
    pub fn wait(&self) {
        // SAFETY: the handle has SYNCHRONIZE access.
        unsafe { WaitForSingleObject(self.0.as_raw_handle(), INFINITE) };
    }
}

/// One server instance of the byte pipe `name`, open only to the principals of the
/// `sddl` security descriptor and never to remote clients. With `first`, creation fails
/// if any process already owns the name.
pub fn create_pipe(name: &str, first: bool, sddl: &str, out_buffer: u32) -> io::Result<File> {
    let sddl_w = wide(OsStr::new(sddl));
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: the SDDL string is NUL-terminated; `descriptor` receives a LocalAlloc'd value.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_w.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let name_w = wide(OsStr::new(name));
    // SAFETY: the name is NUL-terminated and `attributes` points to a valid descriptor.
    let handle = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            out_buffer,
            4096,
            0,
            &attributes,
        )
    };
    let error = io::Error::last_os_error();
    // SAFETY: the pipe keeps its own copy of the descriptor; ours is freed once.
    unsafe { LocalFree(descriptor) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(error);
    }
    // SAFETY: CreateNamedPipeW succeeded, so we own the handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Waits for a client to connect to a server pipe instance.
pub fn accept(pipe: &File) -> io::Result<()> {
    // SAFETY: a valid, synchronous server pipe handle; no OVERLAPPED.
    if unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    // A client that connected between creation and this call is still a connection, and
    // so is one that has already left again (the first read then finds the pipe closed).
    let code = error.raw_os_error();
    if code == Some(ERROR_PIPE_CONNECTED as i32) || code == Some(ERROR_NO_DATA as i32) {
        Ok(())
    } else {
        Err(error)
    }
}

pub fn pipe_client_pid(pipe: &File) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: a valid pipe handle and a u32 out parameter.
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

pub fn pipe_server_pid(pipe: &File) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: a valid pipe handle and a u32 out parameter.
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

impl Process {
    pub fn id(&self) -> u32 {
        use windows_sys::Win32::System::Threading::GetProcessId;
        // SAFETY: the handle is a valid process handle.
        unsafe { GetProcessId(self.0.as_raw_handle()) }
    }

    pub fn has_exited(&self) -> bool {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        // SAFETY: the handle has SYNCHRONIZE access.
        unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) == WAIT_OBJECT_0 }
    }
}

/// Starts `exe params` with a hidden window, through UAC (`runas`) when `elevate`.
/// A declined UAC prompt fails with `ERROR_CANCELLED`.
pub fn shell_execute(exe: &std::path::Path, params: &str, elevate: bool) -> io::Result<Process> {
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
        ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let verb = wide(OsStr::new(if elevate { "runas" } else { "open" }));
    let file = wide(exe.as_os_str());
    let params = wide(OsStr::new(params));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: params.as_ptr(),
        nShow: SW_HIDE,
        ..Default::default()
    };
    // SAFETY: `info` is fully initialized and every string it points to outlives the call.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.hProcess.is_null() {
        return Err(io::Error::other(
            "no process handle for the started program",
        ));
    }
    // SAFETY: SEE_MASK_NOCLOSEPROCESS hands us ownership of `hProcess`.
    Ok(Process(unsafe {
        OwnedHandle::from_raw_handle(info.hProcess)
    }))
}

/// True when this process runs with administrator rights (elevated).
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Security::{TOKEN_ELEVATION, TokenElevation};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no closing.
    let process = unsafe { GetCurrentProcess() };
    let Ok(token) = process_token(process) else {
        return false;
    };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    // SAFETY: the output buffer is a TOKEN_ELEVATION of the size passed.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&raw mut elevation).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    ok != 0 && elevation.TokenIsElevated != 0
}
