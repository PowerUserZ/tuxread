---
name: run-tuxread
description: Build, run and drive TuxRead (the Tauri window TuxRead.exe and tuxread-cli). Use when asked to start or launch TuxRead, open a disk image in it, click or type in its window, take a screenshot of its UI, check a UI change in the real app, run the CLI, or run its tests and smoke test.
---

TuxRead is a Windows-only, read-only browser for Linux filesystems: a Tauri v2 window (`TuxRead.exe`, React in WebView2) and a CLI (`tuxread-cli.exe`) over the Rust engine. Drive the window with `.claude/skills/run-tuxread/driver.mjs`. It launches the built exe with a throwaway WebView2 profile, runs commands from stdin over the DevTools protocol, and takes screenshots. Paths are relative to the repository root.

## Prerequisites

Windows 10/11 with the WebView2 Runtime, Rust stable (MSVC), Node.js 24. Nothing else was installed for this skill. The corpus images (for the 10,000-entry and disk checks) live in WSL at `\\wsl.localhost\Ubuntu-26.04\var\tmp\tuxcorpus`, made by `scripts/make-corpus.sh`. The small test image `crates/core/tests/data/tiny-ext4.img` is in the repo.

## Build

```powershell
npm ci
npx tauri build --no-bundle                     # target\release\TuxRead.exe (about 50 s warm)
cargo build --locked --release -p tuxread-cli   # target\release\tuxread-cli.exe
```

## Run the window (agent path)

Pipe a command script to the driver. It prints `> command` and each result, stops at the first `ERROR:` (exit 1), and closes the app at the end of the input. PowerShell:

```powershell
@"
lang en
open crates/core/tests/data/tiny-ext4.img
click .item.volume
wait document.querySelectorAll(".body .row:not(.head)").length === 4
rows
text .statusbar
ss $env:TEMP/tuxread-shots/browse.png
"@ | node .claude/skills/run-tuxread/driver.mjs
```

Then look at the screenshot (Read the PNG). Each run starts a fresh app, so every script opens its image again.

Git Bash. The heredoc is unquoted so that `$LOCALAPPDATA` expands, which also expands any `$` or backtick in a JS line; quote it (`<<'EOF'`) for such scripts. Don't use `$TEMP` there: Git Bash maps it to `/tmp`, which Node reads as `C:\tmp`.

```bash
node .claude/skills/run-tuxread/driver.mjs <<EOF
lang en
open crates/core/tests/data/tiny-ext4.img
click .item.volume
wait document.querySelectorAll(".body .row:not(.head)").length === 4
rows
ss $LOCALAPPDATA/Temp/tuxread-shots/browse.png
EOF
```

| command | what it does |
|---|---|
| `lang en\|tr` | language, then reload (always set it: the default follows Windows) |
| `open <image>` | opens an image as "Open image…" would (the native dialog can't be driven) |
| `click <css>` / `focus <css>` | first match |
| `key <Key> [mods]` | DOM key name; mods 1 Alt, 2 Ctrl, 8 Shift (`key a 2` = Ctrl+A) |
| `type <text>` | into the focused field |
| `wait <js>` | until truthy, 10 s |
| `eval <js>` | prints JSON |
| `text [css]` / `rows` | innerText; the file list's rendered rows |
| `ss <file.png>` | screenshot of the window; folders are created |
| `sleep <ms>` | |

A copy through the dialog, from the keyboard, as verified:

```powershell
$dest = Join-Path $env:TEMP 'tuxread-copy-out'; Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue; New-Item -ItemType Directory $dest | Out-Null
@"
lang en
open crates/core/tests/data/tiny-ext4.img
click .item.volume
wait document.querySelectorAll(".body .row:not(.head)").length === 4
focus .filelist
key a 2
key c 2
wait document.querySelector("dialog[open] input[name=dest]") !== null
focus dialog[open] input[name=dest]
type $dest
key Enter
wait [...document.querySelectorAll(".job")].at(-1)?.querySelector(".bar") === null
text .job .job-line
"@ | node .claude/skills/run-tuxread/driver.mjs
# -> 4 copied, 0 renamed, 1 skipped, 0 failed   (the skip is the symlink; into a non-empty folder the names get " (2)")
```

Useful selectors: `.item.volume` (tree volumes), `.source-head` (disks and images), `.filelist` (the grid), `.body .row:not(.head)`, `.pathbar button`, `.statusbar`, `dialog[open]`, `.job`.

## Real disks (the elevated helper)

Selecting a locked disk starts the elevated disk helper with a UAC prompt. To drive that path without a USB disk, attach a corpus image as a disk. This machine approves UAC for administrators without asking:

```powershell
$work = Join-Path $env:TEMP 'tuxread-disk'; New-Item -ItemType Directory -Force $work | Out-Null
python scripts/make-vhd.py '\\wsl.localhost\Ubuntu-26.04\var\tmp\tuxcorpus\mbr-logical.img' "$work\mbr-logical.vhd"
Start-Process pwsh -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile', '-Command', "& '$PWD\scripts\attach-vhd.ps1' '$work\mbr-logical.vhd' *> '$work\disk.txt'"
$env:TUXREAD_SMOKE_DISK = (Get-Content "$work\disk.txt").Trim()
node scripts/smoke/run.mjs target\release\TuxRead.exe disk
Start-Process pwsh -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile', '-Command', "Dismount-DiskImage -ImagePath '$work\mbr-logical.vhd'"
```

## Run the CLI

```powershell
target\release\tuxread-cli.exe probe crates/core/tests/data/tiny-ext4.img     # "#1 ext4 [512.0 KiB]"
target\release\tuxread-cli.exe ls crates/core/tests/data/tiny-ext4.img 1 /
target\release\tuxread-cli.exe cp crates/core/tests/data/tiny-ext4.img 1 /dir $env:TEMP\tuxread-cli-out
target\release\tuxread-cli.exe disks
```

`disk:N` sources need an elevated console, or they ask for UAC. `cp` exits non-zero when any item fails.

## Run (human path)

Start `target\release\TuxRead.exe`: the same build the driver launches, with your own profile and no DevTools port, so nothing can script it.

## Test

```powershell
$env:TUXREAD_CORPUS = '\\wsl.localhost\Ubuntu-26.04\var\tmp\tuxcorpus'; cargo test --locked --workspace   # about 70 s; the corpus gate test is the slow one
npm test                                                                                                 # Vitest, src/logic.test.ts
$env:TUXREAD_SMOKE_BIG = '\\wsl.localhost\Ubuntu-26.04\var\tmp\tuxcorpus\ext4.img'; node scripts/smoke/run.mjs target\release\TuxRead.exe
```

The last line runs the scripted UI scenarios (`harden`, `browse`, `copy`, `disk`): `all passed`, with `disk` skipped unless `TUXREAD_SMOKE_DISK` is set. Without `TUXREAD_CORPUS` the corpus tests skip.

## Gotchas

- **An elevated console gets no DevTools port.** WebView2 ignores `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` in an elevated process, so the driver and the smoke test fail with "no DevTools port" / "the window's DevTools port did not open". Run them unelevated. If you must run elevated (GitHub's Windows runners always are), build the port into the window's config. `WEBVIEW2_USER_DATA_FOLDER` still works elevated:
  ```powershell
  $window = (Get-Content src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).app.windows[0]
  $window | Add-Member additionalBrowserArgs '--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port=0'
  @{ app = @{ windows = @($window) } } | ConvertTo-Json -Depth 5 | Set-Content "$env:TEMP\smoke.json"
  npx tauri build --no-bundle --config "$env:TEMP\smoke.json"
  ```
  Rebuild without `--config` afterwards; never ship that build.
- **The DevTools target exists before the app's page does.** Evaluating too early fails with `SecurityError: Failed to read the 'localStorage' property … Access is denied`. The driver waits for `#root` to render first; hand-written CDP code must do the same.
- **Native dialogs ("Open image…", "Choose…", "Save as text…") are outside the webview.** CDP cannot reach them. `open` calls the app's own `open_image` command instead and reloads. For the copy dialog, type the folder into `input[name=dest]`.
- **Script in the window may call only TuxRead's own commands.** Core and plugin commands are refused by design; the `harden` scenario checks that. `window.__TAURI_INTERNALS__.invoke("open_image", { path })` works, as `open` shows.
- **Disks appear in the tree for real.** The list shows this machine's disks, with their models. Clicking one starts the elevated helper, so stick to images unless you mean it.
- **Claude Code's Bash tool turns `\\` into `\`**, even inside single quotes and heredocs. So `'\\wsl.localhost\…'` arrives as `\wsl.localhost\…`.
  - Set `TUXREAD_CORPUS` and `TUXREAD_SMOKE_BIG` from PowerShell.
  - In Bash command scripts, write paths with forward slashes; the driver resolves them.

## Troubleshooting

- **`no DevTools port: an elevated console …`**: see the first gotcha.
- **`ERROR: TypeError: Cannot read properties of null (reading 'click')`**: the selector matched nothing; check it with `eval document.querySelector("…") !== null`.
- **`… TuxRead.exe does not exist`**: build first (`npx tauri build --no-bundle`), or pass the exe path as the driver's first argument.
