#!/usr/bin/env python3
"""Prints what the Linux kernel sees in a mounted filesystem, as one corpus volume (spec §7.1).

Usage: manifest.py <mount-point>  ->  {"outcome": "match", "entries": [...]} on stdout
"""
import base64
import hashlib
import json
import os
import stat
import sys


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


def kind(mode: int) -> str:
    if stat.S_ISREG(mode):
        return "file"
    if stat.S_ISDIR(mode):
        return "dir"
    if stat.S_ISLNK(mode):
        return "symlink"
    return "other"


def sha256(path: bytes) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main(root: bytes) -> None:
    entries = []
    for dirpath, dirnames, filenames in os.walk(root):
        for name in dirnames + filenames:
            full = os.path.join(dirpath, name)
            st = os.lstat(full)
            k = kind(st.st_mode)
            entries.append({
                "path": b64(b"/" + os.path.relpath(full, root)),
                "kind": k,
                "size": st.st_size,
                "sha256": sha256(full) if k == "file" else None,
                "mtime_ns": st.st_mtime_ns,
                "mode": st.st_mode & 0o7777,
                "uid": st.st_uid,
                "gid": st.st_gid,
                "link": b64(os.readlink(full)) if k == "symlink" else None,
            })
    entries.sort(key=lambda e: base64.b64decode(e["path"]))
    json.dump({"outcome": "match", "entries": entries}, sys.stdout)


if __name__ == "__main__":
    main(os.fsencode(sys.argv[1]))
