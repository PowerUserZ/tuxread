# Assembles one architecture's release files (spec §9) from a release build: the NSIS installer,
# a portable zip, and their SHA-256 checksums in sha256sum format. Build first with
#   node scripts/third-party-licenses.mjs THIRD-PARTY-LICENSES.txt
#   npx tauri build --config src-tauri/tauri.release.json
#   cargo build --locked --release -p tuxread-cli
# Usage: scripts/package.ps1 -Arch x64|arm64 -Out <folder>
param(
    [Parameter(Mandatory = $true)][ValidateSet('x64', 'arm64')][string]$Arch,
    [Parameter(Mandatory = $true)][string]$Out
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$version = (Get-Content "$root\package.json" -Raw | ConvertFrom-Json).version
$built = "$root\target\release"
$name = "TuxRead-$version-$Arch"
New-Item -ItemType Directory -Force $Out | Out-Null
# .NET calls below resolve a relative path against the process's folder, not PowerShell's.
$Out = (Resolve-Path $Out).Path

Copy-Item "$built\bundle\nsis\TuxRead_${version}_$Arch-setup.exe" "$Out\$name-setup.exe"

$stage = Join-Path ([System.IO.Path]::GetTempPath()) "$name-portable"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory $stage | Out-Null
Copy-Item "$built\TuxRead.exe", "$built\tuxread-cli.exe", "$root\README.md", "$root\LICENSE-MIT", "$root\LICENSE-APACHE", "$root\THIRD-PARTY-LICENSES.txt" $stage
Compress-Archive -Path "$stage\*" -DestinationPath "$Out\$name-portable.zip" -Force
Remove-Item -Recurse -Force $stage

$sums = foreach ($file in "$name-setup.exe", "$name-portable.zip") {
    '{0}  {1}' -f (Get-FileHash "$Out\$file" -Algorithm SHA256).Hash.ToLowerInvariant(), $file
}
[System.IO.File]::WriteAllText("$Out\$name.sha256", ($sums -join "`n") + "`n")
Get-ChildItem $Out -Filter "$name*" | ForEach-Object { '{0,12:N0}  {1}' -f $_.Length, $_.Name }
