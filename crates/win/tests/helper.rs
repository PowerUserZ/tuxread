//! The disk helper end to end: protocol, identity checks, a separate process, lifetime.
//!
//! This test has its own `main` (`harness = false`). Started as
//! `<this exe> --disk-helper --parent <pid> --pipe <name> --serve-image <file>`, it *is* the
//! helper, serving an image file instead of a physical disk. Client and helper then share
//! one executable, as `TuxRead.exe` and its helper do. No step needs admin rights.

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    windows::main()
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod windows {
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitCode};
    use std::sync::Arc;
    use std::time::Duration;

    use tuxread_core::dev::BlockDev;
    use tuxread_core::probe::{self, NodeKind};
    use tuxread_win::disk::WinDisk;
    use tuxread_win::helper::{self, HelperArgs, HelperDisk};
    use tuxread_win::proto::{Reply, Request};

    fn image() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/data/tiny-ext4.img")
    }

    fn flag_value(args: &[String], flag: &str) -> Option<String> {
        let i = args.iter().position(|a| a == flag)?;
        args.get(i + 1).cloned()
    }

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().collect();
        if let Some(helper_args) = helper::parse_args(&args) {
            // Helper mode, as `TuxRead.exe --disk-helper`; `--serve-image` serves a file instead
            // of physical disks.
            let result = match flag_value(&args, "--serve-image") {
                Some(image) => helper::exit_with_parent(helper_args.parent).and_then(|()| {
                    helper::serve(&helper_args, move |_| WinDisk::open_path(Path::new(&image)))
                }),
                None => helper::run(&helper_args),
            };
            eprintln!("helper stopped: {result:?}");
            return ExitCode::FAILURE;
        }
        if let Some(ms) = flag_value(&args, "--sleep-ms") {
            std::thread::sleep(Duration::from_millis(ms.parse().unwrap_or(0)));
            return ExitCode::SUCCESS;
        }

        let tests: &[(&str, fn())] = &[
            (
                "an_in_process_helper_serves_an_ext4_image",
                an_in_process_helper_serves_an_ext4_image,
            ),
            (
                "unaligned_or_oversized_reads_are_refused",
                unaligned_or_oversized_reads_are_refused,
            ),
            (
                "a_squatted_pipe_name_stops_the_helper",
                a_squatted_pipe_name_stops_the_helper,
            ),
            (
                "only_the_parent_process_is_served",
                only_the_parent_process_is_served,
            ),
            (
                "a_pipe_from_another_server_is_refused",
                a_pipe_from_another_server_is_refused,
            ),
            (
                "a_launched_helper_serves_and_exits_with_its_parent",
                a_launched_helper_serves_and_exits_with_its_parent,
            ),
            (
                "a_real_disk_reads_like_its_image",
                a_real_disk_reads_like_its_image,
            ),
            (
                "a_missing_disk_is_a_clear_error",
                a_missing_disk_is_a_clear_error,
            ),
            (
                "a_source_that_vanishes_fails_reads_without_hanging",
                a_source_that_vanishes_fails_reads_without_hanging,
            ),
            (
                "two_connections_read_at_the_same_time",
                two_connections_read_at_the_same_time,
            ),
            (
                "a_helper_that_dies_fails_reads_without_hanging",
                a_helper_that_dies_fails_reads_without_hanging,
            ),
        ];
        let filter = args.iter().skip(1).find(|a| !a.starts_with('-'));
        let mut failed = 0;
        for &(name, test) in tests {
            if filter.is_some_and(|f| !name.contains(f.as_str())) {
                continue;
            }
            match std::panic::catch_unwind(test) {
                Ok(()) => println!("test {name} ... ok"),
                Err(_) => {
                    println!("test {name} ... FAILED");
                    failed += 1;
                }
            }
        }
        println!(
            "test result: {}. {failed} failed",
            if failed == 0 { "ok" } else { "FAILED" }
        );
        if failed == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }

    /// Serves `image()` in a thread of this process, with this process as the parent.
    fn serve_here() -> HelperArgs {
        let args = HelperArgs {
            parent: std::process::id(),
            pipe: helper::new_pipe_name().unwrap(),
        };
        let served = args.clone();
        std::thread::spawn(move || helper::serve(&served, |_| WinDisk::open_path(&image())));
        args
    }

    fn read_b_txt(dev: Arc<dyn BlockDev>) -> String {
        let nodes = probe::probe(dev);
        let leaves = probe::leaves(&nodes);
        let NodeKind::Volume(volume) = &leaves[0].kind else {
            panic!("not a volume: {}", leaves[0].label);
        };
        let fs = volume.open().unwrap();
        let mut text = String::new();
        fs.open(b"/dir/b.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    fn an_in_process_helper_serves_an_ext4_image() {
        let args = serve_here();
        let disk = HelperDisk::connect(&args.pipe, std::process::id(), 0, &|| true).unwrap();
        assert_eq!(disk.len(), 524_288);
        assert_eq!(disk.sector_size(), 512);
        assert_eq!(read_b_txt(Arc::new(disk)), "beta\n");
    }

    fn unaligned_or_oversized_reads_are_refused() {
        let args = serve_here();
        let mut pipe = loop {
            match OpenOptions::new().read(true).write(true).open(&args.pipe) {
                Ok(pipe) => break pipe,
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        Request::Open { disk: 0 }.write_to(&mut pipe).unwrap();
        assert!(matches!(
            Reply::read_from(&mut pipe).unwrap(),
            Reply::Opened {
                size: 524_288,
                sector: 512
            }
        ));
        for (offset, len) in [
            (1u64, 512u32),
            (0, 100),
            (0, 8 << 20),
            (524_288, 512),
            (u64::MAX - 511, 512),
        ] {
            Request::Read { offset, len }.write_to(&mut pipe).unwrap();
            let reply = Reply::read_from(&mut pipe).unwrap();
            assert!(
                matches!(reply, Reply::Failed { .. }),
                "read {len} at {offset}: {reply:?}"
            );
        }
        Request::Read {
            offset: 1024,
            len: 512,
        }
        .write_to(&mut pipe)
        .unwrap();
        let Reply::Data(data) = Reply::read_from(&mut pipe).unwrap() else {
            panic!("aligned read refused");
        };
        assert_eq!(data[56..58], [0x53, 0xEF]); // ext superblock magic
    }

    fn a_squatted_pipe_name_stops_the_helper() {
        let args = serve_here();
        // Wait until the first server's pipe exists by connecting to it.
        HelperDisk::connect(&args.pipe, std::process::id(), 0, &|| true).unwrap();
        // Without FILE_FLAG_FIRST_PIPE_INSTANCE a second server would share the name and wait
        // for clients forever, so give it a deadline.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let second = helper::serve(&args, |_| WinDisk::open_path(&image()));
            tx.send(second.is_err())
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok(true),
            "a second server took over an existing pipe name"
        );
    }

    fn only_the_parent_process_is_served() {
        // The helper serves a sleeping child, so this process is not its parent.
        let mut sleeper = Command::new(std::env::current_exe().unwrap())
            .args(["--sleep-ms", "30000"])
            .spawn()
            .unwrap();
        let args = HelperArgs {
            parent: sleeper.id(),
            pipe: helper::new_pipe_name().unwrap(),
        };
        let served = args.clone();
        std::thread::spawn(move || helper::serve(&served, |_| WinDisk::open_path(&image())));
        let result = HelperDisk::connect(&args.pipe, std::process::id(), 0, &|| true);
        sleeper.kill().unwrap();
        sleeper.wait().unwrap();
        assert!(
            result.is_err(),
            "a process other than the parent was served"
        );
    }

    fn a_pipe_from_another_server_is_refused() {
        let args = serve_here();
        // The client expects the helper to be some other process.
        let err = HelperDisk::connect(&args.pipe, 4, 0, &|| true)
            .err()
            .unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    fn a_launched_helper_serves_and_exits_with_its_parent() {
        let exe = std::env::current_exe().unwrap();
        // A path with spaces proves that the launch quotes its arguments.
        let dir = std::env::temp_dir().join(format!("tuxread launch {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spaced = dir.join("tiny ext4.img");
        std::fs::copy(image(), &spaced).unwrap();
        let spaced_arg = spaced.display().to_string();
        let launched = helper::launch(&exe, &["--serve-image", &spaced_arg], false, 0).unwrap();
        assert!(launched.is_running());
        let disk = launched.open_disk(0).unwrap();
        assert_eq!(read_b_txt(Arc::new(disk)), "beta\n");

        // Lifetime: a helper whose parent exits, exits too.
        let mut parent = Command::new(&exe)
            .args(["--sleep-ms", "1500"])
            .spawn()
            .unwrap();
        let mut child = Command::new(&exe)
            .args([helper::FLAG, "--parent", &parent.id().to_string()])
            .args(["--pipe", &helper::new_pipe_name().unwrap()])
            .args(["--serve-image", &image().display().to_string()])
            .spawn()
            .unwrap();
        parent.wait().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the helper outlived its parent"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Spec §7.4 "real disk": with `TUXREAD_TEST_DISK=<N>` and `TUXREAD_TEST_DISK_IMAGE=<raw
    /// image>` set (disk N being that image, made into a VHD by `scripts/make-vhd.py` and
    /// attached by `scripts/attach-vhd.ps1`), reads disk N through an elevated helper and
    /// compares every byte with the image. Skipped when the variables are not set.
    fn a_real_disk_reads_like_its_image() {
        let (Ok(number), Ok(image)) = (
            std::env::var("TUXREAD_TEST_DISK"),
            std::env::var("TUXREAD_TEST_DISK_IMAGE"),
        ) else {
            println!("    skipped: TUXREAD_TEST_DISK and TUXREAD_TEST_DISK_IMAGE are not set");
            return;
        };
        let exe = std::env::current_exe().unwrap();
        let launched = helper::launch(&exe, &[], true, 0).unwrap();
        let disk = launched.open_disk(number.parse().unwrap()).unwrap();
        let mut file = std::fs::File::open(&image).unwrap();
        let len = file.metadata().unwrap().len();
        assert_eq!(disk.len(), len);
        let (mut from_disk, mut from_image) = (vec![0u8; 4 << 20], vec![0u8; 4 << 20]);
        let mut offset = 0;
        while offset < len {
            let n = (len - offset).min(4 << 20) as usize;
            disk.read_exact_at(offset, &mut from_disk[..n]).unwrap();
            file.read_exact(&mut from_image[..n]).unwrap();
            assert!(
                from_disk[..n] == from_image[..n],
                "bytes differ in the 4 MiB at {offset}"
            );
            offset += n as u64;
        }
    }

    /// Runs `f` on a thread and fails if it does not finish within `secs` seconds.
    fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(f()));
        rx.recv_timeout(Duration::from_secs(secs))
            .expect("did not finish in time")
    }

    fn a_missing_disk_is_a_clear_error() {
        let args = HelperArgs {
            parent: std::process::id(),
            pipe: helper::new_pipe_name().unwrap(),
        };
        let served = args.clone();
        std::thread::spawn(move || {
            helper::serve(&served, |_| {
                WinDisk::open_path(Path::new(r"C:\no\such\disk.img"))
            })
        });
        let err = HelperDisk::connect(&args.pipe, std::process::id(), 7, &|| true)
            .err()
            .unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
    }

    fn a_source_that_vanishes_fails_reads_without_hanging() {
        // A USB disk pulled out mid-copy: here, the served file shrinks to nothing.
        let copy = std::env::temp_dir().join(format!("tuxread-vanish-{}.img", std::process::id()));
        std::fs::copy(image(), &copy).unwrap();
        let args = HelperArgs {
            parent: std::process::id(),
            pipe: helper::new_pipe_name().unwrap(),
        };
        let (served, path) = (args.clone(), copy.clone());
        std::thread::spawn(move || helper::serve(&served, move |_| WinDisk::open_path(&path)));
        let disk = HelperDisk::connect(&args.pipe, std::process::id(), 0, &|| true).unwrap();
        let mut buf = [0u8; 512];
        disk.read_exact_at(0, &mut buf).unwrap();
        OpenOptions::new()
            .write(true)
            .open(&copy)
            .unwrap()
            .set_len(0)
            .unwrap();
        let (disk, errors) = within(10, move || {
            let errors =
                [0u64, 1024].map(|offset| disk.read_exact_at(offset, &mut [0u8; 512]).err());
            (disk, errors)
        });
        for error in errors {
            let error = error.expect("a read of a vanished source succeeded");
            // A `Failed` reply from a helper that is still serving, not a dropped connection.
            assert!(
                !error.to_string().contains("closed the connection"),
                "{error}"
            );
        }
        // The source comes back: the same connection reads again.
        OpenOptions::new()
            .write(true)
            .open(&copy)
            .unwrap()
            .write_all(&std::fs::read(image()).unwrap())
            .unwrap();
        let block = within(10, move || {
            let mut block = [0u8; 512];
            disk.read_exact_at(1024, &mut block).map(|()| block)
        })
        .unwrap();
        assert_eq!(block[56..58], [0x53, 0xEF]); // ext superblock magic
        let _ = std::fs::remove_file(&copy);
    }

    fn two_connections_read_at_the_same_time() {
        let args = serve_here();
        let expected = Arc::new(std::fs::read(image()).unwrap());
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let disk =
                    HelperDisk::connect(&args.pipe, std::process::id(), 0, &|| true).unwrap();
                let expected = Arc::clone(&expected);
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        let mut all = vec![0u8; expected.len()];
                        disk.read_exact_at(0, &mut all).unwrap();
                        assert!(all == *expected);
                    }
                })
            })
            .collect();
        for reader in readers {
            reader.join().unwrap();
        }
    }

    fn a_helper_that_dies_fails_reads_without_hanging() {
        let exe = std::env::current_exe().unwrap();
        let image_arg = image().display().to_string();
        let launched = helper::launch(&exe, &["--serve-image", &image_arg], false, 0).unwrap();
        let disk = launched.open_disk(0).unwrap();
        disk.read_exact_at(0, &mut [0u8; 512]).unwrap();
        let killed = Command::new("taskkill")
            .args(["/F", "/PID", &launched.pid().to_string()])
            .output()
            .unwrap();
        assert!(killed.status.success());
        let error = within(10, move || disk.read_exact_at(0, &mut [0u8; 512]).err()).unwrap();
        assert!(helper::is_helper_gone(&error), "{error}");
        // A new connection to the dead helper says the same, without waiting for a timeout.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while launched.is_running() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let error = within(5, move || launched.open_disk(0).err()).unwrap();
        assert!(helper::is_helper_gone(&error), "{error}");
    }
}
