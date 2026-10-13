"""Downloads the public PDB for a Windows PE from Microsoft's symbol server.

usage: python3 -I tools/re/fetch_pdb.py <pe> [out_dir]   (default out_dir: symbols/)

Reads the CodeView (RSDS) record for the PDB name, GUID and age, then fetches
https://msdl.microsoft.com/download/symbols/<name>/<GUID><age>/<name>.
"""
import pathlib
import struct
import sys
import urllib.request
import uuid

pe = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else "symbols")
data = pe.read_bytes()
i = data.find(b"RSDS")
if i < 0:
    sys.exit(f"{pe}: no CodeView RSDS record")
guid = uuid.UUID(bytes_le=data[i + 4 : i + 20]).hex.upper()
age = struct.unpack("<I", data[i + 20 : i + 24])[0]
name = data[i + 24 : data.index(b"\0", i + 24)].decode().split("\\")[-1]
url = f"https://msdl.microsoft.com/download/symbols/{name}/{guid}{age:X}/{name}"
req = urllib.request.Request(url, headers={"User-Agent": "Microsoft-Symbol-Server/10.0.0.0"})
out.mkdir(parents=True, exist_ok=True)
with urllib.request.urlopen(req, timeout=120) as r:
    (out / name).write_bytes(r.read())
print(f"{out / name}  ({guid}{age:X})")
