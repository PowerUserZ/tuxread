# TuxRead — Design Spec

- **Date:** 2026-10-01
- **Status:** Design approved section by section in brainstorming; this document awaits review before implementation planning.
- **Planned repository:** `github.com/PowerUserZ/tuxread`
- **License:** MIT OR Apache-2.0
- **App identifier:** `io.github.poweruserz.tuxread`

## 1. Summary

TuxRead is a free, open-source Windows application that lets people browse Linux filesystems and copy files from them to Windows. It never writes to the source. It is a spiritual successor to [ext2read](https://github.com/mregmi/ext2read) (last commit 2017), rewritten from scratch: a Rust engine and a Tauri v2 user interface.

This spec defines the architecture for v1.0 and the exact scope of the first milestone (v0.1). Each milestone gets its own implementation plan; the first plan covers v0.1.

## 2. Goals and non-goals

### Goals (v1.0)

- Read **ext2/3/4, Btrfs and XFS** filesystems.
- Find them inside **MBR/GPT partitions, LVM2 volumes and LUKS1/LUKS2 containers**.
- Open **physical disks** (internal, USB, SD cards) and **image files** (raw, QCOW2, VHDX, VMDK).
- Explorer-like browsing, and copying out that handles Windows filename rules correctly.
- **Never write to the source.** No code path exists that could.
- **Completely free:** no paid tier, no telemetry, no network access.
- Windows 10 and 11 on x64 and ARM64. User interface in English and Turkish.

### Non-goals (v1.0)

- Writing to Linux filesystems.
- Mounting volumes as drive letters (a post-v1.0 candidate, via WinFsp).
- Dragging files out to Explorer, and built-in file preview (post-v1.0 candidates).
- f2fs, ZFS, mdraid, ReiserFS, JFS, APFS, HFS+: detected and labelled, not readable.
- Recovering deleted or corrupt data.
- Linux or macOS builds of the app. The core library stays platform-neutral except for the Windows disk module.

## 3. Background

### Why ext2read stopped working

Verified in its source (commit `a728e9f`):

- It reads ext4 group descriptors as fixed 32-byte records (`ext2fs.cpp:148`). Since e2fsprogs 1.43 (2016), `mkfs.ext4` enables the `64bit` feature by default, which makes descriptors 64 bytes. Every group except group 0 is then read from the wrong offset: the root directory lists, subdirectories come back as garbage. Its open issue #8 describes exactly this.
- It reads the superblock as "sector 2" (`ext2fs.cpp:129`). On disks with 4096-byte sectors that is the wrong offset, and the read copies 8 KiB into a 1 KiB struct.
- It stops enumerating disks at the first gap in `PhysicalDriveN` numbering and handles single digits only.
- Open issue #9 asks for the library to be separated from the GUI.

### The gap today (October 2026)

- **DiskInternals Linux Reader:** closed source. Btrfs isn't listed; full XFS and drive-letter mounting are paid.
- **7-Zip:** reads ext, but not LVM, LUKS, Btrfs or XFS.
- **WSL `wsl --mount`:** needs admin, takes the whole disk away from Windows, and doesn't support USB flash drives or SD card readers.
- **Ext2Fsd / Ext4Fsd:** kernel drivers with data-corruption and blue-screen reports as recent as 2025.

No free, open-source tool runs without a kernel driver and reads modern ext4 plus LVM/LUKS plus Btrfs/XFS from both disks and images.

## 4. Decisions

| Topic | Decision | Reason |
|---|---|---|
| Mode | Read-only browse and copy | Safe; no driver needed |
| Language | Rust | Memory safety while parsing untrusted on-disk data; single executable |
| UI | Tauri v2 + React + TypeScript + Vite | Chosen by the maintainer; flexible UI; correct text rendering for every script |
| License | MIT OR Apache-2.0 | Lets other projects reuse the readers, which brings contributors and bug reports |
| Name | TuxRead | Free on crates.io and GitHub |
| Engine strategy | Crate-first, corpus-gated (§5.4) | Reuse what is credible, write the rest |
| Privilege model | Non-elevated UI + elevated disk helper (§5.1) | Microsoft's WebView2 guidance; see §5.7 |
| Telemetry | None | Privacy; required for the SignPath privacy statement |
| Code signing | SignPath Foundation, after v0.1 ships (§9) | Free for open-source projects |

Rejected options:

- **LKL (Linux kernel as a library):** GPL-2.0-only conflicts with the license; heavy MinGW kernel build; no releases.
- **Kernel filesystem driver:** the corruption history of Ext2Fsd/Ext4Fsd.
- **Relaunching the whole app elevated:** Microsoft says to host WebView2 non-elevated. It also hits WebView2 data-folder conflicts and breaks under Windows "Administrator protection".
- **egui, native Win32, Slint:** the maintainer chose Tauri. Slint would also add an attribution or GPLv3 requirement.

## 5. Architecture

### 5.1 Processes

```
TuxRead.exe  (always non-elevated)               TuxRead.exe --disk-helper  (elevated, no window)
 ├ UI: React in WebView2                          └ opens \\.\PhysicalDrive<N> read-only and
 ├ Rust backend + tuxread-core                       serves sector-aligned reads
 │  (all parsing happens here, unprivileged)
 ├ opens image files directly           ◄──── named pipe ────►
 ├ lists disks (metadata needs no admin)
 └ writes the copied files
```

- The helper starts only when the user opens a physical disk. It costs one UAC prompt per session. If the user declines, disks stay locked and everything else keeps working.
- Disk metadata (model, size, bus type, sector size) is readable without admin, so the disk list appears immediately. Only reading content needs the helper.
- **Why this split:**
  - WebView2 never runs elevated.
  - Code that parses untrusted disk data never runs elevated.
  - Copied files are written by the normal user, so they land in the right profile with the right owner, even under Windows "Administrator protection", where elevated processes run as a separate hidden account.
  - Dropping files from Explorer onto the window keeps working.
  - No single-instance plugin or relaunch logic is needed.

### 5.2 Repository layout

```
tuxread/
├─ Cargo.toml              Cargo workspace
├─ crates/core/            tuxread-core: the engine library
├─ crates/cli/             tuxread-cli: probe / ls / cp for development and bug reports
├─ src-tauri/              Tauri backend: commands, volume workers, copy jobs, helper launch, --disk-helper entry
├─ src/                    React + TypeScript UI
├─ scripts/                make-corpus.sh, manifest.py
└─ docs/
```

`src-tauri`'s `main` checks for `--disk-helper` before building the Tauri app and hands control to the helper server in `tuxread-core`.

### 5.3 Engine layers (`tuxread-core`)

Every layer reads through one trait. There is no write method anywhere in the stack.

```rust
pub trait BlockDev: Send + Sync {
    fn len(&self) -> u64;
    /// Fills `buf` completely or returns an error. Never a short read.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()>;
}
```

From the bottom up:

1. **Sources.**
   - `FileDev`: an image file opened read-only.
   - `WinDisk`: `\\.\PhysicalDrive<N>`, opened directly. Used by the helper and by the CLI when it runs elevated.
   - `HelperDisk`: the UI-side client of the helper pipe.
2. **`CachedDev`.** Aligns every request to the source's sector size. Reads ahead in 1 MiB aligned chunks and keeps an LRU of 64 chunks per source; both values are defaults to tune with measurements. This turns `ext4-view`'s one-block reads into large reads, which cuts USB and pipe round trips.
3. **Virtual disks.** QCOW2, VHDX and VMDK images become a plain `BlockDev`.
4. **Partition tables.** GPT via `gptman` (tries 512- and 4096-byte sectors). MBR via `mbrman`, including logical partitions inside extended partitions. Our own code tells protective, hybrid and plain MBRs apart. Each partition is a `SliceDev` view.
5. **Containers.**
   - **LUKS1/LUKS2:** a passphrase unlocks the container, and the decrypted data is a `BlockDev`.
   - **LVM2:** each logical volume is a `BlockDev`. Linear and striped segments are supported, and so are volume groups spread over several disks.
6. **Filesystems.** One trait:

```rust
/// Deliberately not Send: ext4-view's `Ext4` is Rc-based, so each instance lives on one thread.
pub trait Fs {
    fn info(&self) -> FsInfo; // type, label, UUID, size, used, feature flags, warnings
    fn read_dir(&self, path: &[u8]) -> Result<Vec<Entry>>;
    fn stat(&self, path: &[u8]) -> Result<Entry>;
    fn read_link(&self, path: &[u8]) -> Result<Vec<u8>>;
    fn open(&self, path: &[u8]) -> Result<Box<dyn std::io::Read + '_>>;
}

pub struct Entry {
    pub name: Vec<u8>, // raw bytes; Linux names need not be UTF-8
    pub kind: Kind,    // File, Dir, Symlink, Other
    pub size: u64,
    pub mtime: Option<Timestamp>,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}
```

Paths are absolute, `/`-separated Linux byte paths. Implementations: `ext` (an adapter over `ext4-view`), `btrfs`, `xfs`.

7. **Probe.** Builds the volume tree described in §6.1. A volume node holds its `Arc<dyn BlockDev>` and filesystem type, and opens an `Fs` on whichever thread calls it.

### 5.4 Component sourcing

| Area | Decision |
|---|---|
| ext2/3/4 | **Adopt** `ext4-view` 1.x (maintained by Nicholas Bishop): 149 tests, CI compared against the kernel, no `unsafe`. It refuses volumes using inline_data, meta_bg, largedir, ea_inode, dirdata, mmp, journal_dev or casefold; we report those as "not supported: <feature>". |
| GPT / MBR | **Adopt** `gptman` 3.x and `mbrman` 0.6. |
| Cryptography | **Adopt** RustCrypto: `aes` 0.9, `xts-mode` 0.6, `cbc` 0.2, `pbkdf2` 0.13, `argon2` 0.6, `sha1`/`sha2` 0.11, `hmac` 0.13. Plus `zeroize` 1.x and `serde_json` 1.x. |
| Compression | **Adopt** `flate2`/`miniz_oxide` for zlib, `ruzstd` for zstd and `lzokay` for LZO, all pure Rust. The btrfs LZO segment framing (about 40 lines) is ours. |
| Windows API | **Adopt** `windows-sys` 0.61, pinned. |
| Btrfs, VHDX, VMDK | **Evaluate** `btrfs-disk` (a fork candidate), `vhdx-core` and `vmdk-core` against the corpus. If they fail, we write our own. |
| LUKS1/2, LVM2, XFS, QCOW2 | **Write our own.** Existing crates are incomplete, broken or abandoned. For example, the LVM crates read only linear volumes on a single disk. |

**Adoption gate.** A third-party crate is adopted only if it passes 100% of the corpus for its format (§7) and runs clean under our fuzz targets. Every dependency license must be compatible with MIT OR Apache-2.0. GPL, LGPL and AGPL are always denied, and `cargo-deny` enforces this.

### 5.5 Threads and IPC

- **Async commands only.** Synchronous Tauri commands run on the main thread and would freeze the UI.
- **Volume workers.** Each opened volume has a dedicated worker thread that owns its `Fs` instance. Commands send requests over a `std::sync::mpsc` channel and await a one-shot reply. Tauri `State` holds only `Send + Sync` handles: senders and registries.
- **Copy jobs.** Each copy job runs on its own thread with its own `Fs` instance over the shared `Arc<dyn BlockDev>` stack, so browsing stays responsive during long copies.
- **Streaming.** Listings and progress stream to the UI through `tauri::ipc::Channel`, not events. Listings arrive in pages of 2,000 entries; progress is limited to 10 updates per second.
- **Entry IDs.** Each worker gives entries per-volume numeric IDs mapped to their raw path bytes. The UI displays a lossy UTF-8 name and refers to entries only by ID, so files with non-UTF-8 names can still be copied.
- **Initial command set** (planning may refine names): `list_sources`, `open_image`, `open_disk`, `unlock_luks`, `list_dir`, `stat`, `copy`, `cancel_job`, `diagnostics`.

### 5.6 Windows disk access

- **Enumeration.**
  1. `SetupDiGetClassDevsW(GUID_DEVINTERFACE_DISK)`, then open each disk with access 0.
  2. `IOCTL_STORAGE_GET_DEVICE_NUMBER` gives N.
  3. `IOCTL_STORAGE_QUERY_PROPERTY` gives vendor, product and bus type (`StorageDeviceProperty`; USB = 7, SD = 12, MMC = 13), plus the logical and physical sector sizes (`StorageAccessAlignmentProperty`).
  4. `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX` gives the size.

  None of this needs admin. Drive numbers can have gaps and can change across reboots.
- **Reading.**
  - Open `\\.\PhysicalDrive<N>` with `GENERIC_READ` and share mode read|write.
  - Offsets and lengths must be multiples of the logical sector size; buffers are 4 KiB-aligned.
  - Reads are clamped to the disk size, because reading past the end fails instead of returning a short read.
- **Parallelism.** I/O on a synchronous handle is serialized, so each worker opens its own handle.
- **BitLocker.** BitLocker-protected partitions are ciphertext at the raw level. They are labelled "Windows can open this".

### 5.7 Disk helper: protocol and security

- **Launch.** The UI calls `ShellExecuteExW` with the verb `runas` and `SEE_MASK_NOCLOSEPROCESS`, passing `--disk-helper --parent <ui-pid> --pipe <name>`. The name is `\\.\pipe\tuxread-<128-bit random hex>`. If the user cancels UAC (`ERROR_CANCELLED`), disks stay locked.
- **Pipe creation.** The helper is the pipe server. It creates the pipe with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a squatted name makes it fail, and with `PIPE_REJECT_REMOTE_CLIENTS`. The security descriptor grants access only to the user of the parent UI process (read from the parent's token) and carries a Medium mandatory label so the non-elevated UI can connect.
- **Mutual verification.**
  - The helper accepts a client only if `GetNamedPipeClientProcessId` equals the parent PID and that process's image path equals the helper's own image path.
  - The UI uses a pipe only if `GetNamedPipeServerProcessId` equals the helper's PID, which it knows from the `ShellExecuteExW` process handle.
- **Operations.** Each connection serves one opened disk.
  - `Open { disk: u32 }` replies with `{ size: u64, logical_sector: u32 }`.
  - `Read { offset: u64, len: u32 }` must be sector-aligned with `len` ≤ 4 MiB, and replies with the bytes.
  - Errors carry the Win32 code and message.
  - There are no other operations, no paths and no writes.
  - The UI may open several connections (one per worker or copy job). The helper serves each on its own thread with its own disk handle.
- **Encoding.** Little-endian, length-prefixed frames, hand-written; no serialization framework.
- **Lifetime.** The helper waits on the parent's process handle and exits when the UI exits.
- **Threat model note.** UAC is not a security boundary against malware already running as the same user. These checks stop other processes from using the helper as a raw-disk reading service.

### 5.8 UI hardening

- **Command allowlist.** Tauri does not check an app's own commands unless the app declares them. `build.rs` declares our commands with `tauri_build::AppManifest`, and the capability file grants only those commands plus `dialog:allow-open`. The template's `opener` plugin is removed and `core:default` is trimmed to what we use.
- **CSP.** `default-src 'self'` plus the IPC endpoints (`ipc:` and `http://ipc.localhost`). `withGlobalTauri` and the asset protocol stay off.
- **Filenames.**
  - They are rendered as text only; there is no `dangerouslySetInnerHTML`.
  - Bidirectional overrides and other invisible or control characters are shown visibly, for example `⟨U+202E⟩`, so a name cannot disguise its extension.
- **No network.** The app makes no network requests.

### 5.9 UI outline

Visual design is settled during implementation; the structure is fixed:

- **Sidebar tree.**
  - "Disks" lists each disk with model, size and bus. Disks show a lock until the helper runs.
  - "Images" lists opened images and has an "Open image…" button.
  - Probe children appear under each item: partitions, LUKS, LVM and volumes.
- **Main list.**
  - Columns: Name, Size, Modified, Permissions, Owner.
  - The list is virtualized and sortable, with multi-select.
  - Double-clicking a folder enters it.
- **Path bar and toolbar.** A breadcrumb path bar. The toolbar has Back, Up, Refresh, "Copy to…" and Properties.
- **Status bar and job panel.** The status bar shows item count, selection and filesystem info. The job panel shows copy progress with Cancel.
- **Dialogs.** LUKS passphrase, copy options (conflict policy), copy report, properties, and About (licenses, credit to ext2read).
- **Keyboard and accessibility.** Arrow keys, Enter, Backspace, Ctrl+A, and Ctrl+C for "Copy to…". Focus is visible, and the tree and list carry ARIA roles for screen readers.
- **Language.** English and Turkish strings, chosen from the OS locale, with a manual override stored locally. Technical error details stay in English beneath a localized summary.

## 6. Data flow

### 6.1 Probe chain

```
Disk or image
 └ virtual disk? (VHDX / VMDK / QCOW2) → plain block device
    └ partition table (GPT; MBR with logical partitions; none = one volume)
       └ each region, identified by signature:
          LUKS                         → locked node → passphrase → decrypted device → identify again
          LVM2 PV                      → collected from all sources → VGs assembled → each LV identified
          ext2/3/4, Btrfs, XFS         → browsable volume
          NTFS, FAT, exFAT, ReFS,
          BitLocker                    → "Windows can open this"
          swap, f2fs, ZFS, mdraid,
          ReiserFS, JFS, APFS, HFS+    → "Not supported yet: <name>"
          anything else                → "Unrecognized"
```

- Multi-device sets (LVM volume groups; Btrfs filesystems by fsid) are gathered across all opened sources.
- An incomplete set is shown as "missing k of n devices". Logical volumes whose segments are all present still open.
- LVM segment types other than linear and striped (thin, RAID, mirror, snapshot) are shown as "Not supported yet: <type>".

### 6.2 Copy-out pipeline

- **Destination.** The user picks a destination folder.
- **Conflicts.** Default policy: **keep both**, so the copy becomes `name (2).ext`. Skip and overwrite can be chosen per job.
- **Name sanitization**, in order:
  1. Bytes that are not valid UTF-8 become U+FFFD.
  2. `< > : " / \ | ? *` and U+0000–U+001F become `_`.
  3. Trailing dots and spaces become `_`.
  4. Reserved device names get a `_` prefix. Matching is case-insensitive, with or without an extension: CON, PRN, AUX, NUL, COM1–COM9, COM¹–COM³, LPT1–LPT9, LPT¹–LPT³, CONIN$, CONOUT$.
  5. A component longer than 255 UTF-16 code units is shortened, keeping its extension.
  6. Names that collide after sanitization or case-folding get ` (2)`, ` (3)`, and so on.
  7. Files are created with `create_new`, so nothing is overwritten silently, unless the policy is "overwrite".
- **Timestamps and permissions.** The modification time is preserved with `File::set_modified`. Unix permissions and ownership are not mapped.
- **Special files.**
  - Symlinks, device nodes, FIFOs and sockets are not created on Windows. They are listed in the report, with link targets. Creating symlinks on Windows needs admin rights or Developer Mode.
  - Hard links are copied as independent files.
- **Streaming and cancel.** Data streams in 1 MiB chunks. Cancelling deletes the partly written file.
- **Report.** Lists files copied, renamed (old → new), skipped and failed, each with a reason. It can be saved as text.

### 6.3 Error handling

- **Corrupt data never crashes the app.**
  - In `tuxread-core`, Clippy denies `unwrap_used`, `expect_used`, `panic` and `indexing_slicing` outside tests.
  - Tree walks have depth limits and cycle detection.
  - Counts read from disk are checked against the device size before any allocation.
  - An unreadable directory becomes an error row; the rest of the app keeps working.
- **Read errors.** Bad sectors or an unplugged USB disk fail the affected file only, and the copy continues. A disconnected source is marked as such.
- **Uncleanly unmounted filesystems.**
  - ext4: the journal is replayed in memory by `ext4-view`; nothing is written to disk.
  - XFS: a dirty log shows a warning banner: "recent changes may be missing".
  - Btrfs: always consistent, because it is copy-on-write.
- **LUKS passphrases.** Zeroized after use and never logged. A wrong passphrase asks again.
- **Logging.** `tauri-plugin-log` writes rotating logs to the app's local log directory (`%LOCALAPPDATA%\io.github.poweruserz.tuxread\logs`). Logs never leave the machine.
- **Diagnostics.** "Copy diagnostics" produces a summary without file names: disk models and sizes, partition and filesystem types, feature flags and errors. It is meant to be pasted into a GitHub issue.

## 7. Testing

### 7.1 Corpus of real images

- **Generation.** `scripts/make-corpus.sh` runs on Linux: the developer's WSL Ubuntu, or a GitHub Actions Ubuntu runner. It uses e2fsprogs, btrfs-progs, xfsprogs, cryptsetup, lvm2, qemu-utils, util-linux and dmsetup.
- **Fixture tree.** Every image gets the same tree:
  - empty, small, 1 MiB random and 16 MiB random files (fixed seed);
  - a sparse file;
  - a fragmented file (written interleaved, to force multi-level extent trees);
  - a directory nested 32 levels deep;
  - a directory with 10,000 entries (forces ext4 htree and XFS leaf/node formats);
  - names in Turkish, Japanese and emoji;
  - names containing non-UTF-8 bytes;
  - Windows-hostile names: `CON`, `a:b`, `trailing.`, `File` next to `file`;
  - symlinks to a file and to a directory, a hard link and a FIFO;
  - modes 0600, 0755 and 04755, and owners 0:0 and 1000:1000;
  - fixed timestamps, including one before 1970 and one after 2038.
- **Ground truth.** Expected results come from the Linux kernel. Each image is mounted and walked by `scripts/manifest.py` (Python standard library, byte paths). It writes `manifest.json` with, per entry: base64 path, kind, size, SHA-256, mtime in nanoseconds, mode, uid, gid and base64 link target.
- **Variants, by milestone:**

| Milestone | Variants |
|---|---|
| v0.1 | `match`: ext2 (1 KiB blocks); ext3; ext4 default (64bit, metadata_csum, extents, flex_bg); ext4 without 64bit; ext4 with an unreplayed journal (crash simulated with dm-flakey, as xfstests does). `unsupported`: ext4 with inline_data; ext4 with casefold. ext4 with bigalloc: `match` if `ext4-view` reads it correctly, otherwise `unsupported`; this is checked once during planning and then fixed in the corpus definition. Containers, all `match`: GPT 512, GPT 4096, MBR with logical partitions, bare filesystem. |
| v0.2 | LUKS1 aes-xts-plain64 / PBKDF2; LUKS1 aes-cbc-essiv:sha256; LUKS2 default (Argon2id); LUKS2 PBKDF2-SHA512; LUKS2 with 4096-byte encryption sectors. LVM linear on one PV; linear spanning two PVs; striped over two PVs; Ubuntu layout (LUKS → LVM → ext4); thin LV (must report "not supported"). |
| v0.3 | Btrfs single; DUP metadata; RAID1 over two devices; RAID0; zlib, lzo and zstd compression; subvolumes and a snapshot; inline files; crc32c, xxhash, sha256 and blake2 checksums. |
| v0.4 | XFS v5 default (bigtime, reflink, ftype); XFS v4 if the installed mkfs.xfs still creates it; directories in every format; files with B-tree extent maps; a dirty log via dm-flakey. |
| v0.5 | QCOW2 (plain, zlib, zstd, with a backing file); VMDK (monolithicSparse, streamOptimized, twoGbMaxExtentSparse); VHDX (fixed, dynamic, differencing). |

- **Storage.** Images are not committed to git. They are regenerated from the script and cached in CI.

### 7.2 Corpus gate test

Every corpus variant declares one expected outcome:

- **`match`:** the engine's walk must equal the manifest byte for byte: names, kinds, sizes, content hashes, timestamps, modes, owners and link targets.
- **`unsupported`:** the engine must refuse the volume and name the reason.

A silent misread is always a failure. `crates/core/tests/corpus.rs` opens every image through the engine and checks each variant against its declared outcome. The test is skipped when the `TUXREAD_CORPUS` environment variable is not set. This test is the adoption gate of §5.4.

### 7.3 Fuzzing

- `cargo-fuzz` targets: probe chain, partition tables, ext adapter, LUKS header, LVM metadata, Btrfs and XFS walks, QCOW2/VHDX/VMDK headers.
- Seeds come from the corpus.
- Goals: no panics, no hangs, memory stays within the limit.
- `ext4-view` has no fuzzing of its own, so our ext target exercises it; bugs found go upstream.
- Fuzzing runs nightly on a Linux CI runner.

### 7.4 Windows-specific tests

- **Alignment.** Unit tests run against a fake device that rejects any read not aligned to 512 or 4096 bytes, so the 4K-sector bug of ext2read cannot return.
- **Helper.** Tests cover the pipe protocol and the identity checks. A test-only build flag lets the helper serve an image file instead of a physical disk.
- **Real disk.** In CI, a VHDX is attached as a physical disk with `diskpart` and read end to end through `WinDisk` and the helper.
- **Copy-out.** The fixture tree is copied to NTFS. The test checks sanitization, collision handling, timestamps and paths longer than 260 characters.

### 7.5 UI

There are no automated UI tests in v1. Before each release this manual checklist is run:

- open an image
- unlock LUKS (from v0.2)
- browse a 10,000-entry directory
- copy with a conflict
- cancel a copy
- open a physical disk through the helper, including declining UAC
- switch language

Vitest or WebDriver tests are added when the UI gains logic worth testing.

### 7.6 CI (GitHub Actions)

- **Linux:** generate the corpus (cached by script hash), then upload it as an artifact.
- **Windows x64:** `cargo fmt --check`, `cargo clippy` (strict lints for `tuxread-core`), `cargo test` with the corpus, frontend type-check and build, and `cargo deny check` (licenses and advisories).
- **Nightly:** fuzzing.
- **Release (on tags):** NSIS installer and portable zip for x64 and ARM64, plus SHA-256 checksums.

## 8. Roadmap

Each milestone is a usable GitHub release.

| Release | Scope |
|---|---|
| **v0.1** | Engine core (`BlockDev`, `CachedDev`, probe, GPT/MBR); ext2/3/4; raw images; disk listing and the elevated helper. UI: browse, copy with sanitization and report, properties, diagnostics, English and Turkish. CLI. Corpus and CI for the v0.1 variants. Unsigned NSIS installer and portable zip, x64 and ARM64. LUKS, LVM, Btrfs and XFS volumes are detected and shown as "supported in a later version". |
| **v0.2** | LUKS1/LUKS2 and LVM2 (Ubuntu "encrypt disk" installs, RHEL-family layouts, multi-disk volume groups). |
| **v0.3** | Btrfs: subvolumes, compression, multi-device. RAID5/6 and degraded arrays only if they pass the corpus; otherwise reported as not supported. |
| **v0.4** | XFS. |
| **v0.5** | QCOW2, VHDX, VMDK. |
| **v1.0** | Hardening: all fuzz targets clean, full corpus green, documentation, signed binaries, winget package. |

### v0.1 definition of done

- **Reading.**
  - ext2/3/4 volumes from raw images and physical disks (through the helper) read correctly inside GPT, MBR with logical partitions, and bare layouts. Every v0.1 corpus variant passes the gate test.
  - Every other volume type in §6.1 is labelled, never misread.
- **UI.**
  - Browse, properties, "Copy to…" with the §6.2 rules and report, cancel, and diagnostics all work.
  - English and Turkish.
- **CLI.**
  - `tuxread-cli probe <image|disk:N>`, `tuxread-cli ls <source> <volume> <path>` and `tuxread-cli cp <source> <volume> <path> <dest>` work. Disk sources need an elevated console.
- **Quality.** Fuzz targets exist for the probe chain, the partition tables and the ext adapter, and they run clean for one hour each. CI is green.
- **Release.** Release artifacts are built by CI.

### Candidates after v1.0 (not commitments)

- Drag-and-drop to Explorer.
- Read-only drive letters through WinFsp (after WinFsp 2.2 is stable, for its CVE fixes).
- Built-in preview.
- f2fs and mdraid.
- Upstream `ext4-view` work for inline_data, meta_bg and casefold.
- Microsoft Store distribution as MSIX (Microsoft signs Store packages; the helper would need the `allowElevation` capability).

## 9. Distribution and signing

- **Artifacts.** Each GitHub Release has, for x64 and ARM64:
  - an NSIS installer that installs per user without admin. It embeds the WebView2 bootstrapper, which downloads and installs the runtime if it is missing;
  - a portable zip.

  `TuxRead.exe` and `tuxread-cli.exe` both carry ProductName "TuxRead" and the same version in their Windows version resources. Every artifact gets a SHA-256 checksum.
- **WebView2 check.** WebView2 ships with Windows 11 and most Windows 10 installs, but not all LTSC editions. If the runtime is missing at startup, the app shows a native message box with the download address and exits.
- **Updates.** There is no auto-updater, since the app makes no network requests. Updates come from GitHub Releases, and from v1.0 also through `winget upgrade`.
- **Code signing with SignPath Foundation.**
  - v0.1 ships unsigned, because SignPath requires a project to be released before it applies.
  - **Requirements we meet by design:** an OSI license; no proprietary components; builds by GitHub Actions; product name and version set in every binary.
  - **Requirements on the maintainer:** multi-factor authentication on GitHub and SignPath, and manual approval of every release signing.
  - Windows will show **"SignPath Foundation"** as the publisher.
  - SmartScreen reputation still builds up through downloads. Signed releases share it across versions, whereas unsigned releases start from zero each time.
- **Code signing policy section.** It goes in the README and on every release page, with this text:

  > **Code signing policy**
  > Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).
  > Committers, reviewers and approvers: the repository owner (the project currently has a single maintainer).
  > Privacy policy: This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.

## 10. Risks

- **ext4-view gaps.**
  - Volumes using inline_data, meta_bg or casefold can't be read in v1.
  - Upstream contributions need Google's CLA, and the maintainer has rejected bulk AI-generated pull requests. Our contributions will be small and human-reviewed; a fork remains possible.
- **ext4-view performance.** It reads one block per call and its seek is O(n). Copies read sequentially, and `CachedDev` batches device reads. If large-file throughput is still too low, we improve it upstream or in a fork.
- **Btrfs scope.** RAID5/6 and degraded arrays depend on the v0.3 evaluation.
- **WebView2 availability** on some Windows 10 LTSC machines (mitigated by the bootstrapper and the startup check).
- **SmartScreen warnings** until signing is in place, and reduced warnings until reputation grows.
- **ARM64.** The NSIS installer itself runs under x86 emulation; the app is native.
- **Helper misuse by same-user malware** is limited by the §5.7 checks, not eliminated, because UAC is not a security boundary.

## 11. Credits

TuxRead is inspired by ext2read by Manish Regmi. No ext2read code is reused, so its GPLv3 terms do not apply to TuxRead.
