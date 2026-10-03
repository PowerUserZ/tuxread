#!/usr/bin/env bash
# Tiny seed inputs for the fuzz targets, written to fuzz/corpus/<target>/.
# Needs e2fsprogs and sfdisk, but not root.  Usage: scripts/make-fuzz-seeds.sh
set -euo pipefail

SEEDS=$(dirname "$(realpath "$0")")/../fuzz/corpus
TREE=$(mktemp -d)
trap 'rm -rf "$TREE"' EXIT
mkdir -p "$TREE/dir" "$SEEDS/probe" "$SEEDS/ext" "$SEEDS/part"
printf 'alpha\n' > "$TREE/a.txt"
printf 'beta\n' > "$TREE/dir/b.txt"
ln -s a.txt "$TREE/link"

ext() { # ext <name> <size> <mke2fs options...>
  local img=$SEEDS/ext/$1.img
  rm -f "$img"
  truncate -s "$2" "$img"
  shift 2
  mke2fs -q -F -d "$TREE" "$@" "$img"
}
ext ext2 256K -t ext2 -b 1024
ext ext3 2M -t ext3 -b 1024
ext ext4 512K -t ext4 -b 1024 -O ^has_journal
ext ext4-journal 2M -t ext4 -b 1024
cp "$SEEDS"/ext/*.img "$SEEDS/probe/"

table() { # table <name> <sfdisk script>
  local img=$SEEDS/part/$1.img
  rm -f "$img"
  truncate -s 4M "$img"
  printf '%b' "$2" | sfdisk -q "$img"
}
table gpt 'label: gpt\nstart=1MiB, type=linux\n'
# Explicit sectors: on tiny disks sfdisk would otherwise use the gap before partition 1.
table mbr 'label: dos\nstart=2048, size=2048, type=83\nstart=4096, size=4096, type=5\nstart=4160, size=1024, type=83\nstart=6208, size=1024, type=83\n'

# A whole disk for the probe target: GPT with an ext4 filesystem in its partition.
img=$SEEDS/probe/gpt-ext4.img
cp "$SEEDS/part/gpt.img" "$img"
mke2fs -q -F -t ext4 -b 1024 -O ^has_journal -d "$TREE" -E offset=1048576 "$img" 512
echo "seeds written to $(realpath "$SEEDS")"
