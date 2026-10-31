#!/usr/bin/env python3
"""Copies the crates a Termux build needs patched out of the cargo registry into a folder.

Termux is Android's kernel and C library with a desktop X11 window, but winit, glutin and the crates around
them switch their X11, clipboard and Linux code off for every `target_os = "android"` (they only know the
native activity). In the copies, `android` never matches and `linux` also matches android, so they build as
the desktop Linux crates they are, for the Android target. No source is changed beyond those two cfg names.
Used by scripts/bundle-termux.sh, which hands the folder to cargo as `[patch.crates-io]`.

Usage: termux-patch.py <Cargo.lock> <output folder>
Prints one `name=path` line per crate.
"""
import glob
import os
import re
import shutil
import sys

# The crates whose Cargo.toml, build script or source tell Android apart from desktop Linux, and whether
# `linux` should also match android there. Not in egui-winit: its Linux clipboard is Wayland only (smithay),
# which doesn't build for Android, and the X11 clipboard is arboard's.
CRATES = {"winit": True, "eframe": True, "egui-winit": False, "glutin": True, "glutin_glx_sys": True, "glutin-winit": False, "arboard": False, "trash": False}

ANDROID = re.compile(r'target_os\s*=\s*"android"')
LINUX = re.compile(r'target_os\s*=\s*"linux"')
MARK = "\0LINUX\0"


def versions(lock):
    found = {}
    name = None
    for line in open(lock):
        if line.startswith("name = "):
            name = line.split('"')[1]
        elif line.startswith("version = ") and name in CRATES:
            found.setdefault(name, []).append(line.split('"')[1])
    return found


def patch(path, linux_too):
    text = open(path, encoding="utf-8").read()
    new = LINUX.sub(MARK, text) if linux_too else text
    new = ANDROID.sub('target_os = "termux_never"', new)
    new = new.replace(MARK, 'any(target_os = "linux", target_os = "android")')
    if new != text:
        open(path, "w", encoding="utf-8").write(new)


def main():
    lock, out = sys.argv[1:3]
    cargo_home = os.environ.get("CARGO_HOME") or os.path.expanduser("~/.cargo")
    found = versions(lock)
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    for crate in CRATES:
        vs = found.get(crate, [])
        if len(vs) != 1:
            sys.exit(f"termux-patch: {crate} has {len(vs)} versions in Cargo.lock ({vs}), expected 1")
        src = glob.glob(f"{cargo_home}/registry/src/*/{crate}-{vs[0]}")
        if not src:
            sys.exit(f"termux-patch: {crate} {vs[0]} is not in the cargo registry (run cargo fetch)")
        dest = os.path.join(out, crate)
        shutil.copytree(src[0], dest)
        for root, _, files in os.walk(dest):
            for name in files:
                if name.endswith((".rs", ".toml")):
                    os.chmod(os.path.join(root, name), 0o644)
                    patch(os.path.join(root, name), CRATES[crate])
        print(f"{crate}={os.path.abspath(dest)}")


main()
