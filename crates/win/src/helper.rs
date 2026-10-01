//! The elevated disk helper and its client (spec §5.7).
//!
//! The non-elevated app starts `<its own exe> --disk-helper --parent <pid> --pipe <name>`
//! through UAC. The helper is the pipe server: it serves sector-aligned reads of
//! `\\.\PhysicalDrive<N>`, to its parent process only, until the parent exits.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tuxread_core::dev::BlockDev;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;

use crate::align::{MAX_IO, read_aligned};
use crate::disk::WinDisk;
use crate::proto::{Reply, Request};
use crate::sys::{self, Process};

/// The command-line flag that turns the app into the helper.
pub const FLAG: &str = "--disk-helper";

/// How long a client waits for a starting helper's pipe to appear.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperArgs {
    /// The process the helper serves, and outlives not.
    pub parent: u32,
    pub pipe: String,
}

/// `--disk-helper --parent <pid> --pipe <name>`, found anywhere in `args`.
pub fn parse_args(args: &[String]) -> Option<HelperArgs> {
    if !args.iter().any(|a| a == FLAG) {
        return None;
    }
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    let pipe = value("--pipe")?.clone();
    if !pipe.starts_with(r"\\.\pipe\tuxread-") {
        return None;
    }
    Some(HelperArgs {
        parent: value("--parent")?.parse().ok()?,
        pipe,
    })
}

/// A fresh `\\.\pipe\tuxread-<128-bit random hex>` name.
pub fn new_pipe_name() -> io::Result<String> {
    Ok(format!(r"\\.\pipe\tuxread-{}", sys::random_hex()?))
}

/// The pipe's security descriptor: full access for one user only (the parent's), and a
/// Medium integrity label so the non-elevated app may connect to an elevated helper.
pub fn pipe_sddl(user_sid: &str) -> String {
    format!("D:P(A;;GA;;;{user_sid})S:(ML;;NW;;;ME)")
}

/// The helper's `main`: serves physical disks until the parent exits.
pub fn run(args: &HelperArgs) -> io::Result<()> {
    exit_with_parent(args.parent)?;
    serve(args, WinDisk::open)
}

/// Ends this process when `parent` exits.
pub fn exit_with_parent(parent: u32) -> io::Result<()> {
    let parent = Process::open(parent)?;
    std::thread::spawn(move || {
        parent.wait();
        std::process::exit(0);
    });
    Ok(())
}

/// Accepts connections on `args.pipe` from the parent process only and serves each on its
/// own thread with its own disk from `open`. Returns only on an error.
pub fn serve<F>(args: &HelperArgs, open: F) -> io::Result<()>
where
    F: Fn(u32) -> io::Result<WinDisk> + Send + Sync + 'static,
{
    let parent = Process::open(args.parent)?;
    let own_image = Process::open(std::process::id())?.image_path()?;
    let sddl = pipe_sddl(&parent.user_sid()?);
    let open = Arc::new(open);
    let mut first = true;
    loop {
        // FILE_FLAG_FIRST_PIPE_INSTANCE on the first instance: if another process
        // already owns the name, fail instead of sharing it.
        let pipe = sys::create_pipe(&args.pipe, first, &sddl, (MAX_IO + 64) as u32)?;
        first = false;
        sys::accept(&pipe)?;
        if is_parent(&pipe, args.parent, &own_image) {
            let open = Arc::clone(&open);
            std::thread::spawn(move || {
                let _ = serve_connection(pipe, open.as_ref());
            });
        }
        // Anything else is dropped, which closes its connection.
    }
}

/// The client is our parent process, running the same executable as we do.
fn is_parent(pipe: &File, parent: u32, own_image: &OsStr) -> bool {
    let Ok(pid) = sys::pipe_client_pid(pipe) else {
        return false;
    };
    let image = Process::open(pid).and_then(|p| p.image_path());
    pid == parent
        && image.is_ok_and(|i| {
            i.to_string_lossy()
                .eq_ignore_ascii_case(&own_image.to_string_lossy())
        })
}

fn serve_connection(mut pipe: File, open: &dyn Fn(u32) -> io::Result<WinDisk>) -> io::Result<()> {
    let disk = match Request::read_from(&mut pipe)? {
        Some(Request::Open { disk }) => open(disk),
        Some(Request::Read { .. }) => Err(io::Error::other("no disk is open")),
        None => return Ok(()),
    };
    let disk = match disk {
        Ok(disk) => disk,
        Err(e) => return failed(&e).write_to(&mut pipe),
    };
    let sector = disk.sector_size();
    Reply::Opened {
        size: disk.len(),
        sector,
    }
    .write_to(&mut pipe)?;
    while let Some(request) = Request::read_from(&mut pipe)? {
        let reply = match request {
            Request::Read { offset, len } => read(&disk, u64::from(sector), offset, len),
            Request::Open { .. } => failed(&io::Error::other("a disk is already open")),
        };
        reply.write_to(&mut pipe)?;
    }
    Ok(())
}

fn read(disk: &WinDisk, sector: u64, offset: u64, len: u32) -> Reply {
    let len_u64 = u64::from(len);
    let in_range = offset
        .checked_add(len_u64)
        .is_some_and(|end| end <= disk.len());
    if !offset.is_multiple_of(sector)
        || !len_u64.is_multiple_of(sector)
        || len as usize > MAX_IO
        || !in_range
    {
        return failed(&io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refused read of {len} bytes at {offset}: not aligned or out of range"),
        ));
    }
    let mut data = vec![0u8; len as usize];
    match disk.read_exact_at(offset, &mut data) {
        Ok(()) => Reply::Data(data),
        Err(e) => failed(&e),
    }
}

fn failed(e: &io::Error) -> Reply {
    Reply::Failed {
        code: e.raw_os_error().unwrap_or(0) as u32,
        message: e.to_string(),
    }
}

/// A disk read through the helper, as a `BlockDev`.
pub struct HelperDisk {
    pipe: Mutex<File>,
    len: u64,
    sector: u32,
}

impl HelperDisk {
    /// Connects to `pipe`, which must be served by process `helper_pid`, and opens disk
    /// `number`. Retries while the pipe does not exist yet and `alive()` holds.
    pub fn connect(
        pipe: &str,
        helper_pid: u32,
        number: u32,
        alive: &dyn Fn() -> bool,
    ) -> io::Result<Self> {
        let mut pipe = connect_pipe(pipe, helper_pid, alive)?;
        Request::Open { disk: number }.write_to(&mut pipe)?;
        match Reply::read_from(&mut pipe)? {
            Reply::Opened { size, sector } => Ok(Self {
                pipe: Mutex::new(pipe),
                len: size,
                sector,
            }),
            Reply::Failed { code, message } => Err(helper_error(code, message)),
            Reply::Data(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected reply",
            )),
        }
    }

    pub fn sector_size(&self) -> u32 {
        self.sector
    }
}

fn connect_pipe(name: &str, helper_pid: u32, alive: &dyn Fn() -> bool) -> io::Result<File> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        // SECURITY_IDENTIFICATION: the helper may learn who we are, not act as us.
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name);
        match opened {
            Ok(pipe) => {
                if sys::pipe_server_pid(&pipe)? != helper_pid {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "the pipe is not served by the disk helper",
                    ));
                }
                return Ok(pipe);
            }
            Err(e) => {
                let starting = [ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY]
                    .iter()
                    .any(|&code| e.raw_os_error() == Some(code as i32));
                if !starting || !alive() || Instant::now() > deadline {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn helper_error(code: u32, message: String) -> io::Error {
    if code == 0 {
        io::Error::other(format!("disk helper: {message}"))
    } else {
        io::Error::from_raw_os_error(code as i32)
    }
}

impl BlockDev for HelperDisk {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut pipe = self
            .pipe
            .lock()
            .map_err(|_| io::Error::other("disk helper connection poisoned"))?;
        read_aligned(
            offset,
            buf,
            u64::from(self.sector),
            self.len,
            &mut |pos, chunk| {
                let len = u32::try_from(chunk.len()).map_err(io::Error::other)?;
                Request::Read { offset: pos, len }.write_to(&mut *pipe)?;
                match Reply::read_from(&mut *pipe)? {
                    Reply::Data(data) if data.len() == chunk.len() => {
                        chunk.copy_from_slice(&data);
                        Ok(())
                    }
                    Reply::Failed { code, message } => Err(helper_error(code, message)),
                    _ => Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected reply",
                    )),
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn helper_arguments_are_found_and_checked() {
        let pipe = r"\\.\pipe\tuxread-00ff";
        let args = strings(&["app.exe", FLAG, "--parent", "42", "--pipe", pipe]);
        assert_eq!(
            parse_args(&args),
            Some(HelperArgs {
                parent: 42,
                pipe: pipe.into()
            })
        );
        assert_eq!(
            parse_args(&strings(&["app.exe", "--parent", "42", "--pipe", pipe])),
            None
        );
        assert_eq!(
            parse_args(&strings(&[FLAG, "--parent", "x", "--pipe", pipe])),
            None
        );
        assert_eq!(
            parse_args(&strings(&[
                FLAG,
                "--parent",
                "1",
                "--pipe",
                r"\\.\pipe\other"
            ])),
            None
        );
        assert_eq!(
            parse_args(&strings(&[FLAG, "--parent", "1", "--pipe"])),
            None
        );
    }

    #[test]
    fn pipe_names_are_random_and_recognizable() {
        let a = new_pipe_name().unwrap();
        let b = new_pipe_name().unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with(r"\\.\pipe\tuxread-") && a.len() == r"\\.\pipe\tuxread-".len() + 32);
    }

    #[test]
    fn the_pipe_grants_one_user_at_medium_integrity() {
        assert_eq!(
            pipe_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)S:(ML;;NW;;;ME)"
        );
    }
}
