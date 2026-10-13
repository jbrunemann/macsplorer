"""Disassemble one function with PDB symbol annotations, or list functions that reference a symbol.

usage: python3 -I tools/re/dis.py <pubs.tsv> <pe> <VA | symbol substring>
       python3 -I tools/re/dis.py <pubs.tsv> <pe> --xrefs <VA | symbol substring>
"""
import bisect, re, subprocess, sys

OBJDUMP = "/opt/homebrew/opt/llvm/bin/llvm-objdump"
pubs, pe = sys.argv[1], sys.argv[2]
xrefs = sys.argv[3] == "--xrefs"
target = sys.argv[4] if xrefs else sys.argv[3]

syms, funcs = {}, {}
for line in open(pubs, encoding="utf-8"):
    va, kind, name = line.rstrip("\n").split("\t")
    a = int(va, 16)
    syms.setdefault(a, name)
    if kind == "F":
        funcs.setdefault(a, name)
fa = sorted(funcs)


def resolve(t):
    return int(t, 16) if t.startswith("0x") else next(a for a in sorted(syms) if t in syms[a])


def annotate(line):
    for m in re.finditer(r"0x([0-9a-f]{8,})", line):
        a = int(m.group(1), 16)
        if a in syms:
            return f"{line}   <{syms[a][:90]}>"
    return line


def objdump(start=None, stop=None):
    args = [OBJDUMP, "-d", "--no-show-raw-insn", "--x86-asm-syntax=intel", pe]
    if start:
        args[1:1] = [f"--start-address=0x{start:x}", f"--stop-address=0x{stop:x}"]
    return subprocess.run(args, capture_output=True, text=True).stdout.splitlines()


t = resolve(target)
if xrefs:
    hits = {}
    tag = f"0x{t:x}"
    for line in objdump():
        if tag in line and ":" in line:
            try:
                a = int(line.split(":")[0], 16)
            except ValueError:
                continue
            f = fa[bisect.bisect_right(fa, a) - 1]
            hits.setdefault(funcs[f], []).append(a)
    for name, sites in hits.items():
        print(f"{name}\t{', '.join(hex(s) for s in sites[:4])}")
else:
    end = fa[bisect.bisect_right(fa, t)] if bisect.bisect_right(fa, t) < len(fa) else t + 0x400
    print(f"== {syms.get(t, hex(t))}")
    for line in objdump(t, min(end, t + 0x3000)):
        if re.match(r"\s*[0-9a-f]+:", line):
            print(annotate(line.strip()))
