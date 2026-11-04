#!/usr/bin/env python3
"""Builds a Termux .deb from the bundle that scripts/bundle-termux.sh lays out.

    python3 -I scripts/termux-deb.py <bundle dir> <version> <arch> <out.deb>

Termux's `pkg` is apt and dpkg with the prefix /data/data/com.termux/files/usr, so a package is an ordinary .deb whose
files live under that prefix. `pkg install ./macsplorer_<version>_<arch>.deb` installs it and pulls in the Depends.

This writes the .deb itself (an `ar` archive of debian-binary, control.tar.gz and data.tar.gz) instead of calling
dpkg-deb, so it runs the same on a Mac, on the CI runner and inside Termux, which have different `ar` and `tar`.
"""
import gzip
import hashlib
import io
import os
import sys
import tarfile
import time

PREFIX = "data/data/com.termux/files/usr"

# The X11 libraries the program opens with dlopen, Mesa for OpenGL, xdg-utils for opening files.
DEPENDS = "libx11, libxcb, libxcursor, libxi, libxrandr, libxkbcommon, mesa, xdg-utils"
RECOMMENDS = "termux-x11-nightly"
MAINTAINER = "Jan Brünemann <jan@kaliber.net>"
HOMEPAGE = "https://github.com/jbrunemann/macsplorer"
DESCRIPTION = """Windows 2000 style file browser
 Macsplorer is a Windows Explorer style file browser with the Windows Classic
 look. This build is the desktop program for Termux:X11: start the X server
 with `termux-x11 :0 &`, then run `DISPLAY=:0 macsplorer [folder]`."""


def payload(bundle):
    """The files of the package as (path inside the .deb, mode, bytes), sorted, without the leading ./"""
    files = []

    def add(dest, src, mode):
        with open(os.path.join(bundle, src), "rb") as f:
            files.append((f"{PREFIX}/{dest}", mode, f.read()))

    add("bin/macsplorer", "macsplorer", 0o755)
    add("share/applications/macsplorer.desktop", "macsplorer.desktop", 0o644)
    add("share/doc/macsplorer/LICENSE-dejavu.txt", "LICENSE-dejavu.txt", 0o644)
    icons = os.path.join(bundle, "icons", "hicolor")
    for size in sorted(os.listdir(icons)):
        png = os.path.join("icons", "hicolor", size, "apps", "macsplorer.png")
        if os.path.exists(os.path.join(bundle, png)):
            add(f"share/icons/hicolor/{size}/apps/macsplorer.png", png, 0o644)
    return sorted(files)


def tar_gz(entries, mtime):
    """A gzip'd tar of (name, mode, data) with root as owner and a fixed time, so the build is reproducible.
    Directories are listed too: dpkg wants every parent of a file in the archive."""
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.GNU_FORMAT) as tar:
        def meta(info, mode):
            info.mtime, info.mode = mtime, mode
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            return info

        seen = set()
        for name, _, _ in entries:
            parts = name.split("/")[:-1]
            for i in range(1, len(parts) + 1):
                d = "/".join(parts[:i])
                if d not in seen:
                    seen.add(d)
                    info = meta(tarfile.TarInfo("./" + d), 0o755)
                    info.type = tarfile.DIRTYPE
                    tar.addfile(info)
        for name, mode, data in entries:
            info = meta(tarfile.TarInfo("./" + name), mode)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=mtime, compresslevel=9) as gz:
        gz.write(raw.getvalue())
    return out.getvalue()


def ar(members, mtime):
    """The common (GNU/System V) ar format dpkg reads: a magic line, then a 60 byte header before each member."""
    out = bytearray(b"!<arch>\n")
    for name, data in members:
        header = f"{name + '/':<16}{mtime:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n"
        assert len(header) == 60, header
        out += header.encode("ascii") + data
        if len(data) % 2:
            out += b"\n"
    return bytes(out)


def main():
    bundle, version, arch, out_path = sys.argv[1:5]
    mtime = int(os.environ.get("SOURCE_DATE_EPOCH", time.time()))
    files = payload(bundle)
    data = tar_gz(files, mtime)
    installed_kib = -(-sum(len(d) for _, _, d in files) // 1024)

    control = f"""Package: macsplorer
Version: {version}
Architecture: {arch}
Maintainer: {MAINTAINER}
Installed-Size: {installed_kib}
Depends: {DEPENDS}
Recommends: {RECOMMENDS}
Section: x11
Priority: optional
Homepage: {HOMEPAGE}
Description: {DESCRIPTION}
"""
    md5sums = "".join(f"{hashlib.md5(d).hexdigest()}  {n}\n" for n, _, d in files)
    control_tar = tar_gz(
        [("control", 0o644, control.encode("utf-8")), ("md5sums", 0o644, md5sums.encode("utf-8"))], mtime
    )
    with open(out_path, "wb") as f:
        f.write(ar([("debian-binary", b"2.0\n"), ("control.tar.gz", control_tar), ("data.tar.gz", data)], mtime))
    print(out_path)


if __name__ == "__main__":
    main()
