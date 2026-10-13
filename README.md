<p align="center">
  <img src="assets/icon.png" alt="Macsplorer icon" width="128">
</p>

<h1 align="center">Macsplorer</h1>

<p align="center">
  <strong>The Windows 2000 Explorer you remember, as a native-feeling file browser for macOS.</strong><br>
  Written in Rust. Always-on folder tree, a real address bar, classic bevels and Tahoma.
</p>

<p align="center">
  <img src="docs/window.png" alt="Macsplorer showing a folder with the Folders tree, details view and status bar" width="900">
</p>

Macsplorer is a file browser for people who think better with a folder tree
on the left, the full path on top and a details list in the middle. It looks
like Windows 2000 "Windows Classic", behaves like Explorer where that matters,
and still opens your files with the apps you already use on your Mac.

> **Status: early prototype.** You can browse, search, sort, rename and open
> files. Copy, move, delete and drag and drop are not built yet, so keep
> Finder around for those (see [Not yet](#not-yet)).

## Why use it next to Finder

- **A real folder tree.** The Folders pane stays on screen and follows you:
  the tree expands to and selects the folder you're in. Finder's sidebar is
  a list of shortcuts, and its column view hides the rest of the hierarchy.
- **An address bar you can read and type into.** Every segment of the path is
  a button, each `›` opens a menu of subfolders, and ⌘L turns the bar into a
  text field (`~` works). The full path is always visible.
- **Details view that never gets in the way.** Sortable Name / Size / Type /
  Modified columns, in place rename with Explorer's selection rule (name
  without extension), and no hover highlights jumping around under your mouse.
- **File types you can recognize at a glance.** Every document is the classic
  paper page with a colored emblem from the VS Code "Seti" icon theme: Rust,
  TypeScript, Dockerfile, Makefile, YAML, SQL and hundreds more.
  Apps and anything with a custom Finder icon keep the real macOS icon.
- **Favorites with Finder's glyphs.** Desktop, Downloads, Documents, Pictures,
  Music, Movies, Home and Applications carry their Finder symbol on the
  yellow folder, so you find them the way you already know.
- **Status bar that tells you something.** Object count, total size of the
  selection (or the folder), the free space of the disk you're on and the name
  of your machine, like the original.
- **Never freezes on a big folder.** Listings run on background threads, so a
  huge directory or a slow volume doesn't lock up the window.
- **Hidden files and extensions on your terms.** Toggle both from the View
  menu. File extensions that could make an app pass for a document
  (`Invoice.pdf.app`) always stay visible.
- **Small, fast and open.** A single native Rust binary, no account, no
  telemetry, MIT licensed.
- **Because it looks great.** Gradient caption bar, 3D bevels, classic scroll
  bars, dotted tree lines with +/− boxes. Nothing else on macOS looks like it.

## Install

You need a recent Rust toolchain.

```sh
git clone https://github.com/jbrunemann/macsplorer.git
cd macsplorer
scripts/bundle-macos.sh --open        # builds target/release/Macsplorer.app
```

Or run it straight from the source tree:

```sh
cargo run --release -p macsplorer -- [folder]   # defaults to your home folder
```

Macsplorer uses Tahoma, which ships with macOS, and reads it from the system
fonts at runtime. It isn't bundled.

## Features

- Details view with Name / Date modified / Type / Size columns; click a header to sort, click again to reverse.
- Folders first, natural name sorting (`file2` before `file10`). *Explorer-derived* (`StrCmpLogicalW`); turn it off under View.
- Click, ⌘-click and ⇧-click selection, arrow keys, double-click or Enter to open (files open in their default app).
- Right-click a file for **Open With**, **Show in Finder** and **Rename**.
- **F2 / right-click → Rename**, in place. The initial selection follows Explorer's rule (name without extension, whole name for folders, dotfiles and hidden extensions). *Explorer-derived* (`SHBeginLabelEdit`). Enter or clicking away commits, Esc cancels, `/` can't be typed.
- Back / forward / up / refresh. Right-click back or forward for the history list.
- Breadcrumb address bar: click a segment to go there, `›` for a subfolder menu, click empty space or ⌘L to type a path (`~` works).
- Search box filters the current folder by name.
- **Navigation pane tree** (modeled on Explorer's `CNscTree`): Favorites and Locations roots, folders only, natural-sorted, packages left out. Children load on background threads with a "Loading…" row; chevrons appear only on folders that have subfolders. Click a name to open it, click the chevron or double-click to expand. Arrow keys follow tree conventions (← collapses or goes to the parent, → expands or goes to the first child). Refresh reloads expanded folders.
- **Expand to open folder** (View menu, on by default): the tree expands to and selects the folder you're in, under the most specific root (`~/Documents/x` opens under Documents, not Home).
- Menus: File › Close; Edit › Select All (⌘A), Invert Selection; View › Explorer Bar › Folders, file name extensions, hidden files, expand to open folder, natural sorting, Refresh.
- Toolbar **Folders** button (or the × on the pane) shows and hides the Folders pane.
- Status bar: object count and free disk space (or the selection count), size of the selection (or of the folder), and the machine name.

| Shortcut | Action |
|---|---|
| ⌘[ · ⌥← · ⌫ | Back |
| ⌘] · ⌥→ | Forward |
| ⌘↑ · ⌥↑ | Up |
| ⌘R · F5 | Refresh |
| ⌘L | Edit path |
| F2 | Rename |
| Enter | Open |
| ⌘A | Select all |

## Not yet

File watching (FSEvents, so the list updates live), copy / move / delete,
drag and drop, more views (Large Icons, List), column and window persistence,
pinning favorites, multiple windows and tabs.

## How it's built

Behaviors marked "Explorer-derived" were traced from Windows 10 build 10240
binaries (`ExplorerFrame.dll`, `shell32.dll`, `Windows.Storage.dll`) and are
documented with their evidence in [`docs/`](docs/). The code is written from
those behavioral notes, not copied from decompiled output.

| Crate | Purpose |
|---|---|
| `crates/macsplorer-core` | UI-independent logic: listing, sorting, rename rules, history, breadcrumbs, sidebar places. Unit-tested. |
| `crates/macsplorer` | The app (egui/eframe). |
| `tools/make-icon` | Renders the app icon and `.icns`. |

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo test -p macsplorer -- --ignored   # renders the window offscreen to target/screenshots/window.png
cargo run -p make-icon                  # regenerates assets/icon.png and macsplorer.icns
```

**Look.** A Windows 2000 "Windows Classic" theme
(`crates/macsplorer/src/theme.rs`): system colors, 3D bevels, Tahoma, a
gradient caption bar that replaces the macOS title bar, flat toolbar buttons
that pop out on hover, a dotted-line Folders tree with +/− boxes, and
hand-drawn 16×16 icons. The app icon is a macOS-style squircle in the Windows
2000 desktop blue, holding a classic window and a pixel-art folder.

**File icons.** Files are the Windows 2000 dog-eared paper page. Recognized
types carry a colored emblem from **Seti**, VSCodium's default file icon
theme, resolved like VS Code: exact file name, then the most specific
extension (`spec.ts` before `ts`), then language id. Unrecognized types get
the plain page with gray text lines. App bundles, other packages and items
with a Finder custom icon show the icon macOS shows
(`NSWorkspace.iconForFile`), rendered lazily and cached. Folders are the
yellow Windows 2000 folder, with the Finder SF Symbol on the user folders that
have one.

## Credits and license

MIT licensed. The Seti font, theme map and the extension → language table come
from the VS Code repository (MIT; see `assets/seti/` for licenses). Refresh
them with `scripts/update-seti.py`.
