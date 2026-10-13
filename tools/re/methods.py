"""Print the distinct method names of one or more classes from a pubs.tsv.

usage: python3 -I tools/re/methods.py <pubs.tsv> <Class> [<Class> ...]
"""
import re, sys

BORING = {"QueryInterface", "AddRef", "Release", "GetIids", "GetRuntimeClassName", "GetTrustLevel"}
path, classes = sys.argv[1], sys.argv[2:]
names = [l.split("\t")[2].rstrip("\n") for l in open(path, encoding="utf-8") if l.split("\t")[1] == "F"]
for cls in classes:
    pat = re.compile(r"^\?([A-Za-z_][A-Za-z0-9_]*)@" + re.escape(cls) + r"@@")
    ms = sorted({m.group(1) for n in names if (m := pat.match(n))} - BORING)
    print(f"== {cls} ({len(ms)})")
    print("   " + " ".join(ms))
