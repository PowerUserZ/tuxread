# Attaches a VHD read-only as a physical disk, without a drive letter, and prints its
# disk number. Needs administrator rights. Detach with: Dismount-DiskImage -ImagePath <vhd>
param([Parameter(Mandatory = $true)][string]$Path)
$ErrorActionPreference = 'Stop'
$full = (Resolve-Path $Path).Path
Mount-DiskImage -ImagePath $full -StorageType VHD -Access ReadOnly -NoDriveLetter | Out-Null
(Get-DiskImage -ImagePath $full | Get-Disk).Number
