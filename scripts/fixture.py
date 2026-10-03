#!/usr/bin/env python3
"""Writes the TuxRead fixture tree (spec §7.1) into a mounted, empty filesystem.

Usage: fixture.py <mount-point>   (as root: it sets owners)
"""
import os
import random
import sys

# 2001-02-03T04:05:06.789Z
FIXED_NS = 981_173_106_789_000_000
PRE_1970_NS = -31_536_000 * 10**9  # 1969-01-01
POST_2038_NS = (2**31 + 1000) * 10**9  # 2038-01-19, past the 32-bit limit


def main(root: bytes) -> None:
    def p(*parts):
        return os.path.join(root, *[x if isinstance(x, bytes) else x.encode() for x in parts])

    def write(path, data):
        with open(path, "wb") as f:
            f.write(data)

    rng = random.Random(1)
    write(p("empty"), b"")
    write(p("small.txt"), b"hello tuxread\n")
    write(p("rand-1m.bin"), rng.randbytes(1 << 20))
    write(p("rand-16m.bin"), rng.randbytes(16 << 20))
    with open(p("sparse.bin"), "wb") as f:
        f.seek(32 << 20)
        f.write(rng.randbytes(1 << 20))
        f.truncate(64 << 20)
    # Interleaved, synced appends make the allocator alternate between two files,
    # so frag.bin ends up with many extents.
    with open(p("frag.bin"), "wb") as a, open(p("frag-pad.bin"), "wb") as b:
        for _ in range(64):
            for f in (a, b):
                f.write(rng.randbytes(16 << 10))
                f.flush()
                os.fsync(f.fileno())

    deep = p("deep")
    for i in range(1, 33):
        deep = os.path.join(deep, b"d%02d" % i)
    os.makedirs(deep)
    write(os.path.join(deep, b"leaf.txt"), b"deep\n")

    os.mkdir(p("many"))
    for i in range(10_000):
        write(p("many", "f%05d" % i), b"")

    os.mkdir(p("names"))
    for name in ["türkçe-çğıöşü.txt", "日本語.txt", "emoji-😀.txt"]:
        write(p("names", name), name.encode())
    write(p("names", b"bad-\xff\xfe.txt"), b"not UTF-8\n")

    os.mkdir(p("win"))
    for name in ["CON", "a:b", "trailing.", "File", "file"]:
        write(p("win", name), name.encode())

    os.mkdir(p("links"))
    os.symlink(b"../small.txt", p("links", "to-file"))
    os.symlink(b"../deep", p("links", "to-dir"))
    os.link(p("small.txt"), p("links", "hard"))
    os.mkfifo(p("links", "fifo"))

    os.mkdir(p("modes"))
    for name, mode, owner in [("0600.txt", 0o600, 0), ("0755.sh", 0o755, 1000), ("4755.bin", 0o4755, 1000)]:
        write(p("modes", name), b"#\n")
        os.chown(p("modes", name), owner, owner)  # chown clears setuid, so chmod comes after
        os.chmod(p("modes", name), mode)

    os.mkdir(p("times"))
    write(p("times", "pre-1970.txt"), b"old\n")
    write(p("times", "post-2038.txt"), b"new\n")

    # Fixed times everywhere, children before parents so folder times stick.
    for dirpath, dirnames, filenames in os.walk(root, topdown=False):
        for name in filenames + dirnames:
            os.utime(os.path.join(dirpath, name), ns=(FIXED_NS, FIXED_NS), follow_symlinks=False)
    os.utime(p("times", "pre-1970.txt"), ns=(PRE_1970_NS, PRE_1970_NS))
    os.utime(p("times", "post-2038.txt"), ns=(POST_2038_NS, POST_2038_NS))
    os.utime(root, ns=(FIXED_NS, FIXED_NS))


if __name__ == "__main__":
    main(os.fsencode(sys.argv[1]))
