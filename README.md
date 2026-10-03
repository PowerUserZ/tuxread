# TuxRead

TuxRead lets you browse and copy files from Linux disks and disk images on Windows. It only
reads: it never writes to a disk or an image. It continues the idea of
[ext2read](https://github.com/mregmi/ext2read) by Manish Regmi.

Version 0.1 reads ext2, ext3 and ext4, inside GPT or MBR partition tables (logical partitions
included) or on a disk with no table. It recognizes LUKS, LVM, Btrfs and XFS and labels them
"supported in a later version", and it labels Windows filesystems as such instead of misreading
them.

## Download

Get TuxRead from [GitHub Releases](https://github.com/PowerUserZ/tuxread/releases). Each release
has, for 64-bit Intel/AMD (x64) and ARM64 PCs:

- `TuxRead-<version>-<arch>-setup.exe`: an installer for your account only, with no
  administrator rights needed. It installs the Microsoft Edge WebView2 Runtime if it is missing.
- `TuxRead-<version>-<arch>-portable.zip`: `TuxRead.exe` and the command-line `tuxread-cli.exe`,
  to run from any folder.
- `TuxRead-<version>-<arch>.sha256`: the files' SHA-256 checksums.

Releases are not code-signed yet (see the code signing policy below), so Windows SmartScreen may
warn the first time. Choose "More info", then "Run anyway".

## Using it

- **Disk images** (raw `.img` files): choose "Open image…".
- **Disks** are listed with a lock. Select one to read it: Windows asks once per session for
  administrator approval. Only a small helper runs elevated; it reads sectors and nothing else.
- Select files and folders, then "Copy to…" (or Ctrl+C) to copy them to a folder on Windows.
  A report lists what was copied, renamed, skipped (such as symbolic links) or failed.
- "Copy diagnostics" puts a summary of your disks and what TuxRead found on them on the
  clipboard, without file names or paths, for a bug report.

The window is in English and Turkish; it follows the Windows language, and About lets you choose.

## Command line

```
tuxread-cli disks                                 list the disks
tuxread-cli probe <source>                        what is on a disk or image
tuxread-cli ls <source> <volume> [path]           list a folder
tuxread-cli cp <source> <volume> <path> <folder>  copy out
```

`<source>` is an image file or `disk:N`, and `<volume>` is the `#number` that `probe` prints.

## Privacy

TuxRead makes no network requests. It keeps a log in
`%LOCALAPPDATA%\io.github.poweruserz.tuxread\logs` that never leaves your computer.

## Code signing policy

> **Code signing policy**
> Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).
> Committers, reviewers and approvers: the repository owner (the project currently has a single maintainer).
> Privacy policy: This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.

Version 0.1 ships unsigned: SignPath Foundation signs projects once they have been released.

## Building from source

You need Windows 10 or 11 with the WebView2 Runtime, Rust (stable, MSVC), and Node.js 24.

```
npm ci
npx tauri build --no-bundle            # target\release\TuxRead.exe
cargo build --release -p tuxread-cli   # target\release\tuxread-cli.exe
cargo test --workspace
npm test
node scripts/smoke/run.mjs target\release\TuxRead.exe
```

The design is in [docs/superpowers/specs](docs/superpowers/specs), and the plans each phase
was built from are in [docs/superpowers/plans](docs/superpowers/plans).
[docs/release-checklist.md](docs/release-checklist.md) lists what is checked before a release.

## License

TuxRead is free software under the [MIT](LICENSE-MIT) or the [Apache-2.0](LICENSE-APACHE)
license, at your option. The components it is built with, among them
[ext4-view](https://github.com/nicholasbishop/ext4-view-rs), gptman, mbrman, Tauri and React,
are listed with their licenses in `THIRD-PARTY-LICENSES.txt`, next to the program.
