# Macsplorer

A Windows Explorer–style file browser for macOS (with an experimental Linux port) in Rust, with a Windows 2000
"Windows Classic" look. Behaviors are traced from Windows 10 build 10240
binaries and re-implemented clean-room: write the *behavior* into `docs/`,
never paste decompiled code into the source.

## Layout

| Path | What |
|---|---|
| `crates/macsplorer-core` | UI-independent logic, all unit-tested: `archive` (zip and tar archives browsed as read-only folders, after `zipfldr.dll`: a place inside one is a path through the archive file, `foo.zip/src`, which `split` takes apart; the index is cached by path, size and mtime; `list` makes `Entry`s, `extract` takes items out with progress, cancel, password and the quarantine mark, never outside the destination; `default_destination` for Extract All; findings in `docs/archive-findings.md`; pure-Rust crates only, so the Linux cross-check needs no C toolchain), `listing` (read_dir, sort, sizes, custom-icon flag), `natural` (StrCmpLogicalW-style sort in the user locale's collation, ICU4X; `fold` for type-ahead and search matching), `rename` (Explorer's rename selection + APFS name rules), `slow_click` (click-the-selected-label rename, after the comctl32 ListView; `DELAY_MS` sets the delay), `fileops` (paste: copy/move with "- Copy" names, xattr-preserving; `transfer` asks about name conflicts, merges folders, replaced items go to the Trash; after `CCopyWorkItem::ConfirmOverwrite`; `transfer_with` reports a `Progress` and can be cancelled), `copy_progress` (when the progress dialog shows and closes, after shell32's `CProgressDialog`, and its time-left text), `dragdrop` (what a drop does: copy, move or shortcut by volume and keys, after shell32's `CFSDropTarget`; the drag threshold), `goto` (Go To Folder: fuzzy ranking of the folders worth jumping to, by a few letters of the name; not from Explorer), `autosuggest` (Fish-style prediction of the folder typed in the address bar: the gray rest of the name, and the address once accepted; not from Explorer), `watch` (live updates: watches the open folder and the expanded tree folders through `notify`, and says which to reload after a short pause), `history` (back/forward), `home` (the home folder as `~`: how a path is shown in the address bar being edited, Go To Folder and the status bar, and how a typed `~` and `~/…` are read; `~name` is left as typed; also knows the home folder's real path and the macOS `/System/Volumes/Data` firmlink spelling; not from Explorer), `focus` (Tab/F6 cycle between view, address bar, search and Folders pane, after `CShellBrowser::_CycleFocus`), `isearch` (type-ahead selection, after the comctl32 ListView), `places` (sidebar favorites, including symlinked folders in the home folder; the drive kind of each volume, fixed, removable, optical or network, which picks its Folders pane icon, after Win2000's `GetDriveType`, macOS asks Foundation for local/internal/removable), `tree` (Folders pane model, CNscTree-like), `layout` (Large Icons and List geometry and arrow keys, after the comctl32 ListView), `marquee` (rubber band selection, same source), `menu_delay` (how long the pointer rests on a submenu item before it opens, Windows' `MenuShowDelay`, 400 ms; from memory), `columns` (Details columns: which are shown and in what order, the header menu, after shell32's `CDefView`; header divider hit test and resize drag, after SysHeader32), `seti` (VSCodium file icon resolution), `undo` (undo stack for rename/move/copy/delete), `text_size` (View > Text Size steps for the whole-window zoom, after the Win2000 Font Size / DPI setting), `git` (branch name read from `.git/HEAD` and friends, status summary from `git status --porcelain --branch`, after posh-git; the branch label and the summary text live here, so they are tested; `executable()` waits for the one-time lookup, so call it from workers only, the UI thread uses `available()`; `remember()` hands it the path stored in `settings.json` before anything asks), `highlight` (colored names for executables, links and archives, after Ubuntu's `ls` colors; not from Explorer), `fonts` (Linux: which installed font beats the bundled DejaVu Sans Condensed, and which cover other scripts; the lookup itself is `system_fonts.rs` through `fontdb`, which reads the fontconfig folders), `properties` (Properties dialog data: folder size walk, size/date wording), `folder_style` (Finder's folder color and emoji/SF Symbol emblem, read and written as Finder stores them, for the Properties dialog's Customize tab, plus Macsplorer's own colors mapped to the nearest Finder tag color; Linux keeps colors only, in `user.macsplorer.folder-color`; `bplist` is the binary plist reader/writer it shares with `listing`), `hls` (the Color dialog's Hue/Sat/Lum, after Microsoft's KB 29240 sample), `xattr` (extended attributes on macOS and Linux: read, set, remove), `associations` (the app chosen with "Always use this program", by file type: a UTI on macOS), `preferences` (the `settings.json` that survives between runs: dark mode, the text size, the column layout, the view mode, the sort, the window size, which `columns` converts to and from JSON, the chosen apps (`associations`), Extract All's "Show extracted files", View > Click to rename (`click_rename`), the addresses of LocalSend devices found (`devices`, latest first, eight at most: the next search asks them and their /24s first), Receive Files (`receive`), and the git location the app found last time; read-modify-write so several windows can share it, unknown keys kept; add new persisted settings there). |
| `crates/macsplorer` | egui/eframe 0.36 app. `app.rs` state + UI, `platform.rs` per-OS open/reveal/eject, wording and font paths, `theme.rs` Windows Classic colors/bevels/fonts/icons/title bar, `scrollbar.rs` classic scroll bars drawn over hidden egui ones, `pasteboard.rs` Cut/Copy files on the macOS pasteboard, `properties.rs` the Properties dialog (own borderless window), `column_dialog.rs` the header menu's More... dialog (Column Settings), `color_dialog.rs` the Win2000 Color dialog (Other... on the Customize tab), `open_with_dialog.rs` the Open With dialog (Choose Program..., with Always use this program), `goto_dialog.rs` the Go To Folder window (Shift+Cmd+G), `shortcuts_dialog.rs` the `?`/F1 shortcut list (repeats the manual's chapter 13), `conflict_dialog.rs` Confirm File/Folder Replace (the paste worker waits on its answer), `extract_dialog.rs` Extract All, `password_dialog.rs` Password Needed for encrypted zips (archive jobs run through `Explorer::run_archive_job`: extract, then open, copy or move into place with `transfer_with`; `archive_open` is set while the open folder is in an archive, and every write is refused there with `READ_ONLY`), `progress_dialog.rs` the Copying.../Moving.../Sending.../Receiving... window with Cancel (follows anything `Tracked`), `network/` Network Neighborhood: the glue to `localsend` and `airlynx-core`, one row each under Network Neighborhood, each expanding on its own with a + and running nothing (scan, sweep) until expanded, and each knowing nothing of the other (`mod.rs`: the Network section of the Folders pane and `Network`, which owns what they share (`Shared`: the status bar message, the progress dialogs) and routes an action to a protocol by the device key (`Service::of`, AirLynx keys start with `airlynx:`); what came in this session (`Shared::received`, not kept between runs), which the right pane lists while Network Neighborhood, a protocol's row or one of its devices is selected (`received_files`, `place`; the app keeps it in `received_shown`, and refuses pastes, new folders and drops there since the list isn't a folder); the root row, the item menu's Send To (which starts both scans for `MENU_SCAN_SECONDS`), the status bar computer's Receive Files menu and the sharing hand `theme::paint_shared` while receiving; `localsend.rs` LocalSend's side: the scan and its search note, devices, sends, introduce, the Receive Files question's dialog and the transfers in; `airlynx.rs` AirLynx's side: this computer's identity and favorites in `airlynx/` in the config dir, the scan (a thread asking every six seconds while the row is expanded), the send worker (files only, folders are skipped and reported), the tooltip that says which favorite is missing, first and last seen times for the session, `quarantine` for received files; `offer_dialog.rs` the Receive Files question, Decline is the default; `device_dialog.rs` a device's Properties (address, port, model or ids, favorites, first and last seen for AirLynx only, in a window laid out like the file Properties General page, using its `pub(crate)` helpers); `receiving.rs` the worker that runs both receivers while `preferences::receive` is on and this process holds `localsend.lock` in the config dir, since every window is a process; the identity is kept in `localsend-identity.pem` there, and it starts the AirLynx receiver beside LocalSend's, and when its ports are taken (the AirLynx app) LocalSend goes on without it, with an `AirUnavailable` note; the app only talks to `Network` and `NetAction`, so the feature comes out with this folder and the crates, and a protocol with its file and the lines in `mod.rs` that route to it), `item_drag.rs` item drags inside the window (drop zones the views register while drawing, drag image, target highlight), `native_drag.rs` drags to and from other apps through AppKit (drop methods added to winit's view, a drag that leaves the window becomes an AppKit session), `icons.rs` icon choice + native icon cache, `native_icon.rs` NSWorkspace icons via objc2, `system_fonts.rs` (Linux) finds an installed Tahoma and script fallbacks; Linux builds embed DejaVu Sans Condensed from `assets/fonts` (never in the macOS binary). |
| `crates/egui-classic-menu` | Standalone egui 0.36 crate for the Windows Classic menu rows: check/radio gutter on the left, submenu arrow gutter on the right, shortcuts left of it, whole-row highlight (`style`, `item`, `checkbox`, `radio`, `submenu_row`, `dot`). Colors come from the `Ui`'s `Visuals`, so it has no dependency on `theme.rs`; `theme::menu_style` and `theme::dot` re-export it. Reusable in other projects, see its README. |
| `crates/localsend` | Standalone crate, the LocalSend protocol v2 (no egui, nothing from this repo, meant to move to its own repo): `discovery` (multicast announce/listen on 224.0.0.167:53317, the announcement also sent as UDP broadcasts to the network's broadcast address and 255.255.255.255, bound to the group address so it works next to a running LocalSend app, plus a `GET /info` sweep of the private networks (by netmask, up to a /16, our /24 first, since devices can share a /16 and sit in different /24s); a `Scan` listens until dropped), `send` (`prepare-upload`, which waits for the receiver to accept, then `upload` per file; folders as `/`-separated paths, modified times in the metadata; `Progress` with Cancel), `receive` (the server: `info`, `register`, `prepare-upload` handed to the caller as an `Offer` to accept or decline, `upload`, `cancel`; one transfer at a time; `clean`/`destinations` make received names safe and new; presence: announces and answers announcements; port 53317 or the next free one), `http` (the little HTTP/1.1 the server needs), `time` (ISO 8601 for the metadata), `protocol` (the JSON), `devices`. Tests include a fake HTTP receiver and our own sender against our own receiver (`tests/`); `--example scan`, `--example send` and `--example receive` try it on the real network. TLS (rustls with ring) is its default `tls` feature; protocol 2.2 receivers demand a client certificate (mutual TLS) and reset the connection otherwise, so `Me::new` makes a self-signed one (rcgen) whose SHA-256 is our fingerprint, and the receiver serves HTTPS with it; `Identity::load_or_generate` keeps it. |
| `crates/airlynx-core` | The AirLynx project's `core`, copied in (from `../airlynx`, a sibling checkout, commit 634b3cb; copy changes back by hand): UDP broadcast discovery on 47801 (`discover`, answers carry whether the device has us as a favorite), `Favorites`/`Profile` (a device id and the trusted devices, in text files), `send_files` and `receive::start` (encrypted TCP on 47800: X25519 key exchange, ChaCha20-Poly1305 records, SHA-256 per file; files only, no folders, no question: a device takes files from its favorites). Pure Rust, so the Linux cross-check needs no C toolchain. |
| `crates/update-check` | Standalone crate, no UI and no macsplorer dependency: asks a web address (given by the caller, kept in the program's settings) whether a newer release exists. Reads GitHub's "latest release" JSON or a plain `{"version","url"}` file; fetches with `curl`, so no TLS stack (and the Linux cross-check needs no C toolchain). Blocks up to 15 s: call from a worker. `Version` compares tags (`v1.10.0`, `-rc.1`). Reusable, see its README. The About box's "Check for Updates" button uses it with `preferences::update_url` (default `DEFAULT_UPDATE_URL`, written to `settings.json` on every save so it can be edited); nothing is ever checked without a click. |
| `tools/make-icon` | Renders `assets/icon.png` + `assets/macsplorer.icns` (tiny-skia). |
| `tools/re` | PDB/disassembly helpers for the Windows analysis (see its README). |
| `scripts/bundle-macos.sh` | Builds `target/release/Macsplorer.app`. |
| `scripts/bundle-linux.sh` | Builds `target/release/macsplorer-linux-<arch>.tar.gz` (binary, `assets/macsplorer.desktop`, icons in the hicolor sizes, `install.sh`). The dock icon on Linux comes from the installed `.desktop` file matching the window's app id (`macsplorer`), not from `with_icon` (winit ignores that on Wayland), so run `install.sh` after unpacking. |
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
- English is the only UI language (no translations): dates, numbers and sort order follow the user's region and language settings (`locale.rs`, `natural`), but every word stays English. `Info.plist` lists `en` as the only localization so Foundation does the same.
- The user-facing name is "Macsplorer" (capital M): window title, About, bundle
  name. Crate, binary, file names and bundle id stay lowercase `macsplorer`.

## Linux port

Cross-checking from a Mac works (`cargo check|clippy --target x86_64-unknown-linux-gnu`), but it
can't run, so Linux behavior is only exercised by the CI job on `ubuntu-latest`. OS differences
live behind `#[cfg(target_os = "macos")]` with a Linux branch next to it: `platform.rs`
(open/reveal/eject, `CMD` tooltip text, Tahoma paths), `trash.rs` (`trash` crate), `places.rs`
(`/proc/self/mounts` volumes), `theme::resize_frame` (borderless-window resize handles),
`xattr` and `folder_style` (folder colors only, drawn as the Win2000 folder in the color).
Linux hides what Nautilus hides (`listing::hidden_list`: a folder's `.hidden` file, and `~` backups in `is_hidden_name`), and takes files dropped from other apps through winit's `dropped_files` (X11 only, no pointer position: `Explorer::drop_files` puts them in the open folder). Not ported yet: Open With (`open_with::SUPPORTED`), native icons, SF Symbol emblems, folder emblems, dragging out of the window and drops on Wayland (`native_drag`).

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
  `comctl32.dll` v6.10 (the SysListView32 that Win2000's Explorer view was built on),
  `zipfldr.dll` (Compressed (zipped) Folders) and `cabview.dll` (Cabinet File Viewer).
  `analysis/` hard-links them under their real names next to their PDBs.
- `symbols/*.pubs.tsv` = recovered public symbols (≈100k functions).
- REA MCP with Ghidra 12.1.4 at `~/opt/ghidra_12.1.4_PUBLIC` (macOS arm64
  decompiler built locally; `GHIDRA_INSTALL_DIR` set in the `rea` server env
  in `~/.claude.json`). Ghidra headless does not load the PDBs: decompile by
  address and resolve names via the `.pubs.tsv`. Opening `shell32.dll` takes
  a few minutes of analysis the first time per session.
- Hopper via REA crashed on these binaries; prefer Ghidra.

## Backlog (ideas raised so far)

- AirLynx devices sit beside LocalSend's in Network Neighborhood (own logo, `assets/airlynx`): send (files only), receive from favorites into Downloads, Add to Favorites in the device menu. Still open: asking remembered addresses directly (discovery is a broadcast, so it stays within one subnet), progress dialog and Cancel for incoming AirLynx files (only the status bar follows them), a Receive Files choice per protocol, sending folders as files.
- LocalSend: Network Neighborhood sends (drop on a device, Send To) and receives (Receive Files, off by default; asks for each offer; into Downloads). Still open: PINs, pinning the device's certificate by its fingerprint, text messages (shown instead of saved as a file), choosing the folder received files go to, auto-accept from known devices, keyboard navigation of the network rows, showing the devices in the right pane when Network Neighborhood is opened.
- Opening a file with Reaview (chosen app or the system default, `open_with::open_args`) passes the window's listing as `open -n -a Reaview file --args --sort name|size|date --asc|--desc [--plain] [--hidden]`, so Reaview's ← / → follow it: `--plain` when "Natural number sorting" is off, `--hidden` when "Hidden files and folders" is on. Only Name, Size and Modified have a sort counterpart; other columns send no `--sort`. Reaview reads its arguments at launch only, hence `-n`. Reaview lists images only, so positions still differ if the folder holds other files.
- Copy / move / delete and undo are in (menu, Edit menu, ⌘X/C/V/Z, Delete, toolbar Undo). Cut/Copy go through the system pasteboard on macOS (`pasteboard.rs`: file URLs plus a private `dev.macsplorer.cut` type on the first item, so separate windows and Finder interoperate; Option+Cmd+V = Move Here; Finder has no cut, Command X leaves no mark on the pasteboard, checked); the Linux port still has an in-memory clipboard. Name conflicts ask with Win2000's Confirm File/Folder Replace. A paste shows Win2000's progress dialog after a second, with Cancel. Drag and drop is in (`dragdrop`, `item_drag.rs`, `native_drag.rs`); still open there: the right-drag menu, dropping on an app, Linux beyond the window. Still open: Shift+Delete, redo.
- Archives: browsing zip and tar is in (`archive`, `docs/archive-findings.md`). Still open: the Packed Size, Ratio, Password, Method and CRC-32 columns, 7z/rar/cab, Extract All's Browse button, writing into a zip (drop, Send To), the Folders pane inside an archive, and drags out of the window on Linux.
- More Win2000 views: Small Icons, Thumbnails, the "Web view" left info panel.
- Keyboard: Shift+F10/menu key context menus and Alt/F10 into the menu bar need keyboard-driven popup menus (egui's can't be); see `docs/file-browser-findings.md` §2.12 "Not done".
- Type column with language names ("Rust Source") and image thumbnails.
- Trace remaining Explorer behaviors listed in `docs/file-browser-findings.md` §4.
- Persist the rest of the settings (View menu options); the dark mode choice, the text size, the view mode, the sort, the window size (saved half a second after a resize ends) and the Details column layout (shown, order, widths, shared by all folders and windows) already are, in `settings.json`; pinning favorites; tabs.
