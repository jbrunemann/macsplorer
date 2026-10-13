#!/usr/bin/env python3 -I
"""Refreshes assets/seti from the VS Code repo (MIT).

- seti.ttf                 Seti glyph font, converted from the shipped WOFF
- vs-seti-icon-theme.json  icon theme: names/extensions/languages -> glyph+color
- languages.json           extension/file name -> VS Code language id, taken
                           from the built-in language extensions, because the
                           theme maps many types (rust, python, ...) by id only
- license notices

Usage: scripts/update-seti.py
"""

import json
import pathlib
import struct
import urllib.request
import zlib

REPO = "microsoft/vscode"
RAW = f"https://raw.githubusercontent.com/{REPO}/main"
OUT = pathlib.Path(__file__).resolve().parent.parent / "assets" / "seti"


def get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "macsplorer-update-seti"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def woff_to_ttf(woff: bytes) -> bytes:
    """Converts WOFF 1.0 to a plain sfnt (TrueType) font."""
    sig, flavor, _length, num_tables = struct.unpack(">4sIIH", woff[:14])
    assert sig == b"wOFF", "not a WOFF 1.0 file"
    tables = []
    for i in range(num_tables):
        tag, off, comp_len, orig_len, checksum = struct.unpack(">4sIIII", woff[44 + 20 * i : 64 + 20 * i])
        data = woff[off : off + comp_len]
        if comp_len < orig_len:
            data = zlib.decompress(data)
        assert len(data) == orig_len, f"bad table {tag!r}"
        tables.append((tag, checksum, data))

    entry_selector = max(0, num_tables.bit_length() - 1)
    search_range = 16 << entry_selector
    out = bytearray(struct.pack(">IHHHH", flavor, num_tables, search_range, entry_selector, num_tables * 16 - search_range))
    offset = 12 + 16 * num_tables
    body = bytearray()
    for tag, checksum, data in sorted(tables):
        out += struct.pack(">4sIII", tag, checksum, offset + len(body), len(data))
        body += data + b"\0" * (-len(data) % 4)
    return bytes(out + body)


def languages() -> dict:
    """Collects language contributions from VS Code's built-in extensions."""
    listing = json.loads(get(f"https://api.github.com/repos/{REPO}/contents/extensions"))
    by_ext, by_name = {}, {}
    for item in sorted(listing, key=lambda i: i["name"]):
        if item["type"] != "dir":
            continue
        try:
            pkg = json.loads(get(f"{RAW}/extensions/{item['name']}/package.json"))
        except Exception:
            continue
        for lang in pkg.get("contributes", {}).get("languages", []):
            lid = lang.get("id")
            if not lid:
                continue
            for ext in lang.get("extensions", []):
                by_ext.setdefault(ext.lower().lstrip("."), lid)
            for name in lang.get("filenames", []):
                by_name.setdefault(name.lower(), lid)
    return {"extensions": dict(sorted(by_ext.items())), "fileNames": dict(sorted(by_name.items()))}


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    seti = f"{RAW}/extensions/theme-seti"
    (OUT / "seti.ttf").write_bytes(woff_to_ttf(get(f"{seti}/icons/seti.woff")))
    (OUT / "vs-seti-icon-theme.json").write_bytes(get(f"{seti}/icons/vs-seti-icon-theme.json"))
    (OUT / "ThirdPartyNotices.txt").write_bytes(get(f"{seti}/ThirdPartyNotices.txt"))
    (OUT / "LICENSE.vscode.txt").write_bytes(get(f"{RAW}/LICENSE.txt"))
    langs = languages()
    (OUT / "languages.json").write_text(json.dumps(langs, indent=1) + "\n")
    print(f"wrote {OUT}: {len(langs['extensions'])} extensions, {len(langs['fileNames'])} file names")


if __name__ == "__main__":
    main()
