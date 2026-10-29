<p align="center">
  <img src="assets/icon.png" alt="Macsplorer icon" width="128">
</p>

<h1 align="center">Macsplorer</h1>

<p align="center">
  <strong>The Windows 2000 Explorer you remember, as a native file browser for macOS and Linux.</strong><br>
  Written in Rust. Always-on folder tree, a real address bar, classic bevels and Tahoma font.
</p>

<p align="center">
  <img src="docs/window.png" alt="Macsplorer showing a folder with the Folders tree, details view and status bar" width="900">
</p>

<p align="center">
  <a href="docs/manual/README.md"><strong>User manual</strong></a> ·
  <a href="#install">Install</a> ·
  <a href="https://github.com/jbrunemann/macsplorer/releases/latest">Download</a> ·
  <a href="#for-developers">For developers</a>
</p>

Macsplorer is a file browser for people who think better with a folder tree on the left, the full path on top and a details list in the middle. It looks like Windows 2000 "Windows Classic", behaves like Explorer where that matters, and still opens your files with the apps you already use.

## Why use it

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
- **Files to the phone next to you.** Network Neighborhood lists the devices
  nearby that run LocalSend or AirLynx, each app under its own row. Drag files
  onto a device to send them, and see what came in right in the file list.
- **Small, fast and open.** A single native Rust binary, no account, no
  telemetry, MIT licensed.
- **Because it looks great.** Gradient caption bar, 3D bevels, classic scroll
  bars, dotted tree lines with +/− boxes. Nothing else on macOS looks like it.

New to Macsplorer? Read the **[user manual](docs/manual/README.md)**: it walks through every feature with screenshots, in plain language.

## Install

### macOS

Grab `Macsplorer-macOS.zip` from the
[latest release](https://github.com/jbrunemann/macsplorer/releases/latest),
unzip it and drag `Macsplorer.app` to Applications. It's a universal build
(Apple silicon and Intel, macOS 11 or later).

The app isn't notarized, so the first launch is blocked by Gatekeeper. Either
right-click the app and choose **Open**, or clear the quarantine flag once:

```sh
xattr -dr com.apple.quarantine /Applications/Macsplorer.app
```

### Linux (experimental)

Download `Macsplorer-linux-x86_64.tar.gz` from the
[latest release](https://github.com/jbrunemann/macsplorer/releases/latest),
unpack it and run `./macsplorer [folder]`. To get the dock icon and a launcher
entry, run `./install.sh` from the unpacked folder (`README.txt` inside says
more). It needs a Wayland or X11 desktop with Vulkan or OpenGL, and the usual
`libxkbcommon` and `libwayland` or `libx11` libraries.

Differences from macOS: Delete moves to the freedesktop Trash, Show in File
Manager uses the `FileManager1` D-Bus interface (falling back to `xdg-open`),
Eject uses `gio` or `umount`, volumes come from `/media`, `/run/media` and
`/mnt`, shortcuts use Ctrl instead of ⌘, and the window gets resize handles on
its frame. Like Nautilus, it hides files listed in a folder's `.hidden` file
and `~` backup files. Folder colors work, but not the emblems. **Open With** is
not available yet, apps keep the paper-page icon instead of a native one, and
drag and drop only works inside the window (plus files dropped from other apps
on X11). This port is built and tested in CI but hasn't had much real-world
use, so expect rough edges.

**Fonts.** Macsplorer uses Tahoma and reads it from the system fonts at
runtime, it isn't bundled. macOS ships it. On Linux install it (Wine and the
`msttcorefonts` package have it, also in `~/.local/share/fonts`). Without it
the Linux build uses DejaVu Sans Condensed, which is built into the binary, and
adds installed system fonts for CJK, Arabic, Hebrew and symbols so such file
names don't show as boxes.

## Features

New to Macsplorer? The **[user manual](docs/manual/README.md)** walks through
every feature with screenshots, in plain language.

### Browsing

- **Large Icons**, **List** and **Details** views (View menu). Details shows Name / Size / Modified / Attributes and, from the header's right-click menu, Type, Comment, Created, Accessed, Owner, Group and Branch (Column Settings under More...). Click a header to sort, click again to reverse. Drag a divider to resize a column (Esc cancels), double-click it to fit the contents. The column layout, view mode, sort, text size, dark mode and window size are remembered between runs.
- Large Icons uses 75 px slots with names wrapped under the icon and cut to two lines (the focused item shows its full name), and arrow keys move to the nearest icon in that direction. List fills columns top to bottom with small icons and scrolls sideways (the mouse wheel too); ↑↓ step through the items, ←→ jump a column.
- Folders first, natural name sorting (`file2` before `file10`); turn it off under View.
- Click, ⌘-click and ⇧-click selection, arrow keys, type a few letters to jump to a name, double-click or Enter to open (files open in their default app).
- **Marquee selection**: drag a dotted rectangle from empty space (in Details, anywhere beside a name) to select what it touches. ⌘ toggles items, ⇧ adds to the selection, Esc clears it, and the view scrolls when you drag past its edge.
- **Folders pane** with Favorites and Locations roots. Children load in the background with a "Loading…" row, and chevrons appear only on folders that have subfolders. Click a name to open it, click the chevron or double-click to expand. Arrow keys follow tree conventions (← collapses or goes to the parent, → expands or goes to the first child).
- **Expand to open folder** (View menu, on by default): the tree expands to and selects the folder you're in, under the most specific root (`~/Documents/x` opens under Documents, not Home).
- **Breadcrumb address bar**: click a segment to go there, `›` for a subfolder menu, click empty space or ⌘L to type a path (`~` works).
- Back / forward / up / refresh. Right-click back or forward for the history list.
- Search box filters the current folder by name.
- **Live updates**: the list and the Folders pane follow changes other programs make, within a second, keeping the selection.
- **Dark mode** (View menu), for people who want the classic look after sunset.

### Working with files

- Right-click an item for **Open**, **Open With**, **Show in Finder**, **Open in Terminal** (folders), **Eject** (volumes), **Cut**, **Copy**, **Copy as Path**, **Paste** (into a folder), **Delete**, **Rename** and **Properties**. Right-click empty space for **Paste**, **Refresh**, **Open in Terminal** and **Properties** of the open folder.
- **Cut, Copy and Paste** files and folders (⌘X / ⌘C / ⌘V). Pasting next to an existing name creates "Name - Copy.ext", "Name - Copy (2).ext", as Explorer does. A name that's taken elsewhere asks with Windows 2000's Confirm File Replace (replaced items go to the Trash). Cut items are drawn faded until pasted. A paste that takes more than a second shows the **Copying...** window with the time left and **Cancel**, which leaves no half-copied item behind. On macOS, Cut and Copy use the system pasteboard, so they work with Finder and between Macsplorer windows.
- **Drag and drop** inside the window, into the Folders pane and to and from Finder and other apps (macOS): the same disk moves, another copies, ⌥ copies, ⌘ moves, ⌥⌘ makes a shortcut (a symbolic link "Name - Shortcut"). The dragged items follow the pointer half transparent and the target folder is highlighted. A closed folder in the Folders pane opens when you rest on it. On Linux it works inside the window, with Windows' keys (Shift, Ctrl, Ctrl+Shift).
- **Rename** with F2 or right-click, in place. The initial selection follows Explorer's rule (name without extension, whole name for folders, dotfiles and hidden extensions). Enter or clicking away commits, Esc cancels, `/` can't be typed. Clicking a selected name also starts a rename, after a short pause.
- **Delete** (⌦ forward delete, or ⌘⌫) moves the selection to the Trash.
- **Undo** (toolbar button, Edit menu or ⌘Z) reverses the last rename, move, copy or delete, up to 10 steps back. Undoing a delete restores the items from the Trash.
- **Copy as Path** (⌘⌥C) puts the selected items' quoted full paths on the clipboard.
- **Properties** (⌘I, File menu or right-click): the Windows 2000 General page in its own window, with name (editable, renames on OK or Apply), type (Finder's Kind, e.g. "Microsoft Excel Workbook"), location, size, and for folders the number of files and folders inside, counted in the background. Files also show created, modified and accessed dates, folders their created date. A Customize tab gives folders a color and an emblem, the way Finder stores them.
- **Open in Terminal** starts Terminal on macOS; on Linux `$TERMINAL`, else the first common terminal found. Set `"terminal"` in `settings.json` (an app name such as `iTerm` or `Ghostty` on macOS, a command on Linux) to use another one.
- Dates and numbers follow the system locale (macOS Language & Region settings).

### Network Neighborhood

The **Network** section of the Folders pane sends files to, and takes files from, phones, tablets and computers nearby, over [LocalSend](https://localsend.org) and AirLynx. Network Neighborhood holds one row for each app, and each expands on its own with a +: nothing looks for devices until you open its row, and it stops again when you close it. The two are separate in the code as well (`network/localsend.rs`, `network/airlynx.rs`), so either can be used or removed without the other.

- **Send** by dragging items onto a device, with Send To, or with **Send Selected Items**. LocalSend devices ask their user first; AirLynx devices take files from their favorites without asking (files only, no folders).
- **Receive Files** (off until turned on, in Network Neighborhood's menu or the status bar computer's) takes files into Downloads. A LocalSend offer asks first, with Decline as the default; files from AirLynx favorites arrive without a question. Received files are marked as downloaded.
- Selecting Network Neighborhood, LocalSend, AirLynx or a device lists what was received since the app started in the right pane (the address bar reads `Network Neighborhood/LocalSend`). The files stay where they were saved, and one you delete or move away leaves the list by itself.
- Right-click a device for **Properties**, or for **Add to Favorites** (AirLynx) or **Let Device Find This Computer** (LocalSend).

### Git

Inside a Git working tree the status bar gets a pane with the branch glyph and a [posh-git](https://github.com/dahlbyk/posh-git) style summary (`[main ≡ +0 ~2 -1 | +1 ~1 -0 !]`, same symbols and colors). The **Branch** column (header menu) shows the branch of folders that are working trees. Nothing runs on the UI thread, and nothing is shown when no git is found.

### Look

A Windows 2000 "Windows Classic" theme: system colors, 3D bevels, Tahoma, a gradient caption bar that replaces the macOS title bar, flat toolbar buttons that pop out on hover, classic scroll bars, a dotted-line tree with +/− boxes and hand-drawn 16×16 icons. Documents are the dog-eared paper page with a colored emblem from the Seti icon theme, folders are the yellow Windows 2000 folder, and apps keep their real macOS icon. File names get ls-style colors for executables, links and archives. The menus from File to Help follow the original, with Folders and Views buttons on the toolbar. The status bar shows the object count, free disk space (or the selection count), size of the selection (or of the folder) and the machine name.

### Shortcuts

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
| ⌘X · ⌘C · ⌘V | Cut · Copy · Paste |
| ⌘⌥C | Copy as Path |
| ⌦ · ⌘⌫ | Move to Trash |
| ⌘Z | Undo |
| ⌘I | Properties |

On Linux, ⌘ is Ctrl.

### Not yet

Pinning favorites and tabs. Dragging with the right button (Explorer's
Copy Here / Move Here menu) and dropping onto an app. For file operations: no
redo and no Shift+Delete. Small Icons and Thumbnails views are still to come.

## For developers

### Build from source

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

For Linux, `scripts/bundle-linux.sh` builds `target/release/macsplorer-linux-<arch>.tar.gz`
(the binary, the `.desktop` entry, hicolor icons and `install.sh`). From a Mac,
`scripts/build-linux-docker.sh [linux/amd64]` builds the same tarball in an
Ubuntu 22.04 container (needs Docker), into `target/linux-docker/`. The window's
dock icon on Linux comes from the installed `.desktop` file, so run `install.sh`
after unpacking. `cargo check --target x86_64-unknown-linux-gnu` cross-checks
from a Mac (`rustup target add` first).

### Tests

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo test -p macsplorer -- --ignored   # renders the window offscreen to target/screenshots/*.png
cargo run -p make-icon                  # regenerates assets/icon.png and macsplorer.icns
```

### Project layout

| Path | Purpose |
|---|---|
| `crates/macsplorer-core` | UI-independent logic: listing, sorting, rename rules, copy/move, history, breadcrumbs, sidebar places, the Folders tree model, Seti icon lookup, the Large Icons and List layouts, marquee selection, column header resizing, Git status and the Properties folder size walk. Unit-tested. |
| `crates/macsplorer` | The app (egui/eframe 0.36). |
| `crates/egui-classic-menu` | Standalone egui crate for Windows Classic menu rows. Reusable, see its README. |
| `tools/make-icon` | Renders the app icon and `.icns`. |
| `tools/re` | PDB and disassembly helpers for the Windows analysis. |
| `docs/` | Findings from the Windows binaries, and the user manual in `docs/manual/`. |

`CLAUDE.md` has the longer module-by-module tour.

### Where the behavior comes from

Behaviors that follow Explorer were traced from Windows 10 build 10240
binaries (`ExplorerFrame.dll`, `shell32.dll`, `Windows.Storage.dll`,
`comctl32.dll`) and are documented with their evidence in [`docs/`](docs/).
The code is written from those behavioral notes, not copied from decompiled
output.

| Feature | Traced from |
|---|---|
| Large Icons and List layout, arrow keys, type-ahead | comctl32 `SysListView32` |
| Details column header, divider resize | comctl32 `SysHeader32`, shell32 `CDefView` |
| Marquee selection | comctl32 `CLVSelectionManager::DragSelect` |
| Natural sorting | `StrCmpLogicalW` |
| Rename selection | `SHBeginLabelEdit` |
| Folders pane | `CNscTree` |
| Copy, move, name conflicts | Windows.Storage `CCopyWorkItem` |
| Copying... window timing | shell32 `CProgressDialog` |
| Drop behavior and drag threshold | shell32 `CFSDropTarget`, comctl32 `CheckForDragBeginEx` |
| Properties General page | shell32 `CFilePropSheetPage` |
| Copy as Path | `CCopyAsPathMenu` |
| Focus cycling | `CShellBrowser::_CycleFocus` |

Dark mode, the ls-style name colors, the Git summary and the folder colors are
not from the Windows binaries.

### Look and icons

The Windows 2000 theme lives in `crates/macsplorer/src/theme.rs`. The app icon
is a macOS-style squircle in the Windows 2000 desktop blue, holding a classic
window and a pixel-art folder.

Files are the Windows 2000 dog-eared paper page. Recognized types carry a
colored emblem from **Seti**, VSCodium's default file icon theme, resolved like
VS Code: exact file name, then the most specific extension (`spec.ts` before
`ts`), then language id. Unrecognized types get the plain page with gray text
lines. App bundles, other packages and items with a Finder custom icon show the
icon macOS shows (`NSWorkspace.iconForFile`), rendered lazily and cached.
Folders are the yellow Windows 2000 folder, with the Finder SF Symbol on the
user folders that have one. Folders that are Git working trees are not marked on
their icon; the Branch column shows them.

### Git integration

The summary text and the branch label live in `macsplorer-core::git`, from
posh-git's `GitPrompt.ps1` and `GitUtils.ps1`; the branch glyph is drawn as
vector shapes after Ionicons' `git-branch-outline`, so Linux gets it too.
`git status` runs on a worker thread when a working tree is entered or the
folder is reloaded (Refresh, file operations), and the answer is kept while
moving around inside it. A change in the open folder also asks again. Finding
git happens on a background thread at start: the location is remembered in
`settings.json` as `git`, checked on every start and searched for again if it
moved. The `.git` look-up in each listed folder only runs while the Branch
column is shown. `git::executable` searches `PATH`, Homebrew and `/usr/bin`;
macOS's `/usr/bin/git` stub only counts once the developer tools are installed,
so it never pops up an install prompt.

## Credits and license

MIT licensed. The Seti font, theme map and the extension → language table come
from the VS Code repository (MIT; see `assets/seti/` for licenses). Refresh
them with `scripts/update-seti.py`. The branch glyph is drawn after Ionicons'
`git-branch-outline` (MIT), and the Git status summary follows the format and
counting rules of [posh-git](https://github.com/dahlbyk/posh-git) (MIT); no
code from either is included.

Macsplorer is released under the [MIT License](LICENSE).

Macsplorer is an independent project. It is not affiliated with, endorsed by or
sponsored by Microsoft. Windows, Windows Explorer and Tahoma are trademarks or
registered trademarks of Microsoft Corporation. The Windows 2000 look is
re-created from scratch and the behavior was documented from observation, with
no Microsoft code or artwork included. Tahoma is not bundled; it is loaded from
the system when present.
