"""Dump S_PUB32 public symbols from an MSF 7.0 PDB as VA<TAB>name, using a PE for section bases."""
import struct, sys

pdb_path, pe_path = sys.argv[1], sys.argv[2]
d = open(pdb_path, "rb").read()
assert d.startswith(b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0")
bs, _fpm, nblocks, dir_bytes, _r, dir_map = struct.unpack_from("<6I", d, 32)

def blocks(idx_list, size):
    return b"".join(d[b * bs:(b + 1) * bs] for b in idx_list)[:size]

ndir_blocks = (dir_bytes + bs - 1) // bs
dir_block_idx = struct.unpack_from(f"<{ndir_blocks}I", d, dir_map * bs)
directory = blocks(dir_block_idx, dir_bytes)
nstreams = struct.unpack_from("<I", directory, 0)[0]
sizes = struct.unpack_from(f"<{nstreams}I", directory, 4)
pos = 4 + 4 * nstreams
streams = []
for s in sizes:
    if s == 0xFFFFFFFF:
        streams.append(b"")
        continue
    n = (s + bs - 1) // bs
    idx = struct.unpack_from(f"<{n}I", directory, pos)
    pos += 4 * n
    streams.append(blocks(idx, s))

dbi = streams[3]
sym_rec_stream = struct.unpack_from("<H", dbi, 20)[0]
recs = streams[sym_rec_stream]

pe = open(pe_path, "rb").read()
peoff = struct.unpack_from("<I", pe, 0x3C)[0]
nsec = struct.unpack_from("<H", pe, peoff + 6)[0]
optsz = struct.unpack_from("<H", pe, peoff + 20)[0]
image_base = struct.unpack_from("<Q", pe, peoff + 24 + 24)[0]
sec_tab = peoff + 24 + optsz
sec_rva = [struct.unpack_from("<I", pe, sec_tab + 40 * i + 12)[0] for i in range(nsec)]

p = 0
while p + 4 <= len(recs):
    reclen, kind = struct.unpack_from("<HH", recs, p)
    if kind == 0x110E:  # S_PUB32
        flags, off, seg = struct.unpack_from("<IIH", recs, p + 4)
        end = recs.index(b"\0", p + 14)
        name = recs[p + 14:end].decode("utf-8", "replace")
        if 1 <= seg <= nsec:
            va = image_base + sec_rva[seg - 1] + off
            print(f"0x{va:x}\t{'F' if flags & 2 else 'D'}\t{name}")
    p += reclen + 2
