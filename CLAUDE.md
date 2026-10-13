# macsplorer

A Windows Explorer–style file browser for macOS in Rust, with a Windows 2000
"Windows Classic" look. Behaviors are traced from Windows 10 build 10240
binaries and re-implemented clean-room: write the *behavior* into `docs/`,
never paste decompiled code into the source.

## Layout

| Path | What |
|---|---|
| `crates/macsplorer-core` | UI-independent logic, all unit-tested: `listing` (read_dir, sort, sizes, custom-icon flag), `natural` (StrCmpLogicalW-style sort), `rename` (Explorer's rename selection + APFS name rules), `history` (back/forward), `breadcrumb`, `places`, `tree` (Folders pane model, CNscTree-like), `seti` (VSCodium file icon resolution). |
| `crates/macsplorer` | egui/eframe 0.36 app. `app.rs` state + UI, `theme.rs` Windows Classic colors/bevels/fonts/icons/title bar, `icons.rs` icon choice + native icon cache, `native_icon.rs` NSWorkspace icons via objc2. |
| `tools/make-icon` | Renders `assets/icon.png` + `assets/macsplorer.icns` (tiny-skia). |
| `tools/re` | PDB/disassembly helpers for the Windows analysis (see its README). |
| `scripts/bundle-macos.sh` | Builds `target/release/macsplorer.app`. |
| `scripts/update-seti.py` | Refreshes `assets/seti` (Seti font/theme + language table) from the VS Code repo. |
| `docs/` | Findings from the Windows binaries. `file-browser-findings.md` §4 lists untraced behaviors. |

## Commands

```sh
cargo run -p macsplorer -- [folder]
cargo test --workspace
cargo clippy --workspace --all-targets          # keep it warning-free
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
  highlights only the label text, Tahoma 8pt (loaded from
  `/System/Library/Fonts/Supplemental`, not bundled). Every file icon is the
  Win2000 paper page (`theme::paint_page`) with the Seti glyph as an emblem;
  apps/packages/Finder custom icons use the real macOS icon; folders are the
  yellow Win2000 folder.
- The window is borderless (`with_decorations(false)`) so macOS doesn't
  round its corners; the caption bar is drawn by `theme::title_bar`.
- UI code collects `Action`s while drawing and applies them afterwards
  (`Explorer::apply`), so drawing only needs shared borrows.
- Navigation pane folder listings run on worker threads (`Explorer::load`)
  and come back through a channel; never block the UI thread on disk I/O.
- Core logic goes in `macsplorer-core` with tests; keep the app crate thin.

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
  `ExplorerFrame.dll`, `shell32.dll`, `Windows.Storage.dll` (build 10240).
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
- Copy / move / delete (`CFileOperation` in Windows.Storage is the reference), drag and drop.
- More Win2000 views: Large Icons, List, the "Web view" left info panel.
- Type column with language names ("Rust Source") and image thumbnails.
- Trace remaining Explorer behaviors listed in `docs/file-browser-findings.md` §4.
- Scroll bar arrows / 3D thumb (egui limitation), column header filler to the right edge.
- Persist settings, column widths and window size; pinning favorites; tabs/multiple windows.
- Replace the placeholder bundle id `dev.macsplorer.app` once the user picks one.
