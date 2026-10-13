# Macsplorer

A Windows Explorer–style file browser for macOS (with an experimental Linux port) in Rust, with a Windows 2000
"Windows Classic" look. Behaviors are traced from Windows 10 build 10240
binaries and re-implemented clean-room: write the *behavior* into `docs/`,
never paste decompiled code into the source.

## Layout

| Path | What |
|---|---|
| `crates/macsplorer-core` | UI-independent logic, all unit-tested: `listing` (read_dir, sort, sizes, custom-icon flag), `natural` (StrCmpLogicalW-style sort), `rename` (Explorer's rename selection + APFS name rules), `fileops` (paste: copy/move with "- Copy" names, xattr-preserving), `history` (back/forward), `breadcrumb`, `places`, `tree` (Folders pane model, CNscTree-like), `layout` (Large Icons and List geometry and arrow keys, after the comctl32 ListView), `marquee` (rubber band selection, same source), `columns` (Details header divider hit test and resize drag, after SysHeader32), `seti` (VSCodium file icon resolution), `undo` (undo stack for rename/move/copy/delete). |
| `crates/macsplorer` | egui/eframe 0.36 app. `app.rs` state + UI, `platform.rs` per-OS open/reveal/eject, wording and font paths, `theme.rs` Windows Classic colors/bevels/fonts/icons/title bar, `scrollbar.rs` classic scroll bars drawn over hidden egui ones, `icons.rs` icon choice + native icon cache, `native_icon.rs` NSWorkspace icons via objc2. |
| `tools/make-icon` | Renders `assets/icon.png` + `assets/macsplorer.icns` (tiny-skia). |
| `tools/re` | PDB/disassembly helpers for the Windows analysis (see its README). |
| `scripts/bundle-macos.sh` | Builds `target/release/Macsplorer.app`. |
| `scripts/bundle-linux.sh` | Builds `target/release/macsplorer-linux-<arch>.tar.gz` (binary, `assets/macsplorer.desktop`, icon). |
| `scripts/update-seti.py` | Refreshes `assets/seti` (Seti font/theme + language table) from the VS Code repo. |
| `docs/` | Findings from the Windows binaries. `file-browser-findings.md` §4 lists untraced behaviors. |

## Commands

```sh
cargo run -p macsplorer -- [folder]
cargo test --workspace
cargo clippy --workspace --all-targets          # keep it warning-free
cargo clippy --workspace --all-targets --target x86_64-unknown-linux-gnu   # same for the Linux port (rustup target add first)
cargo test -p macsplorer -- --ignored           # offscreen render → target/screenshots/window.png
scripts/bundle-macos.sh --open
cargo run -p make-icon                          # after changing the icon design
```

**Check UI changes visually** with the ignored screenshot test (egui_kittest
+ wgpu) and read the PNG. The terminal has no Screen Recording permission, so
real-window screenshots fail. The render doesn't include the macOS window
frame. The fixture folder is built in `app.rs` `mod screenshot`.

## Conventions

- Explorer-derived behavior: cite the Windows function in a doc comment and
  in `docs/`, mark unverified guesses as such (e.g. folders-first sorting,
  history limit, "expand to open folder" default).
- Windows 2000 look is a hard requirement: system colors in `theme::color`,
  3D edges via `theme::bevel`, no hover highlight in lists, selection
  highlights only the label text, Tahoma at 12 px, one step above Win2000's 8pt (loaded from
  `/System/Library/Fonts/Supplemental`, not bundled). Every file icon is the
  Win2000 paper page (`theme::paint_page`) with the Seti glyph as an emblem;
  apps/packages/Finder custom icons use the real macOS icon; folders are the
  yellow Win2000 folder (the user folders Finder gives a sidebar glyph carry that SF Symbol as an emblem).
- The window is borderless (`with_decorations(false)`) so macOS doesn't
  round its corners; the caption bar is drawn by `theme::title_bar`.
- UI code collects `Action`s while drawing and applies them afterwards
  (`Explorer::apply`), so drawing only needs shared borrows.
- Folder listings run on worker threads and come back through channels
  (`Explorer::load` for the navigation pane, `Explorer::reload` for the
  details view); never block the UI thread on disk I/O.
- Core logic goes in `macsplorer-core` with tests; keep the app crate thin.
- The user-facing name is "Macsplorer" (capital M): window title, About, bundle
  name. Crate, binary, file names and bundle id stay lowercase `macsplorer`.

## Linux port

Cross-checking from a Mac works (`cargo check|clippy --target x86_64-unknown-linux-gnu`), but it
can't run, so Linux behavior is only exercised by the CI job on `ubuntu-latest`. OS differences
live behind `#[cfg(target_os = "macos")]` with a Linux branch next to it: `platform.rs`
(open/reveal/eject, `CMD` tooltip text, Tahoma paths), `trash.rs` (`trash` crate), `places.rs`
(`/proc/self/mounts` volumes), `theme::resize_frame` (borderless-window resize handles).
Not ported yet: Open With (`open_with::SUPPORTED`), native icons, SF Symbol emblems.

## egui 0.36 notes (newer than most examples)

- `eframe::App` requires `fn ui(&mut self, ui: &mut Ui, frame)`, not `update`.
- Panels: `egui::Panel::top/bottom/left(id).show(ui, …)`; `CentralPanel::default().show(ui, …)`.
- Menus: `egui::MenuBar::new().ui(ui, …)`, `egui::Popup::menu(&response).show(…)`, `ui.close()`.
- Text cursor: `TextEdit::load_state` / `store_state`, `state.cursor.set_char_range(Some(CCursorRange::two(..)))`.
- `egui_extras` table rows paint `widgets.hovered.bg_fill` on hover; the
  details view overrides it locally to white.
- Check APIs in `~/.cargo/registry/src/*/egui-0.36.2` before guessing.

## Never commit

Microsoft binaries/symbols: `*.blob`, `analysis/`, `symbols/*.pdb|tsv|asm|txt`
(already in `.gitignore`). `assets/seti/` is MIT and is committed with its
licenses.

## Windows analysis setup (local, not in git)

- `*.blob` in the repo root are the user's copies of `explorer.exe`,
  `ExplorerFrame.dll`, `shell32.dll`, `Windows.Storage.dll` (build 10240) and
  `comctl32.dll` v6.10 (the SysListView32 that Win2000's Explorer view was built on).
  `analysis/` hard-links them under their real names next to their PDBs.
- `symbols/*.pubs.tsv` = recovered public symbols (≈100k functions).
- REA MCP with Ghidra 12.1.4 at `~/opt/ghidra_12.1.4_PUBLIC` (macOS arm64
  decompiler built locally; `GHIDRA_INSTALL_DIR` set in the `rea` server env
  in `~/.claude.json`). Ghidra headless does not load the PDBs: decompile by
  address and resolve names via the `.pubs.tsv`. Opening `shell32.dll` takes
  a few minutes of analysis the first time per session.
- Hopper via REA crashed on these binaries; prefer Ghidra.

## Backlog (ideas raised so far)

- File watching (FSEvents) so the list and tree update live.
- Copy / move / delete and undo are in (menu, Edit menu, ⌘X/C/V/Z, Delete, toolbar Undo). Still open: the replace/skip conflict dialog, progress UI, Shift+Delete, redo, Finder pasteboard interop, drag and drop (`CFileOperation` in Windows.Storage is the reference).
- More Win2000 views: Small Icons, Thumbnails, the "Web view" left info panel; a dotted focus rectangle on the focused item.
- Type column with language names ("Rust Source") and image thumbnails.
- Trace remaining Explorer behaviors listed in `docs/file-browser-findings.md` §4.
- Classic scroll bars for the breadcrumb chevron popup.
- Persist settings and window size (column widths are deliberately per window only, shared by all folders); pinning favorites; tabs/multiple windows.
- Replace the placeholder bundle id `dev.macsplorer.app` once the user picks one.
