#!/usr/bin/env bash
# Builds the TuxRead test corpus (spec §7.1): real images made by Linux tools, each
# with <name>.expect.json describing what the Linux kernel reads from every volume.
#
# Usage, as root on Linux or WSL:  scripts/make-corpus.sh <output-dir>
set -euo pipefail
shopt -s inherit_errexit

OUT=$(realpath -m "${1:?usage: make-corpus.sh <output-dir>}")
HERE=$(dirname "$(realpath "$0")")
WORK=$(mktemp -d)
MNT=$WORK/mnt
LOOPS=()
mkdir -p "$OUT" "$MNT"

cleanup() {
  umount -q "$MNT" 2>/dev/null || true
  for l in "${LOOPS[@]}"; do losetup -d "$l" 2>/dev/null || true; done
  rm -rf "$WORK"
}
trap cleanup EXIT

# mkfs options shared by every ext variant: enough inodes for the 10,000-file folder.
EXT_COMMON=(-q -F -N 20000)

new_image() { rm -f "$1"; truncate -s "$2" "$1"; }

attach() { # attach <image> [losetup options...] -> prints the loop device
  local dev
  dev=$(losetup -f --show -P "${@:2}" "$1")
  LOOPS+=("$dev")
  udevadm settle 2>/dev/null || sleep 1
  echo "$dev"
}

populate() { mount "$1" "$MNT"; python3 "$HERE/fixture.py" "$MNT"; umount "$MNT"; }

# What the kernel reads, after a fresh mount (so nothing comes from caches).
volume_manifest() { mount -o ro "$1" "$MNT"; python3 "$HERE/manifest.py" "$MNT"; umount "$MNT"; }

write_expect() { # write_expect <name> <volume-json>...
  local name=$1; shift
  local IFS=,
  printf '{"volumes":[%s]}\n' "$*" > "$OUT/$name.expect.json"
  echo "built $name"
}

unsupported() { printf '{"outcome":"unsupported","reason":"%s"}' "$1"; }

bare_ext() { # bare_ext <name> <mkfs.extN> <options...>
  local name=$1 mkfs=$2; shift 2
  local img=$OUT/$name.img
  new_image "$img" 128M
  "$mkfs" "${EXT_COMMON[@]}" "$@" "$img"
  populate "$img"
  local manifest
  manifest=$(volume_manifest "$img")
  write_expect "$name" "$manifest"
}

# ---- v0.1 variants -----------------------------------------------------------

bare_ext ext2-1k mkfs.ext2 -b 1024 -I 128
bare_ext ext3 mkfs.ext3 -I 256
bare_ext ext4 mkfs.ext4 -I 256
bare_ext ext4-no64bit mkfs.ext4 -I 256 -O ^64bit
bare_ext ext4-bigalloc mkfs.ext4 -I 256 -O bigalloc -C 16384

# Refused from the superblock alone, so no files are needed (some kernels, e.g. WSL's,
# cannot even mount casefold).
for feature in inline_data casefold; do
  img=$OUT/ext4-$feature.img
  new_image "$img" 128M
  mkfs.ext4 "${EXT_COMMON[@]}" -O "$feature" "$img"
  write_expect "ext4-$feature" "$(unsupported "$feature")"
done

# An ext4 image whose journal still holds committed transactions: copied while mounted,
# after a sync. The expected result is what the kernel reads after replaying a copy.
img=$OUT/ext4-dirty-journal.img
new_image "$WORK/live.img" 128M
mkfs.ext4 "${EXT_COMMON[@]}" -I 256 "$WORK/live.img"
mount "$WORK/live.img" "$MNT"
python3 "$HERE/fixture.py" "$MNT"
sync
cp --sparse=always "$WORK/live.img" "$img"
umount "$MNT"
dumpe2fs -h "$img" 2>/dev/null | grep -q needs_recovery || { echo "dirty-journal image is clean" >&2; exit 1; }
cp --sparse=always "$img" "$WORK/replay.img"
mount "$WORK/replay.img" "$MNT" # a read-write mount replays the journal
umount "$MNT"
manifest=$(volume_manifest "$WORK/replay.img")
write_expect ext4-dirty-journal "$manifest"

# GPT with 512- and 4096-byte sectors, one ext4 partition each.
for sector in 512 4096; do
  img=$OUT/gpt-$sector.img
  new_image "$img" 160M
  loop=$(attach "$img" --sector-size "$sector")
  printf 'label: gpt\nstart=1MiB, type=linux\n' | sfdisk -q "$loop"
  blockdev --rereadpt "$loop" 2>/dev/null || true
  udevadm settle 2>/dev/null || sleep 1
  mkfs.ext4 "${EXT_COMMON[@]}" -I 256 "${loop}p1"
  populate "${loop}p1"
  manifest=$(volume_manifest "${loop}p1")
  write_expect "gpt-$sector" "$manifest"
done

# MBR: one primary and two logical ext4 partitions.
img=$OUT/mbr-logical.img
new_image "$img" 200M
loop=$(attach "$img")
printf 'label: dos\nstart=1MiB, size=60MiB, type=83\nsize=+, type=5\nsize=60MiB, type=83\nsize=+, type=83\n' |
  sfdisk -q "$loop"
blockdev --rereadpt "$loop" 2>/dev/null || true
udevadm settle 2>/dev/null || sleep 1
volumes=()
for part in p1 p5 p6; do
  mkfs.ext4 "${EXT_COMMON[@]}" -I 256 "$loop$part"
  populate "$loop$part"
  manifest=$(volume_manifest "$loop$part")
  volumes+=("$manifest")
done
write_expect mbr-logical "${volumes[@]}"
