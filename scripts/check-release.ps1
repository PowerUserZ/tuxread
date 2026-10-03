# Checks one architecture's release files made by scripts/package.ps1 (spec §9):
# - the checksums match;
# - the zip holds exactly the program, the CLI, the README and the license files;
# - both executables are built for the architecture and carry ProductName "TuxRead" and the
#   release's version, as the installer does;
# - the license file names the window's and the engine's components.
# With -Install it also installs per user without a prompt, checks the installed files, and
# uninstalls again (CI only: it changes the current user's programs).
# Usage: scripts/check-release.ps1 -Arch x64|arm64 -Dir <folder> [-Install]
param(
    [Parameter(Mandatory = $true)][ValidateSet('x64', 'arm64')][string]$Arch,
    [Parameter(Mandatory = $true)][string]$Dir,
    [switch]$Install
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$version = (Get-Content "$root\package.json" -Raw | ConvertFrom-Json).version
$name = "TuxRead-$version-$Arch"
$machine = @{ x64 = 0x8664; arm64 = 0xAA64 }[$Arch]
$failures = [System.Collections.Generic.List[string]]::new()
function Check([bool]$ok, [string]$what) { if ($ok) { "ok    $what" } else { "FAIL  $what"; $failures.Add($what) } }

# The machine type in a PE file's header.
function Machine([string]$path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    [BitConverter]::ToUInt16($bytes, $pe + 4)
}

function Release-Exe([string]$path, [string]$what) {
    $info = (Get-Item $path).VersionInfo
    Check ($info.ProductName -eq 'TuxRead') "$what has ProductName TuxRead (got '$($info.ProductName)')"
    Check ($info.ProductVersion -eq $version) "$what has version $version (got '$($info.ProductVersion)')"
}

$setup = "$Dir\$name-setup.exe"
$zip = "$Dir\$name-portable.zip"
foreach ($line in Get-Content "$Dir\$name.sha256") {
    $hash, $file = $line -split '  ', 2
    Check ((Get-FileHash "$Dir\$file" -Algorithm SHA256).Hash -eq $hash) "checksum of $file"
}
Release-Exe $setup 'the installer'

$unzipped = Join-Path ([System.IO.Path]::GetTempPath()) "$name-check"
Remove-Item -Recurse -Force $unzipped -ErrorAction SilentlyContinue
Expand-Archive $zip $unzipped
$files = (Get-ChildItem $unzipped | ForEach-Object Name | Sort-Object) -join ', '
Check ($files -eq 'LICENSE-APACHE, LICENSE-MIT, README.md, THIRD-PARTY-LICENSES.txt, tuxread-cli.exe, TuxRead.exe') "the zip holds the program, the CLI, the README and the licenses (got $files)"
foreach ($exe in 'TuxRead.exe', 'tuxread-cli.exe') {
    Release-Exe "$unzipped\$exe" $exe
    Check ((Machine "$unzipped\$exe") -eq $machine) "$exe is built for $Arch"
}
$licenses = Get-Content "$unzipped\THIRD-PARTY-LICENSES.txt" -Raw
foreach ($component in 'react 19', 'ext4-view', 'tauri', 'gptman', 'mbrman') {
    Check ($licenses -match [regex]::Escape($component)) "THIRD-PARTY-LICENSES.txt covers $component"
}
Remove-Item -Recurse -Force $unzipped

if ($Install) {
    $installed = "$env:LOCALAPPDATA\TuxRead"
    $process = Start-Process $setup -ArgumentList '/S' -PassThru -Wait
    Check ($process.ExitCode -eq 0) "the installer runs silently, per user (exit $($process.ExitCode))"
    foreach ($file in 'TuxRead.exe', 'THIRD-PARTY-LICENSES.txt', 'LICENSE-MIT', 'LICENSE-APACHE') {
        Check (Test-Path "$installed\$file") "the installation has $file"
    }
    $process = Start-Process "$installed\uninstall.exe" -ArgumentList '/S' -PassThru -Wait
    Check ($process.ExitCode -eq 0) "the uninstaller runs silently (exit $($process.ExitCode))"
    # The uninstaller copies itself away and finishes in the background.
    for ($i = 0; $i -lt 50 -and (Test-Path "$installed\TuxRead.exe"); $i++) { Start-Sleep -Milliseconds 200 }
    Check (-not (Test-Path "$installed\TuxRead.exe")) 'the uninstaller removes TuxRead.exe'
}

if ($failures.Count -gt 0) { throw "$($failures.Count) check(s) failed" }
'all release checks passed'
