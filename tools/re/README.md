# Reverse-engineering helpers

Used to study Windows 10 build 10240 Explorer behavior (see `docs/`). The
Microsoft binaries and symbols are **not** in git; keep them local
(`*.blob`, `analysis/`, `symbols/*.pdb|tsv|asm|txt` are ignored).

Run Python with `-I` (isolated) since the inputs are downloaded files.

| Script | Purpose |
|---|---|
| `fetch_pdb.py <pe> [out]` | Download the matching public PDB from Microsoft's symbol server. |
| `pdbpubs.py <pdb> <pe> > x.pubs.tsv` | Dump PDB public symbols as `VA \t F/D \t mangled name` (llvm-pdbutil crashes on these stripped PDBs). Demangle with `/opt/homebrew/opt/llvm/bin/llvm-cxxfilt`. |
| `methods.py <pubs.tsv> <Class>...` | List a class's method names. |
| `dis.py <pubs.tsv> <pe> <VA\|name>` | Disassemble one function (llvm-objdump) with PDB names on call targets. |
| `dis.py <pubs.tsv> <pe> --xrefs <VA\|name>` | List functions that reference an address or symbol. |

Workflow for a new behavior question:

1. Find candidate classes/functions: `grep` the `*.pubs.tsv` files or `methods.py`.
2. Read the call structure with `dis.py`.
3. Decompile with Ghidra through the REA MCP server (`open_binary` on
   `analysis/<Name>.dll`, `provider_id: "ghidra"`), then `procedure_pseudo_code`
   **by address** (Ghidra headless doesn't apply the PDB, so resolve `FUN_…`
   names against the `.pubs.tsv`).
4. Write the behavior (not the code) into `docs/`, marking Observed vs Inferred.
