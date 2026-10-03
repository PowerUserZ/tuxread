# Release checklist

Run this list for every release, in order. CI builds and checks the files; the steps here
cover what CI cannot: a UAC prompt, a standard user, Narrator, a real USB disk.

## 1. Version

1. Set the new version in `Cargo.toml` (`[workspace.package] version`) and in `package.json`.
   The version lives only in these two files, and a test keeps them equal.
2. Run `cargo check --workspace` and `npm install --package-lock-only`, so `Cargo.lock` and
   `package-lock.json` carry the new version.
3. Commit ("Release X.Y.Z") and merge to `main`.

## 2. Builds

1. CI is green on that commit of `main`.
2. **Fuzzing:** run the Fuzz workflow by hand (Actions, Fuzz, "Run workflow") with
   `seconds = 3600`. All three targets finish clean, an hour each. A crash leaves its input as
   the run's artifact: fix it, and add the input to the target's seed corpus.
3. **Dry run:** run the Release workflow by hand on `main`. It tests, builds, packages and
   checks both architectures and keeps the files as the run's artifacts, `release-x64` and
   `release-arm64`. Download them for the manual checks.

## 3. Manual checks

Use the dry run's files. Unless a step says otherwise, use the x64 installer on Windows 11 with
UAC at its default setting, so that it prompts.

1. **Installer as a standard user.** Run `TuxRead-X.Y.Z-x64-setup.exe` signed in as a standard
   user. It shows no UAC prompt and installs to `%LOCALAPPDATA%\TuxRead`, with a Start menu
   entry. "Installed apps" lists TuxRead with the new version.
2. **Upgrade.** With the previous release installed and running, run the new installer. It
   closes the running TuxRead (asking first) and replaces it, and About shows the new version.
3. **SmartScreen.** Downloaded with a browser, the unsigned installer may show "Windows
   protected your PC". "More info" then "Run anyway" starts it, as the README says.
4. **UAC declined.** Select a locked disk and answer No. The banner says "Administrator approval
   was declined, so the disk stays locked.", the disk keeps its lock, and selecting it again asks
   again.
5. **The UAC prompt's owner.** The prompt opens in front of the TuxRead window and blinks its
   taskbar button, not behind other windows.
6. **A standard user's disk.** Signed in as a standard user, open a disk with an administrator's
   credentials. The disk opens, and copied files belong to the standard user.
7. **Administrator protection.** With Windows 11's Administrator protection on, opening a disk
   still works, and copied files land in the signed-in user's folders with that user as owner.
8. **A real USB disk.** Plug in a USB disk or stick with an ext4 partition. It appears in the
   list; open it, browse, and copy a folder. Unplugged while open, its reads fail with a message
   instead of hanging, and the rest of the window keeps working.
9. **Keyboard only.**
   - Tab reaches the tree, the list, its column headers and the toolbar, and comes back to the
     chosen volume in the tree. The arrow keys move in the tree and the list.
   - Enter opens a volume or folder, and Enter on a column header sorts by it. Ctrl+A then
     Ctrl+C opens the copy dialog. Enter in the dialog copies, and Escape closes it.
10. **Narrator.** Narrator reads a tree item's name and size and a row's name, size and date, and
    says when a row is selected.
11. **"Choose…" and "Save as text…".** Both open the native dialogs. The chosen folder fills the
    field, and the saved report opens in Notepad with one line per item.
12. **Themes.** Switching Windows between light and dark restyles the open window.
13. **Run from a mapped network drive.** Copy the portable build to a network share mapped to a
    drive letter, start it from that letter, and open a disk.
14. **The disk smoke test.** Attach the corpus image `mbr-logical.img` as a disk
    (`scripts/attach-vhd.ps1`, elevated) and run
    `node scripts/smoke/run.mjs <TuxRead.exe> disk` with `TUXREAD_SMOKE_DISK` set to its number.
    CI runs the other scenarios; this one needs a UAC prompt, which CI runners do not show.
15. **Hidden from Windows.** Give an exFAT partition on a test disk (a VHD will do) the type
    "Linux filesystem": Windows stops showing it. TuxRead marks it "Hidden from Windows: how to
    show it". The dialog's commands, pasted into an administrator PowerShell, make it a drive
    again with its files; run a second time, they say nothing was found and change nothing.
16. **ARM64.** If an ARM64 PC is at hand, install `TuxRead-X.Y.Z-arm64-setup.exe` there, open an
    image and copy from it. CI installs the ARM64 build and opens its window, but browses and
    copies only with the x64 build.

## 4. Publish

1. Tag the commit and push the tag:

   ```
   git tag -a vX.Y.Z -m "TuxRead X.Y.Z"
   git push origin vX.Y.Z
   ```

   The Release workflow refuses a tag that is not `v` plus the version in `package.json`. It
   builds both architectures again and makes a **draft** release with the six files and the
   notes from `.github/release-notes.md`.
2. Open the draft, add what changed above the notes, check that the six files are there, and
   publish it.
3. **After the first release:** apply to the [SignPath Foundation](https://signpath.org) for
   free code signing (spec §9). Once accepted, signing becomes a step of the Release workflow
   that the maintainer approves for each release.
