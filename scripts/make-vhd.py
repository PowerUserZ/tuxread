#!/usr/bin/env python3
"""Turns a raw disk image into a fixed VHD: the same bytes plus a 512-byte footer.

Windows can attach the result read-only as a physical disk (Mount-DiskImage), which lets
tests read a known image through \\\\.\\PhysicalDriveN.

Usage: make-vhd.py <raw image> <output .vhd>
Format: Microsoft "Virtual Hard Disk Image Format Specification", hard disk footer.
"""
import shutil
import struct
import sys
import time
import uuid

SECTOR = 512


def chs(size: int) -> int:
    """CHS geometry as the VHD specification computes it, packed as C(16) H(8) S(8)."""
    total = min(size // SECTOR, 65535 * 16 * 255)
    if total >= 65535 * 16 * 63:
        spt, heads = 255, 16
        cth = total // spt
    else:
        spt = 17
        cth = total // spt
        heads = max((cth + 1023) // 1024, 4)
        if cth >= heads * 1024 or heads > 16:
            spt, heads = 31, 16
            cth = total // spt
        if cth >= heads * 1024:
            spt, heads = 63, 16
            cth = total // spt
    return ((cth // heads) << 16) | (heads << 8) | spt


def footer(size: int) -> bytes:
    seconds = int(time.time()) - 946_684_800  # since 2000-01-01T00:00:00Z
    f = bytearray(512)
    struct.pack_into(
        ">8sIIQI4sI4sQQIII16sB",
        f,
        0,
        b"conectix",  # cookie
        2,  # features: reserved bit, always set
        0x0001_0000,  # format version
        0xFFFF_FFFF_FFFF_FFFF,  # data offset: none for a fixed disk
        seconds,
        b"tuxr",  # creator application
        0x0001_0000,  # creator version
        b"Wi2k",  # creator host OS
        size,  # original size
        size,  # current size
        chs(size),
        2,  # disk type: fixed
        0,  # checksum, filled in below
        uuid.uuid4().bytes,
        0,  # saved state
    )
    struct.pack_into(">I", f, 64, ~sum(f) & 0xFFFF_FFFF)
    return bytes(f)


def main(src: str, dst: str) -> None:
    shutil.copyfile(src, dst)
    with open(dst, "r+b") as out:
        size = out.seek(0, 2)
        if size % SECTOR or size < 3 * 1024 * 1024:
            raise SystemExit(f"{src}: size {size} must be a multiple of 512 and at least 3 MiB")
        out.write(footer(size))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
