# Macsplorer

A Windows Explorer–style file browser for macOS (with an experimental Linux port) in Rust, with a Windows 2000
"Windows Classic" look. Behaviors are traced from Windows 10 build 10240
binaries and re-implemented clean-room: write the *behavior* into `docs/`,
never paste decompiled code into the source.

## Layout

| Path | What |
|---|---|
| `crates/macsplorer-core` | UI-independent logic, all unit-tested: `listing` (read_dir, sort, sizes, custom-icon flag), `natural` (StrCmpLogicalW-style sort), `rename` (Explorer's rename selection + APFS name rules), `slow_click` (click-the-selected-label rename, after the comctl32 ListView; `DELAY_MS` sets the delay), `fileops` (paste: copy/move with "- Copy" names, xattr-preserving; `transfer` asks about name conflicts, merges folders, replaced items go to the Trash; after `CCopyWorkItem::ConfirmOverwrite`; `transfer_with` reports a `Progress` and can be cancelled), `copy_progress` (when the progress dialog shows and closes, after shell32's `CProgressDialog`, and its time-left text), `dragdrop` (what a drop does: copy, move or shortcut by volume and keys, after shell32's `CFSDropTarget`; the drag threshold), `watch` (live updates: watches the open folder and the expanded tree folders through `notify`, and says which to reload after a short pause), `history` (back/forward), `focus` (Tab/F6 cycle between view, address bar, search and Folders pane, after `CShellBrowser::_CycleFocus`), `isearch` (type-ahead selection, after the comctl32 ListView), `breadcrumb`, `places`, `tree` (Folders pane model, CNscTree-like), `layout` (Large Icons and List geometry and arrow keys, after the comctl32 ListView), `marquee` (rubber band selection, same source), `menu_delay` (how long the pointer rests on a submenu item before it opens, Windows' `MenuShowDelay`, 400 ms; from memory), `columns` (Details columns: which are shown and in what order, the header menu, after shell32's `CDefView`; header divider hit test and resize drag, after SysHeader32), `seti` (VSCodium file icon resolution), `undo` (undo stack for rename/move/copy/delete), `text_size` (View > Text Size steps for the whole-window zoom, after the Win2000 Font Size / DPI setting), `git` (branch name read from `.git/HEAD` and friends, status summary from `git status --porcelain --branch`, after posh-git; the branch label and the summary text live here, so they are tested; `executable()` waits for the one-time lookup, so call it from workers only, the UI thread uses `available()`; `remember()` hands it the path stored in `settings.json` before anything asks), `highlight` (colored names for executables, links and archives, after Ubuntu's `ls` colors; not from Explorer), `fonts` (Linux: which installed font beats the bundled DejaVu Sans Condensed, and which cover other scripts; the lookup itself is `system_fonts.rs` through `fontdb`, which reads the fontconfig folders), `properties` (Properties dialog data: folder size walk, size/date wording), `folder_style` (Finder's folder color and emoji/SF Symbol emblem, read and written as Finder stores them, for the Properties dialog's Customize tab, plus Macsplorer's own colors mapped to the nearest Finder tag color; Linux keeps colors only, in `user.macsplorer.folder-color`; `bplist` is the binary plist reader/writer it shares with `listing`), `hls` (the Color dialog's Hue/Sat/Lum, after Microsoft's KB 29240 sample), `xattr` (extended attributes on macOS and Linux: read, set, remove), `associations` (the app chosen with "Always use this program", by file type: a UTI on macOS), `preferences` (the `settings.json` that survives between runs: dark mode, the text size, the column layout, the view mode, the sort, the window size, which `columns` converts to and from JSON, the chosen apps (`associations`), and the git location the app found last time; read-modify-write so several windows can share it, unknown keys kept; add new persisted settings there). |
| `crates/macsplorer` | egui/eframe 0.36 app. `app.rs` state + UI, `platform.rs` per-OS open/reveal/eject, wording and font paths, `theme.rs` Windows Classic colors/bevels/fonts/icons/title bar, `scrollbar.rs` classic scroll bars drawn over hidden egui ones, `pasteboard.rs` Cut/Copy files on the macOS pasteboard, `properties.rs` the Properties dialog (own borderless window), `column_dialog.rs` the header menu's More... dialog (Column Settings), `color_dialog.rs` the Win2000 Color dialog (Other... on the Customize tab), `open_with_dialog.rs` the Open With dialog (Choose Program..., with Always use this program), `conflict_dialog.rs` Confirm File/Folder Replace (the paste worker waits on its answer), `progress_dialog.rs` the Copying.../Moving... window with Cancel, `item_drag.rs` item drags inside the window (drop zones the views register while drawing, drag image, target highlight), `native_drag.rs` drags to and from other apps through AppKit (drop methods added to winit's view, a drag that leaves the window becomes an AppKit session), `icons.rs` icon choice + native icon cache, `native_icon.rs` NSWorkspace icons via objc2, `system_fonts.rs` (Linux) finds an installed Tahoma and script fallbacks; Linux builds embed DejaVu Sans Condensed from `assets/fonts` (never in the macOS binary). |
| `crates/egui-classic-menu` | Standalone egui 0.36 crate for the Windows Classic menu rows: check/radio gutter on the left, submenu arrow gutter on the right, shortcuts left of it, whole-row highlight (`style`, `item`, `checkbox`, `radio`, `submenu_row`, `dot`). Colors come from the `Ui`'s `Visuals`, so it has no dependency on `theme.rs`; `theme::menu_style` and `theme::dot` re-export it. Reusable in other projects, see its README. |
| `tools/make-icon` | Renders `assets/icon.png` + `assets/macsplorer.icns` (tiny-skia). |
| `tools/re` | PDB/disassembly helpers for the Windows analysis (see its README). |
| `scripts/bundle-macos.sh` | Builds `target/release/Macsplorer.app`. |
| `scripts/bundle-linux.sh` | Builds `target/release/macsplorer-linux-<arch>.tar.gz` (binary, `assets/macsplorer.desktop`, icon). |
| `scripts/build-linux-docker.sh` | Builds the same Linux tarball in an Ubuntu 22.04 Docker container (arm64 natively on Apple Silicon, or pass `linux/amd64`), into `target/linux-docker/`. |
| `scripts/update-seti.py` | Refreshes `assets/seti` (Seti font/theme + language table) from the VS Code repo. |
| `scripts/publish-wiki.py` | Mirrors `docs/manual` into the GitHub wiki (one page per chapter, sidebar, screenshots). The wiki is generated: never edit it by hand. Run it after a manual change is on `master`; `--dry-run` writes to `target/wiki`. |
| `docs/manual/` | The user manual (`README.md`, plain language, no technical background) and its screenshots, refreshed with `scripts/update-manual-images.sh`. Update it when a user-visible feature changes. |
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
  The branch glyph (`icons::paint_git_branch`, vector shapes) only appears beside text, in the status bar and the Branch column, not on folder icons.
- Dark mode (View menu, remembered in `settings.json` in the config dir, see `preferences`) is not
  from the Windows binaries: build 10240 has no dark mode code, Explorer got it in
  Windows 10 1809. `theme::color` are functions that read the palette flag, so
  never cache a color across frames; icon art that must stay the same in both
  (folders, the paper page) uses the constants there. `SHOT_DARK=1` renders the
  screenshot tests dark.
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
(`/proc/self/mounts` volumes), `theme::resize_frame` (borderless-window resize handles),
`xattr` and `folder_style` (folder colors only, drawn as the Win2000 folder in the color).
Not ported yet: Open With (`open_with::SUPPORTED`), native icons, SF Symbol emblems, folder emblems, drag and drop beyond the window (`native_drag`).

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

- Copy / move / delete and undo are in (menu, Edit menu, ⌘X/C/V/Z, Delete, toolbar Undo). Cut/Copy go through the system pasteboard on macOS (`pasteboard.rs`: file URLs plus a private `dev.macsplorer.cut` type on the first item, so separate windows and Finder interoperate; Option+Cmd+V = Move Here; Finder has no cut, Command X leaves no mark on the pasteboard, checked); the Linux port still has an in-memory clipboard. Name conflicts ask with Win2000's Confirm File/Folder Replace. A paste shows Win2000's progress dialog after a second, with Cancel. Drag and drop is in (`dragdrop`, `item_drag.rs`, `native_drag.rs`); still open there: the right-drag menu, dropping on an app, Linux beyond the window. Still open: Shift+Delete, redo.
- More Win2000 views: Small Icons, Thumbnails, the "Web view" left info panel.
- Keyboard: Shift+F10/menu key context menus and Alt/F10 into the menu bar need keyboard-driven popup menus (egui's can't be); see `docs/file-browser-findings.md` §2.12 "Not done".
- Type column with language names ("Rust Source") and image thumbnails.
- Trace remaining Explorer behaviors listed in `docs/file-browser-findings.md` §4.
- Persist the rest of the settings (View menu options); the dark mode choice, the text size, the view mode, the sort, the window size (saved half a second after a resize ends) and the Details column layout (shown, order, widths, shared by all folders and windows) already are, in `settings.json`; pinning favorites; tabs.
