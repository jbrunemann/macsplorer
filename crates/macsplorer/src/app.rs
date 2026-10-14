//! The prototype window: toolbar, breadcrumb, navigation pane, item view
//! (Large Icons, List or Details), status bar.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui;
use egui::text::CCursor;
use egui::text_selection::CCursorRange;
use egui::{Align, Key, Layout, RichText, Sense, TextEdit, Ui};
use egui_extras::{Column as TableColumn, TableBuilder};

use macsplorer_core::breadcrumb::crumbs;
use macsplorer_core::columns::{self, Column, ColumnDrag, ColumnSet, HeaderHit};
use macsplorer_core::copy_path;
use macsplorer_core::fileops::{copy_into, move_into};
use macsplorer_core::history::History;
use macsplorer_core::isearch::{self, IncrementalSearch};
use macsplorer_core::layout::{self, IconCell, IconGrid, ListColumns, Nav};
use macsplorer_core::marquee::{self, Marquee};
use macsplorer_core::listing::{Entry, SortSpec, format_byte_size, format_size, free_space, list_dir, sort_entries};
use macsplorer_core::natural::natural_cmp;
use macsplorer_core::places::{computer_name, favorites, is_ejectable, volumes};
use macsplorer_core::rename::{RenameKind, initial_selection, is_invalid_char, rename_no_replace, validate_name};
use macsplorer_core::slow_click::{self, SlowClick};
use macsplorer_core::tree::{FolderTree, Node, NodeKey, Root, Row, Section, load_children};
use macsplorer_core::undo::{Trash, UndoOp, UndoStack};

use crate::column_dialog::{self, ColumnDialog};
use crate::icons::{FileIcons, IconState};
use crate::locale;
use crate::open_with;
use crate::platform;
use crate::properties::{Outcome, Properties};
use crate::scrollbar::{self, Bars};
use crate::theme::{self, Bevel, Icon};
use crate::trash::SystemTrash;

const LIST_ROW_HEIGHT: f32 = 17.0;
const LIST_HEADER_HEIGHT: f32 = 18.0;
const TREE_ROW_HEIGHT: f32 = 18.0;
const TREE_INDENT: f32 = 19.0;

type LoadResult = (PathBuf, bool, Result<Vec<Node>, String>);
/// A details-view listing, tagged with the request it answers.
type ListResult = (u64, Result<Vec<Entry>, String>, Option<u64>);
/// Search-filtered rows and the (entries version, extensions shown, needle) they came from.
type VisibleCache = ((u64, bool, String), Rc<[usize]>);

/// The item view's mode. Windows 2000's View menu also had Small Icons and
/// Thumbnails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewMode {
    LargeIcons,
    List,
    Details,
}

/// Last frame's Large Icons or List layout, for arrow keys handled before
/// the view is drawn.
#[derive(Clone, Copy)]
enum NavLayout {
    Icons { grid: IconGrid, client_height: f32 },
    List { columns: ListColumns, client_width: f32 },
}

struct Settings {
    view: ViewMode,
    show_extensions: bool,
    show_hidden: bool,
    natural_sort: bool,
    /// Explorer's "Expand to open folder" navigation pane option.
    expand_to_open: bool,
    /// The Folders explorer bar (navigation pane) is shown.
    show_folders: bool,
    /// Executables, links and archives get colored names
    /// ([`macsplorer_core::highlight`]).
    color_names: bool,
}

struct RenameState {
    path: PathBuf,
    kind: RenameKind,
    text: String,
    /// The extension left out of `text`, put back on commit.
    hidden_ext: Option<String>,
    /// Selection and focus are applied on the first frame the editor exists.
    needs_init: bool,
}

/// Things the user did this frame. Applied after drawing so the UI code only
/// needs shared borrows of the listing.
enum Action {
    Navigate(PathBuf),
    Open(PathBuf),
    /// Open a file with an app; `None` asks the user to pick one.
    OpenWith(PathBuf, Option<PathBuf>),
    /// A click on an item. `on_label`: a left click on its label text,
    /// which may start a slow-click rename.
    Click { index: usize, toggle: bool, extend: bool, on_label: bool },
    /// Scroll the item view so this entry is visible.
    Reveal(usize),
    /// Characters typed in the item view at `time_ms`.
    TypeAhead { text: String, time_ms: u64 },
    /// A key that ends the typed text (Enter, Esc, Tab, Backspace, navigation keys).
    EndTypeAhead,
    /// Clear the selection (a click on empty space in the view).
    Deselect,
    /// Start a marquee at this content point; `extend` (Cmd or Shift) keeps
    /// the selection.
    MarqueeBegin { origin: (f32, f32), extend: bool },
    BeginRename,
    CommitRename,
    CancelRename,
    Sort(Column),
    /// A header menu click: show or hide a column.
    ToggleColumn(Column),
    /// The header menu's "More...".
    ChooseColumns,
    Back,
    Forward,
    HistoryJump(usize),
    Up,
    Refresh,
    RevealInFinder(PathBuf),
    Eject(PathBuf),
    Cut,
    Copy,
    /// Explorer's "Copy as path": the selection's quoted full paths as text.
    CopyAsPath,
    /// Paste into a folder; `None` is the open folder.
    Paste(Option<PathBuf>),
    Delete,
    TreeSelect(NodeKey),
    TreeToggle(NodeKey),
    TreeExpand(NodeKey),
    TreeCollapse(NodeKey),
    Focus(Pane),
    ToggleFolders,
    SelectAll,
    InvertSelection,
    Undo,
    SetView(ViewMode),
    /// Open the Properties dialog for an item or folder.
    Properties(PathBuf),
}

/// What Cut or Copy remembered. The paths' text is also put on the system
/// clipboard: egui only reports Cmd+V when it holds text, and a paste only
/// counts while that text is still the clipboard's, i.e. nothing newer was
/// copied elsewhere.
struct Clipboard {
    paths: Vec<PathBuf>,
    cut: bool,
    token: String,
}

/// The outcome of a paste, delete or undo that ran on a worker thread.
struct OpDone {
    /// Folders whose contents changed.
    dirs: Vec<PathBuf>,
    /// Items to select once they show up in the open folder.
    select: Vec<PathBuf>,
    error: Option<String>,
    /// What Undo reverses, for the part that got done.
    undo: Option<UndoOp>,
}

/// Which pane receives arrow keys.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Tree,
    List,
}

pub struct Explorer {
    history: History,
    entries: Vec<Entry>,
    /// Bumped whenever `entries` changes or is reordered.
    entries_version: u64,
    /// The search-filtered rows, keyed by what they were computed from.
    visible_cache: Option<VisibleCache>,
    load_error: Option<String>,
    /// Free bytes on the open folder's volume, queried on the listing thread.
    free_space: Option<u64>,
    /// Shown in the status bar's right pane, where Explorer says "My Computer".
    computer_name: String,
    /// The details view lists folders on a worker thread so a huge or slow
    /// folder never freezes the window. Results for anything but the latest
    /// request (`list_request`) are dropped.
    list_tx: Sender<ListResult>,
    list_rx: Receiver<ListResult>,
    list_request: u64,
    listing: bool,
    sort: SortSpec,
    settings: Settings,
    selection: BTreeSet<PathBuf>,
    /// Where Shift+click and Shift+arrow selections start.
    anchor: Option<usize>,
    /// The focused item, which arrow keys move from (`entries` index).
    focus_item: Option<usize>,
    /// An entry to scroll into view on the next frame.
    reveal: Option<usize>,
    /// Letters typed in the item view, which select the item they spell.
    type_ahead: IncrementalSearch,
    nav_layout: Option<NavLayout>,
    /// The widest visible name in the List view, keyed like `visible_cache`.
    widest_name: Option<((u64, bool, String), f32)>,
    /// The rubber band while it's being dragged in the item view.
    marquee: Option<Marquee>,
    /// Details view columns and their widths, shared by every folder until
    /// the window closes. (Explorer kept them per folder.)
    columns: ColumnSet,
    /// The open "More..." dialog.
    column_dialog: Option<ColumnDialog>,
    /// A header divider drag, and whether it started on a divider proper
    /// (where a double-click auto-sizes) rather than a hidden column's.
    column_drag: Option<(ColumnDrag, bool)>,
    /// The details header's left edge on screen last frame, for hit testing
    /// before this frame's layout.
    header_left: f32,
    rename: Option<RenameState>,
    /// A rename armed by clicking the selected item's label, started when
    /// its delay passes unless a double-click or key press cancels it.
    slow_click: SlowClick<PathBuf>,
    /// `Some` while the address bar shows a text field instead of crumbs.
    address_edit: Option<String>,
    address_needs_focus: bool,
    search: String,
    status: Option<String>,
    tree: FolderTree,
    /// Tree folder listings run on worker threads and report back here.
    /// Each result carries the hidden-items setting it was loaded with, so
    /// stale results from before a toggle are dropped.
    load_tx: Sender<LoadResult>,
    load_rx: Receiver<LoadResult>,
    /// `platform::eject` runs on a worker thread; (volume, error text).
    eject_tx: Sender<(PathBuf, Result<(), String>)>,
    eject_rx: Receiver<(PathBuf, Result<(), String>)>,
    clipboard: Option<Clipboard>,
    /// Paste, delete and undo run on worker threads and report back here.
    op_tx: Sender<OpDone>,
    op_rx: Receiver<OpDone>,
    undo: UndoStack,
    pending_select: BTreeSet<PathBuf>,
    focus: Pane,
    /// Set while navigating from a tree click, so the clicked row stays
    /// selected instead of jumping to another root that has the same folder.
    navigating_from_tree: bool,
    icons: FileIcons,
    /// The "Open With" apps for the file whose context menu is open.
    open_with: Option<(PathBuf, Vec<PathBuf>)>,
    /// Open Properties dialogs, each in its own window.
    properties: Vec<Properties>,
    ctx: egui::Context,
    home: Option<PathBuf>,
}

impl Explorer {
    pub fn new(ctx: egui::Context, start: PathBuf) -> Self {
        let (load_tx, load_rx) = channel();
        let (list_tx, list_rx) = channel();
        let (eject_tx, eject_rx) = channel();
        let (op_tx, op_rx) = channel();
        let mut app = Self {
            history: History::new(start),
            entries: Vec::new(),
            entries_version: 0,
            visible_cache: None,
            load_error: None,
            free_space: None,
            computer_name: computer_name().unwrap_or_else(|| "My Computer".into()),
            list_tx,
            list_rx,
            list_request: 0,
            listing: false,
            sort: SortSpec::default(),
            settings: Settings { view: ViewMode::Details, show_extensions: true, show_hidden: false, natural_sort: true, expand_to_open: true, show_folders: true, color_names: true },
            selection: BTreeSet::new(),
            anchor: None,
            focus_item: None,
            reveal: None,
            type_ahead: IncrementalSearch::default(),
            nav_layout: None,
            widest_name: None,
            marquee: None,
            columns: ColumnSet::default(),
            column_dialog: None,
            column_drag: None,
            header_left: 0.0,
            rename: None,
            slow_click: SlowClick::default(),
            address_edit: None,
            address_needs_focus: false,
            search: String::new(),
            status: None,
            tree: FolderTree::new(tree_roots()),
            load_tx,
            load_rx,
            eject_tx,
            eject_rx,
            clipboard: None,
            op_tx,
            op_rx,
            undo: UndoStack::default(),
            pending_select: BTreeSet::new(),
            focus: Pane::List,
            navigating_from_tree: false,
            icons: FileIcons::new(),
            open_with: None,
            properties: Vec::new(),
            ctx,
            home: dirs::home_dir(),
        };
        // Windows' double-click time is also the slow-click rename delay, so a
        // second click before the rename starts always opens the item.
        app.ctx.options_mut(|o| o.input_options.max_double_click_delay = slow_click::DELAY_MS as f64 / 1000.0);
        app.reload();
        app.sync_tree();
        app
    }

    // ---- navigation pane loading ----------------------------------------

    fn load(&self, paths: impl IntoIterator<Item = PathBuf>) {
        let show_hidden = self.settings.show_hidden;
        for path in paths {
            let tx = self.load_tx.clone();
            let ctx = self.ctx.clone();
            thread::spawn(move || {
                let result = load_children(&path, show_hidden);
                if tx.send((path, show_hidden, result)).is_ok() {
                    ctx.request_repaint();
                }
            });
        }
    }

    fn poll_loads(&mut self) {
        while let Ok((path, show_hidden, result)) = self.load_rx.try_recv() {
            if show_hidden == self.settings.show_hidden {
                self.tree.loaded(path, result);
            }
        }
    }

    /// Unmounts and ejects a volume off the UI thread; `poll_ejects` reports.
    fn eject(&mut self, volume: PathBuf) {
        self.status = Some(format!("Ejecting {}...", volume.display()));
        let (tx, ctx) = (self.eject_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = platform::eject(&volume);
            if tx.send((volume, result)).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    fn poll_ejects(&mut self) {
        while let Ok((volume, result)) = self.eject_rx.try_recv() {
            match result {
                Ok(()) => {
                    self.status = None;
                    if self.current().starts_with(&volume) {
                        let target = self.home.clone().unwrap_or_else(|| PathBuf::from("/"));
                        self.navigate(target);
                    }
                    self.tree.set_roots(tree_roots());
                    let loads = self.tree.refresh();
                    self.load(loads);
                    self.reload();
                }
                Err(e) => {
                    self.status = Some(format!("Couldn't eject {}: {e}", volume.display()));
                }
            }
        }
    }

    fn set_clipboard(&mut self, cut: bool) {
        let paths: Vec<PathBuf> = self.entries.iter().filter(|e| self.selection.contains(&e.path)).map(|e| e.path.clone()).collect();
        if paths.is_empty() {
            return;
        }
        let token = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
        self.ctx.copy_text(token.clone());
        self.clipboard = Some(Clipboard { paths, cut, token });
    }

    /// Puts the selection's quoted full paths on the system clipboard as text.
    /// Leaves the Cut/Copy state alone, but the text is no longer the one a
    /// paste checks for, so Cmd+V pastes nothing until the next Copy.
    fn copy_as_path(&mut self) {
        let paths: Vec<PathBuf> = self.entries.iter().filter(|e| self.selection.contains(&e.path)).map(|e| e.path.clone()).collect();
        if paths.is_empty() {
            return;
        }
        self.ctx.copy_text(copy_path::as_path_text(&paths));
    }

    /// Copies or moves the clipboard's items into `dest` on a worker thread.
    fn paste(&mut self, dest: PathBuf) {
        let Some(clip) = self.clipboard.as_ref() else { return };
        let (paths, cut) = (clip.paths.clone(), clip.cut);
        if cut {
            // Like Explorer, a cut can be pasted once.
            self.clipboard = None;
        }
        self.status = Some(format!("{} {} item(s)...", if cut { "Moving" } else { "Copying" }, paths.len()));
        let (tx, ctx) = (self.op_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let mut done = OpDone { dirs: vec![dest.clone()], select: Vec::new(), error: None, undo: None };
            let (mut moved, mut copies) = (Vec::new(), Vec::new());
            for src in &paths {
                let result = if cut { move_into(src, &dest) } else { copy_into(src, &dest) };
                match result {
                    Ok(new) => {
                        done.select.push(new.clone());
                        if cut && let Some(parent) = src.parent() {
                            done.dirs.push(parent.to_path_buf());
                        }
                        if !cut {
                            copies.push(new);
                        } else if new != *src {
                            moved.push((src.clone(), new));
                        }
                    }
                    Err(e) => {
                        let name = src.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        let reason = match e.kind() {
                            std::io::ErrorKind::AlreadyExists => format!("There is already an item named \"{name}\" here."),
                            _ => format!("Couldn't {} \"{name}\": {e}", if cut { "move" } else { "copy" }),
                        };
                        done.error = Some(reason);
                        break;
                    }
                }
            }
            done.undo = Some(if cut { UndoOp::Move(moved) } else { UndoOp::Copy(copies) });
            if tx.send(done).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// Moves the selection to the Trash on a worker thread.
    fn delete_selection(&mut self) {
        let paths: Vec<PathBuf> = self.entries.iter().filter(|e| self.selection.contains(&e.path)).map(|e| e.path.clone()).collect();
        if paths.is_empty() {
            return;
        }
        self.rename = None;
        let dir = self.current().to_path_buf();
        let (tx, ctx) = (self.op_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let mut done = OpDone { dirs: vec![dir], select: Vec::new(), error: None, undo: None };
            let mut trashed = Vec::new();
            for path in &paths {
                match SystemTrash.trash(path) {
                    Ok(t) => trashed.push(t),
                    Err(e) => {
                        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        done.error = Some(format!("Couldn't delete \"{name}\": {e}"));
                        break;
                    }
                }
            }
            done.undo = Some(UndoOp::Delete(trashed));
            if tx.send(done).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// Reverses the newest undoable operation on a worker thread.
    fn undo_last(&mut self) {
        let Some(op) = self.undo.pop() else { return };
        self.rename = None;
        self.status = Some(format!("Undoing {}...", op.label().to_lowercase()));
        let (tx, ctx) = (self.op_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let mut done = OpDone { dirs: op.dirs(), select: Vec::new(), error: None, undo: None };
            match op.undo(&SystemTrash) {
                Ok(back) => done.select = back,
                Err((path, e)) => {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    done.error = Some(match e.kind() {
                        std::io::ErrorKind::AlreadyExists => format!("Couldn't undo: there is already an item named \"{name}\"."),
                        _ => format!("Couldn't undo {} of \"{name}\": {e}", op.label().to_lowercase()),
                    });
                }
            }
            if tx.send(done).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    fn poll_ops(&mut self) {
        while let Ok(done) = self.op_rx.try_recv() {
            self.status = done.error;
            if let Some(op) = done.undo {
                self.undo.push(op);
            }
            let current = self.current().to_path_buf();
            if done.dirs.contains(&current) {
                self.pending_select = done.select.iter().filter(|p| p.parent() == Some(current.as_path())).cloned().collect();
                self.reload();
            }
            for dir in &done.dirs {
                if let Some(path) = self.tree.refresh_path(dir) {
                    self.load([path]);
                }
            }
        }
    }

    /// Points the navigation pane at the open folder.
    fn sync_tree(&mut self) {
        let current = self.current().to_path_buf();
        if self.settings.expand_to_open {
            let loads = self.tree.reveal(&current);
            self.load(loads);
        } else {
            self.tree.select_visible(&current);
        }
    }

    fn current(&self) -> &Path {
        self.history.current()
    }

    /// Lists the open folder on a worker thread; `poll_lists` applies the
    /// result. The current entries stay up until then.
    fn reload(&mut self) {
        self.list_request += 1;
        self.listing = true;
        let (request, dir, show_hidden) = (self.list_request, self.current().to_path_buf(), self.settings.show_hidden);
        let (tx, ctx) = (self.list_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = list_dir(&dir, show_hidden).map_err(|e| format!("Can't open {}: {e}", dir.display()));
            if tx.send((request, result, free_space(&dir))).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    fn poll_lists(&mut self) {
        while let Ok((request, result, free)) = self.list_rx.try_recv() {
            self.listed(request, result, free);
        }
    }

    fn listed(&mut self, request: u64, result: Result<Vec<Entry>, String>, free: Option<u64>) {
        if request != self.list_request {
            return;
        }
        self.listing = false;
        self.free_space = free;
        match result {
            Ok(mut entries) => {
                sort_entries(&mut entries, self.sort);
                self.entries = entries;
                self.load_error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.load_error = Some(e);
            }
        }
        self.entries_version += 1;
        let present: BTreeSet<_> = self.entries.iter().map(|e| &e.path).collect();
        self.selection.retain(|p| present.contains(p));
        let picked: Vec<PathBuf> = std::mem::take(&mut self.pending_select).into_iter().filter(|p| present.contains(p)).collect();
        if !picked.is_empty() {
            self.selection = picked.into_iter().collect();
        }
    }

    /// Blocks until the latest listing has arrived, for tests.
    #[cfg(test)]
    fn finish_listing(&mut self) {
        while self.listing {
            let (request, result, free) = self.list_rx.recv().expect("listing thread");
            self.listed(request, result, free);
        }
    }

    fn resort(&mut self) {
        sort_entries(&mut self.entries, self.sort);
        self.entries_version += 1;
        self.anchor = None;
        self.focus_item = None;
    }

    /// Indices into `entries` that pass the search filter, in display order.
    /// Cached, so a large folder isn't refiltered every frame.
    fn visible(&mut self) -> Rc<[usize]> {
        let key = (self.entries_version, self.settings.show_extensions, self.search.trim().to_lowercase());
        if let Some((k, rows)) = &self.visible_cache
            && *k == key
        {
            return rows.clone();
        }
        let needle = &key.2;
        let rows: Rc<[usize]> = (0..self.entries.len())
            .filter(|&i| {
                needle.is_empty()
                    || self.entries[i].display_name(self.settings.show_extensions).to_lowercase().contains(needle)
            })
            .collect();
        self.visible_cache = Some((key, rows.clone()));
        rows
    }

    fn after_navigation(&mut self) {
        self.selection.clear();
        self.anchor = None;
        self.focus_item = None;
        self.type_ahead.reset();
        self.marquee = None;
        self.rename = None;
        self.slow_click.cancel();
        self.address_edit = None;
        self.search.clear();
        // Don't show the previous folder's items while the new one loads.
        self.entries.clear();
        self.entries_version += 1;
        self.load_error = None;
        self.reload();
        if !std::mem::take(&mut self.navigating_from_tree) {
            self.sync_tree();
        }
    }

    fn navigate(&mut self, path: PathBuf) {
        if !path.is_dir() {
            self.status = Some(format!("{} is not a folder.", path.display()));
            return;
        }
        self.history.push(path);
        self.after_navigation();
    }

    fn open(&mut self, path: PathBuf) {
        let is_folder = self.entries.iter().find(|e| e.path == path).map_or(path.is_dir(), Entry::is_folder);
        if is_folder {
            self.navigate(path);
        } else if let Err(e) = platform::open_default(&path) {
            self.status = Some(format!("Couldn't open {}: {e}", path.display()));
        }
    }

    fn open_with(&mut self, path: PathBuf, app: Option<PathBuf>) {
        let Some(app) = app else {
            // Like Windows' "Choose Program..." dialog. Runs off the UI
            // thread since the picker blocks until the user answers.
            thread::spawn(move || {
                let script = "POSIX path of (choose application with prompt \"Choose the program you want to use to open this file:\" as alias)";
                let Ok(out) = Command::new("osascript").arg("-e").arg(script).output() else { return };
                let app = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                if out.status.success() && !app.is_empty() {
                    let _ = Command::new("open").arg("-a").arg(app).arg(path).spawn();
                }
            });
            return;
        };
        if let Err(e) = Command::new("open").arg("-a").arg(&app).arg(&path).spawn() {
            self.status = Some(format!("Couldn't open {} with {}: {e}", path.display(), open_with::app_name(&app)));
        }
    }

    fn click(&mut self, index: usize, toggle: bool, extend: bool, visible: &[usize]) {
        self.focus_item = Some(index);
        let path = self.entries[index].path.clone();
        let pos = visible.iter().position(|&i| i == index);
        match (extend, self.anchor.and_then(|a| visible.iter().position(|&i| i == a)), pos) {
            (true, Some(a), Some(p)) => {
                if !toggle {
                    self.selection.clear();
                }
                let (lo, hi) = if a <= p { (a, p) } else { (p, a) };
                self.selection.extend(visible[lo..=hi].iter().map(|&i| self.entries[i].path.clone()));
                return;
            }
            _ if toggle => {
                if !self.selection.remove(&path) {
                    self.selection.insert(path);
                }
            }
            _ => {
                self.selection.clear();
                self.selection.insert(path);
            }
        }
        self.anchor = Some(index);
    }

    fn begin_rename(&mut self) {
        let [path] = self.selection.iter().collect::<Vec<_>>()[..] else { return };
        let Some(entry) = self.entries.iter().find(|e| &e.path == path) else { return };
        self.rename = Some(RenameState {
            path: entry.path.clone(),
            kind: entry.kind,
            text: entry.display_name(self.settings.show_extensions).to_owned(),
            hidden_ext: entry.hidden_extension(self.settings.show_extensions).map(str::to_owned),
            needs_init: true,
        });
    }

    fn commit_rename(&mut self) {
        let Some(r) = self.rename.take() else { return };
        let Some(entry) = self.entries.iter().find(|e| e.path == r.path) else { return };
        let mut name = r.text.trim().to_owned();
        // With extensions hidden the user edited the stem only. Put the
        // extension back, as Explorer's SetNameOf does for editing names.
        // Packages too: dropping ".app" would turn an app into a folder.
        if let Some(ext) = &r.hidden_ext {
            name = format!("{name}.{ext}");
        }
        if name == entry.name {
            return;
        }
        if let Some(target) = self.rename_item(&r.path, &name) {
            self.selection = BTreeSet::from([target]);
        }
    }

    /// Renames `path` to `name` in its folder, with undo. Failures go to the
    /// status bar. Returns the new path.
    fn rename_item(&mut self, path: &Path, name: &str) -> Option<PathBuf> {
        if let Err(e) = validate_name(name) {
            self.status = Some(e.to_string());
            return None;
        }
        let target = path.with_file_name(name);
        match rename_no_replace(path, &target) {
            Ok(()) => {
                self.status = None;
                self.undo.push(UndoOp::Rename { from: path.to_path_buf(), to: target.clone() });
                self.reload();
                let loads = self.tree.refresh_path(self.history.current());
                self.load(loads);
                if self.selection.remove(path) {
                    self.selection.insert(target.clone());
                }
                // The open folder (or one above it) was renamed from its Properties.
                if let Ok(rest) = self.current().strip_prefix(path) {
                    let moved = target.join(rest);
                    self.navigate(moved);
                }
                Some(target)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                self.status = Some(format!("There is already an item named \"{name}\" here."));
                None
            }
            Err(e) => {
                self.status = Some(format!("Couldn't rename: {e}"));
                None
            }
        }
    }

    // ---- Properties dialogs --------------------------------------------

    /// What ⌘I and File > Properties show: in the item view the focused item
    /// if it's selected, else the first selected one, else the open folder.
    fn properties_target(&self, in_list: bool) -> PathBuf {
        let focused = self.focus_item.and_then(|i| self.entries.get(i)).map(|e| &e.path).filter(|p| self.selection.contains(*p));
        match focused.or_else(|| self.selection.first()) {
            Some(path) if in_list => path.clone(),
            _ => self.current().to_path_buf(),
        }
    }

    fn open_properties(&mut self, path: PathBuf) {
        let entry = self.entries.iter().find(|e| e.path == path).cloned().unwrap_or_else(|| Properties::folder_entry(&path));
        // The root and mounted volumes can't be renamed from here.
        let editable = path.parent().is_some() && !is_ejectable(&path);
        self.properties.push(Properties::new(&self.ctx, entry, self.settings.show_extensions, editable));
    }

    /// Draws the open dialogs and carries out their OK, Cancel and Apply.
    fn show_properties(&mut self, ctx: &egui::Context) {
        let mut dialogs = std::mem::take(&mut self.properties);
        dialogs.retain_mut(|dialog| match dialog.show(ctx, &mut self.icons) {
            Outcome::Keep => true,
            Outcome::Close => false,
            Outcome::Rename { close } => {
                let path = dialog.path().to_path_buf();
                match self.rename_item(&path, &dialog.new_name()) {
                    Some(target) => {
                        dialog.renamed(target);
                        !close
                    }
                    // Like a failed PSN_APPLY, the dialog stays open.
                    None => true,
                }
            }
        });
        // Dialogs opened while these were drawn come after them.
        dialogs.append(&mut self.properties);
        self.properties = dialogs;
    }

    fn apply(&mut self, actions: Vec<Action>, visible: &[usize]) {
        for action in actions {
            match action {
                Action::Navigate(p) => self.navigate(p),
                Action::Open(p) => {
                    // The second click of a double-click.
                    self.slow_click.cancel();
                    self.open(p);
                }
                Action::OpenWith(p, app) => self.open_with(p, app),
                Action::Click { index, toggle, extend, on_label } => {
                    let path = self.entries[index].path.clone();
                    let click = slow_click::Click {
                        on_label,
                        only_selected: self.selection.len() == 1 && self.selection.contains(&path),
                        view_focused: self.focus == Pane::List,
                        modifiers: toggle || extend,
                    };
                    self.slow_click.click(path, click, now_ms(&self.ctx));
                    self.focus = Pane::List;
                    self.click(index, toggle, extend, visible);
                }
                Action::Reveal(index) => self.reveal = Some(index),
                Action::EndTypeAhead => self.type_ahead.reset(),
                Action::TypeAhead { text, time_ms } => {
                    let show_extensions = self.settings.show_extensions;
                    for c in text.chars() {
                        let focus = self.focus_item.and_then(|f| visible.iter().position(|&v| v == f));
                        let names: Vec<&str> = visible.iter().map(|&i| self.entries[i].display_name(show_extensions)).collect();
                        if let isearch::Outcome::Found(pos) = self.type_ahead.type_char(c, time_ms, &names, focus) {
                            self.click(visible[pos], false, false, visible);
                            self.reveal = Some(visible[pos]);
                        }
                    }
                }
                Action::Deselect => {
                    self.slow_click.cancel();
                    self.focus = Pane::List;
                    self.selection.clear();
                }
                Action::MarqueeBegin { origin, extend } => {
                    self.slow_click.cancel();
                    self.focus = Pane::List;
                    if !extend {
                        self.selection.clear();
                    }
                    self.marquee = Some(Marquee::new(origin));
                }
                Action::BeginRename => self.begin_rename(),
                Action::CommitRename => self.commit_rename(),
                Action::CancelRename => self.rename = None,
                Action::Sort(column) => {
                    if self.sort.column == column {
                        self.sort.ascending = !self.sort.ascending;
                    } else {
                        self.sort.column = column;
                        self.sort.ascending = true;
                    }
                    self.resort();
                }
                Action::ToggleColumn(column) => self.columns.toggle(column),
                Action::ChooseColumns => {
                    if self.column_dialog.is_none() {
                        self.column_dialog = Some(ColumnDialog::new(&self.columns));
                    }
                }
                Action::Back => {
                    if self.history.back().is_some() {
                        self.after_navigation();
                    }
                }
                Action::Forward => {
                    if self.history.forward().is_some() {
                        self.after_navigation();
                    }
                }
                Action::HistoryJump(i) => {
                    if self.history.go_to(i).is_some() {
                        self.after_navigation();
                    }
                }
                Action::Up => {
                    if let Some(parent) = self.current().parent().map(Path::to_path_buf) {
                        self.navigate(parent);
                    }
                }
                Action::Refresh => {
                    self.icons.clear_native();
                    self.tree.set_roots(tree_roots());
                    let loads = self.tree.refresh();
                    self.load(loads);
                    self.reload();
                }
                Action::RevealInFinder(p) => {
                    thread::spawn(move || platform::reveal(&p));
                }
                Action::Eject(p) => self.eject(p),
                Action::Cut => self.set_clipboard(true),
                Action::Copy => self.set_clipboard(false),
                Action::CopyAsPath => self.copy_as_path(),
                Action::Paste(dir) => {
                    let dir = dir.unwrap_or_else(|| self.current().to_path_buf());
                    self.paste(dir);
                }
                Action::Delete => self.delete_selection(),
                Action::TreeSelect(key) => {
                    self.focus = Pane::Tree;
                    let path = key.path.clone();
                    self.tree.select(key);
                    if path != self.current() {
                        self.navigating_from_tree = true;
                        self.navigate(path);
                        self.navigating_from_tree = false;
                    }
                }
                Action::TreeToggle(key) => {
                    let loads = self.tree.toggle(key);
                    self.load(loads);
                }
                Action::TreeExpand(key) => {
                    let loads = self.tree.expand(key);
                    self.load(loads);
                }
                Action::TreeCollapse(key) => self.tree.collapse(&key),
                Action::Focus(pane) => {
                    if pane != Pane::List {
                        self.slow_click.cancel();
                    }
                    self.focus = pane;
                }
                Action::ToggleFolders => {
                    self.settings.show_folders = !self.settings.show_folders;
                    if !self.settings.show_folders {
                        self.focus = Pane::List;
                    }
                }
                Action::Undo => self.undo_last(),
                Action::Properties(path) => self.open_properties(path),
                Action::SetView(mode) => {
                    self.settings.view = mode;
                    // Keep the focused item in sight across the switch.
                    self.reveal = self.focus_item;
                }
                Action::SelectAll => {
                    self.focus = Pane::List;
                    self.selection = visible.iter().map(|&i| self.entries[i].path.clone()).collect();
                }
                Action::InvertSelection => {
                    self.focus = Pane::List;
                    self.selection = visible
                        .iter()
                        .map(|&i| &self.entries[i].path)
                        .filter(|p| !self.selection.contains(*p))
                        .cloned()
                        .collect();
                }
            }
        }
    }

    /// Starts a slow-click rename whose delay has passed, if the item is
    /// still the only selection. Any key press cancels a pending one
    /// (`CLVKeyboardManager::OnKey` calls `CancelPendingEdit`). Like
    /// `CListView::OnTimer`, the item is scrolled into view first.
    fn slow_click_rename(&mut self, ui: &Ui) {
        if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_)))) {
            self.slow_click.cancel();
        }
        let Some(path) = self.slow_click.due(now_ms(ui.ctx())) else { return };
        if self.rename.is_some() || self.focus != Pane::List || self.selection.len() != 1 || !self.selection.contains(&path) {
            return;
        }
        self.reveal = self.entries.iter().position(|e| e.path == path);
        self.begin_rename();
    }

    // ---- keyboard ------------------------------------------------------

    fn keyboard(&self, ui: &Ui, visible: &[usize], actions: &mut Vec<Action>) {
        // Text fields own the keyboard while they are focused.
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return;
        }
        ui.input(|i| {
            let cmd = i.modifiers.command;
            let alt = i.modifiers.alt;
            let in_list = self.focus == Pane::List;
            if cmd && i.key_pressed(Key::A) {
                actions.push(Action::SelectAll);
            }
            if in_list && i.events.iter().any(|e| matches!(e, egui::Event::Cut)) {
                actions.push(Action::Cut);
            }
            // Cmd+Option+C is Finder's "Copy as Pathname"; egui reports it as a Copy too.
            if in_list && cmd && alt && i.key_pressed(Key::C) {
                actions.push(Action::CopyAsPath);
            } else if in_list && i.events.iter().any(|e| matches!(e, egui::Event::Copy)) {
                actions.push(Action::Copy);
            }
            let pasted = i.events.iter().any(|e| matches!(e, egui::Event::Paste(text) if self.clipboard.as_ref().is_some_and(|c| &c.token == text)));
            if pasted {
                actions.push(Action::Paste(None));
            }
            if cmd && !i.modifiers.shift && i.key_pressed(Key::Z) {
                actions.push(Action::Undo);
            }
            if in_list && (i.key_pressed(Key::Delete) || (cmd && i.key_pressed(Key::Backspace))) {
                actions.push(Action::Delete);
            }
            if in_list && i.key_pressed(Key::F2) {
                actions.push(Action::BeginRename);
            }
            // Finder's Get Info key (Explorer's is Alt+Enter): the focused
            // item if it's selected, else the first selected, else the folder.
            if cmd && i.key_pressed(Key::I) {
                actions.push(Action::Properties(self.properties_target(in_list)));
            }
            if in_list && i.key_pressed(Key::Enter) && let Some(p) = self.selection.first() {
                actions.push(Action::Open(p.clone()));
            }
            if (i.key_pressed(Key::Backspace) && !cmd) || (cmd && i.key_pressed(Key::OpenBracket)) || (alt && i.key_pressed(Key::ArrowLeft)) {
                actions.push(Action::Back);
            }
            if (cmd && i.key_pressed(Key::CloseBracket)) || (alt && i.key_pressed(Key::ArrowRight)) {
                actions.push(Action::Forward);
            }
            if (cmd || alt) && i.key_pressed(Key::ArrowUp) {
                actions.push(Action::Up);
            }
            if i.key_pressed(Key::F5) || (cmd && i.key_pressed(Key::R)) {
                actions.push(Action::Refresh);
            }
            if !in_list {
                if !(cmd || alt) {
                    let rows = self.tree.rows();
                    for key in [Key::ArrowDown, Key::ArrowUp, Key::ArrowLeft, Key::ArrowRight] {
                        if i.key_pressed(key) {
                            actions.extend(tree_key_action(&rows, key));
                        }
                    }
                }
                return;
            }
            const KEYS: [(Key, Nav); 8] = [
                (Key::ArrowLeft, Nav::Left),
                (Key::ArrowRight, Nav::Right),
                (Key::ArrowUp, Nav::Up),
                (Key::ArrowDown, Nav::Down),
                (Key::Home, Nav::Home),
                (Key::End, Nav::End),
                (Key::PageUp, Nav::PageUp),
                (Key::PageDown, Nav::PageDown),
            ];
            if [Key::Enter, Key::Escape, Key::Tab, Key::Backspace].into_iter().chain(KEYS.map(|(k, _)| k)).any(|k| i.key_pressed(k)) {
                actions.push(Action::EndTypeAhead);
            }
            if cmd || visible.is_empty() {
                return;
            }
            let text: String = i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
            if !text.is_empty() {
                actions.push(Action::TypeAhead { text, time_ms: (i.time * 1000.0) as u64 });
            }
            if alt {
                return;
            }
            let Some(nav) = KEYS.iter().find(|(k, _)| i.key_pressed(*k)).map(|&(_, n)| n) else { return };
            if let Some(to) = self.navigate_items(nav, visible) {
                actions.push(Action::Click { index: visible[to], toggle: false, extend: i.modifiers.shift, on_label: false });
                actions.push(Action::Reveal(visible[to]));
            }
        });
    }

    /// Where an arrow or page key moves the focused item, as a position in
    /// `visible`. Large Icons and List use the list view's rules
    /// (`layout::IconGrid::navigate`, `layout::ListColumns::navigate`);
    /// Details steps through the rows.
    fn navigate_items(&self, nav: Nav, visible: &[usize]) -> Option<usize> {
        let Some(from) = self.focus_item.and_then(|f| visible.iter().position(|&v| v == f)) else {
            return Some(0);
        };
        let last = visible.len() - 1;
        match (self.settings.view, self.nav_layout) {
            (ViewMode::LargeIcons, Some(NavLayout::Icons { grid, client_height })) if grid.count == visible.len() => {
                grid.navigate(from, nav, client_height)
            }
            (ViewMode::List, Some(NavLayout::List { columns, client_width })) if columns.count == visible.len() => {
                columns.navigate(from, nav, client_width)
            }
            (ViewMode::Details, _) => match nav {
                Nav::Up => Some(from.saturating_sub(1)),
                Nav::Down => Some((from + 1).min(last)),
                Nav::Home => Some(0),
                Nav::End => Some(last),
                _ => None,
            },
            _ => None,
        }
    }

    // ---- menu bar, toolbar and address bar --------------------------------

    fn menu_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Properties").clicked() {
                    actions.push(Action::Properties(self.properties_target(self.focus == Pane::List)));
                }
                ui.separator();
                if ui.button("Close").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = self.undo.peek().map_or_else(|| "Undo".to_owned(), |op| format!("Undo {}", op.label()));
                if ui.add_enabled(self.undo.peek().is_some(), egui::Button::new(undo)).clicked() {
                    actions.push(Action::Undo);
                }
                ui.separator();
                let has_selection = !self.selection.is_empty();
                if ui.add_enabled(has_selection, egui::Button::new("Cut")).clicked() {
                    actions.push(Action::Cut);
                }
                if ui.add_enabled(has_selection, egui::Button::new("Copy")).clicked() {
                    actions.push(Action::Copy);
                }
                if ui.add_enabled(has_selection, egui::Button::new("Copy as Path")).clicked() {
                    actions.push(Action::CopyAsPath);
                }
                if ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste")).clicked() {
                    actions.push(Action::Paste(None));
                }
                ui.separator();
                if ui.add_enabled(has_selection, egui::Button::new("Delete")).clicked() {
                    actions.push(Action::Delete);
                }
                ui.separator();
                if ui.button("Select All").clicked() {
                    actions.push(Action::SelectAll);
                }
                if ui.button("Invert Selection").clicked() {
                    actions.push(Action::InvertSelection);
                }
            });
            ui.menu_button("View", |ui| {
                ui.menu_button("Explorer Bar", |ui| {
                    if ui.checkbox(&mut self.settings.show_folders, "Folders").changed() {
                        ui.close();
                    }
                });
                ui.separator();
                view_choices(ui, self.settings.view, actions);
                ui.separator();
                let mut changed = false;
                changed |= ui.checkbox(&mut self.settings.show_extensions, "File name extensions").changed();
                changed |= ui.checkbox(&mut self.settings.show_hidden, "Hidden files and folders").changed();
                ui.checkbox(&mut self.settings.color_names, "Special files in color");
                if ui.checkbox(&mut self.settings.expand_to_open, "Expand to open folder").changed() && self.settings.expand_to_open {
                    self.sync_tree();
                }
                if ui.checkbox(&mut self.settings.natural_sort, "Natural number sorting").changed() {
                    self.sort.natural = self.settings.natural_sort;
                    self.resort();
                }
                if changed {
                    self.rename = None;
                    self.reload();
                    let loads = self.tree.reset_children();
                    self.load(loads);
                }
                ui.separator();
                if ui.button("Refresh").clicked() {
                    actions.push(Action::Refresh);
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About Macsplorer").clicked() {
                    self.status = Some(format!("Macsplorer {}", env!("CARGO_PKG_VERSION")));
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(2.0);
            let back = theme::tool_button(ui, Icon::Back, Some("Back"), self.history.can_go_back()).on_hover_text(format!("Back ({}[)", platform::CMD));
            if back.clicked() {
                actions.push(Action::Back);
            }
            let arrow = theme::drop_arrow_button(ui, self.history.can_go_back());
            egui::Popup::menu(&arrow).show(|ui| {
                for (i, p) in self.history.back_entries().take(12) {
                    if ui.button(place_label(p)).clicked() {
                        actions.push(Action::HistoryJump(i));
                    }
                }
            });
            let fwd = theme::tool_button(ui, Icon::Forward, None, self.history.can_go_forward()).on_hover_text(format!("Forward ({}])", platform::CMD));
            if fwd.clicked() {
                actions.push(Action::Forward);
            }
            let arrow = theme::drop_arrow_button(ui, self.history.can_go_forward());
            egui::Popup::menu(&arrow).show(|ui| {
                for (i, p) in self.history.forward_entries().take(12) {
                    if ui.button(place_label(p)).clicked() {
                        actions.push(Action::HistoryJump(i));
                    }
                }
            });
            let has_parent = self.current().parent().is_some();
            if theme::tool_button(ui, Icon::Up, None, has_parent).on_hover_text(format!("Up ({}↑)", platform::CMD)).clicked() {
                actions.push(Action::Up);
            }
            theme::etched_vline(ui, 22.0);
            let folders = theme::tool_toggle(ui, Icon::Folders, "Folders", self.settings.show_folders);
            if folders.clicked() {
                actions.push(Action::ToggleFolders);
            }
            if theme::tool_button(ui, Icon::Refresh, Some("Refresh"), true).on_hover_text(format!("Refresh ({}R)", platform::CMD)).clicked() {
                actions.push(Action::Refresh);
            }
            theme::etched_vline(ui, 22.0);
            let undo = self.undo.peek().map_or_else(|| "Undo".to_owned(), |op| format!("Undo {}", op.label()));
            let undo_button = theme::tool_button(ui, Icon::Undo, None, self.undo.peek().is_some());
            if undo_button.on_hover_text(format!("{undo} ({}Z)", platform::CMD)).clicked() {
                actions.push(Action::Undo);
            }
            theme::etched_vline(ui, 22.0);
            // Windows 2000's Views button is all drop-down: a click anywhere opens the menu.
            let views = theme::tool_dropdown(ui, Icon::Views, "Views").on_hover_text("Views");
            egui::Popup::menu(&views).show(|ui| view_choices(ui, self.settings.view, actions));
        });
    }

    fn address_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label("Address");
            let search_w = 170.0;
            let go_w = 44.0;
            let field_w = (ui.available_width() - go_w - search_w - 16.0).max(120.0);
            let (rect, bg) = ui.allocate_exact_size(egui::vec2(field_w, 22.0), Sense::click());
            let inner = theme::sunken_field(ui.painter(), rect);
            let mut field = ui.new_child(egui::UiBuilder::new().max_rect(inner.shrink2(egui::vec2(2.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
            self.address_contents(&mut field, actions);
            if bg.clicked() {
                self.begin_address_edit();
            }
            let go = theme::tool_button(ui, Icon::Go, Some("Go"), true);
            if go.clicked() {
                match &self.address_edit {
                    Some(text) => actions.push(Action::Navigate(expand_tilde(text.trim(), self.home.as_deref()))),
                    None => actions.push(Action::Refresh),
                }
            }
            ui.add_space(4.0);
            let (srect, _) = ui.allocate_exact_size(egui::vec2(search_w, 22.0), Sense::hover());
            let sinner = theme::sunken_field(ui.painter(), srect);
            let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(sinner.shrink2(egui::vec2(3.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
            sui.add(TextEdit::singleline(&mut self.search).hint_text("Search this folder").frame(egui::Frame::NONE).desired_width(f32::INFINITY));
        });
        if ui.input(|i| i.modifiers.command && i.key_pressed(Key::L)) {
            self.begin_address_edit();
        }
    }

    fn begin_address_edit(&mut self) {
        if self.address_edit.is_none() {
            self.address_edit = Some(self.current().display().to_string());
            self.address_needs_focus = true;
        }
    }

    fn address_contents(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let (icon, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
        theme::paint_icon(ui.painter(), icon, Icon::Folder, true);
        if let Some(text) = &mut self.address_edit {
            let id = ui.make_persistent_id("address-edit");
            let resp = ui.add(TextEdit::singleline(text).id(id).desired_width(f32::INFINITY).frame(egui::Frame::NONE));
            if std::mem::take(&mut self.address_needs_focus) {
                resp.request_focus();
                select_all(ui, id, text);
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    actions.push(Action::Navigate(expand_tilde(text.trim(), self.home.as_deref())));
                } else {
                    self.address_edit = None;
                }
            }
            return;
        }
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
        // When the path is too long, the leading crumbs fold into a "«"
        // button that lists them, and the tail stays visible, as in Explorer.
        let all = crumbs(self.current(), self.home.as_deref());
        let widths: Vec<f32> = all.iter().map(|c| crumb_width(ui, &c.label) + crumb_width(ui, "›")).collect();
        let mut room = ui.available_width();
        let mut first = all.len();
        while first > 0 && widths[first - 1] <= room - if first > 1 { crumb_width(ui, "«") } else { 0.0 } {
            room -= widths[first - 1];
            first -= 1;
        }
        // The current folder always stays, even if it has to be clipped.
        let first = first.min(all.len().saturating_sub(1));
        if first > 0 {
            let overflow = crumb_button(ui, "«");
            egui::Popup::menu(&overflow).show(|ui| {
                for crumb in &all[..first] {
                    if ui.button(&crumb.label).clicked() {
                        actions.push(Action::Navigate(crumb.path.clone()));
                    }
                }
            });
        }
        for crumb in &all[first..] {
            if crumb_button(ui, &crumb.label).clicked() {
                actions.push(Action::Navigate(crumb.path.clone()));
            }
            // Explorer's chevron: a menu of the crumb's subfolders.
            let chevron = crumb_button(ui, "›");
            egui::Popup::menu(&chevron).show(|ui| {
                let folders = subfolders(&crumb.path, self.settings.show_hidden);
                if let Some(path) = scrolled_menu(ui, &folders) {
                    actions.push(Action::Navigate(path));
                }
            });
        }
    }

    // ---- folders pane ----------------------------------------------------

    fn folders_pane(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        // Pane caption: "Folders" with a close box, as in Explorer's bar.
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label("Folders");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::close_box(ui).on_hover_text("Close").clicked() {
                    actions.push(Action::ToggleFolders);
                }
            });
        });
        ui.add_space(1.0);
        let rect = ui.available_rect_before_wrap();
        let inner = theme::sunken_field(ui.painter(), rect);
        ui.allocate_rect(rect, Sense::hover());
        let mut tree_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        self.tree_view(&mut tree_ui, actions);
    }

    fn tree_view(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let rows = self.tree.rows();
        let guides = tree_guides(&rows);
        let scroll = self.tree.take_scroll_request();
        let focused = self.focus == Pane::Tree && window_focused(ui);
        let (bars, viewport) = Bars::begin(ui, ui.id().with("tree-bars"), ui.max_rect(), true, TREE_ROW_HEIGHT);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down(Align::Min)));
        let hidden = scrollbar::hidden(&mut ui);
        let out = egui::ScrollArea::both().auto_shrink(false).scroll_bar_visibility(hidden).show(&mut ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut section = None;
            for (row, guide) in rows.iter().zip(&guides) {
                if section != Some(row.section) {
                    section = Some(row.section);
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new(row.section.title()).font(theme::bold(theme::FONT_SIZE)));
                    });
                    ui.add_space(1.0);
                }
                tree_row(ui, &mut self.icons, row, guide, scroll, focused, actions);
            }
        });
        bars.end(&mut ui, &out);
    }

    // ---- item view -----------------------------------------------------

    fn item_view(&mut self, ui: &mut Ui, visible: &[usize], actions: &mut Vec<Action>) {
        match self.settings.view {
            ViewMode::LargeIcons => self.icons_view(ui, visible, actions),
            ViewMode::List => self.list_view(ui, visible, actions),
            ViewMode::Details => {
                self.nav_layout = None;
                self.details(ui, visible, actions);
            }
        }
    }

    fn details(&mut self, ui: &mut Ui, visible: &[usize], actions: &mut Vec<Action>) {
        let rect = ui.available_rect_before_wrap();
        let inner = theme::sunken_field(ui.painter(), rect);
        ui.allocate_rect(rect, Sense::hover());
        // The vertical bar runs beside the column header too, as in a
        // Windows list view, so it is carved out of the whole field. The
        // header scrolls sideways with the rows.
        let (bars, viewport) = Bars::begin(ui, ui.id().with("details-bars"), inner, [true, true], LIST_ROW_HEIGHT);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down(Align::Min)));
        let ui = &mut ui;
        let hidden = scrollbar::hidden(ui);

        if let Some(err) = &self.load_error {
            ui.centered_and_justified(|ui| ui.label(err));
            return;
        }
        let header_rect = egui::Rect::from_min_size(viewport.min, egui::vec2(viewport.width(), LIST_HEADER_HEIGHT));
        let header_hit = self.header_input(ui, header_rect, visible);
        let focused = self.focus == Pane::List && window_focused(ui);
        let can_paste = self.clipboard.is_some();
        let folder = self.current().to_path_buf();
        let cut: &[PathBuf] = self.clipboard.as_ref().filter(|c| c.cut).map_or(&[], |c| &c.paths);
        let reveal = self.reveal.take().and_then(|r| visible.iter().position(|&v| v == r));
        let menu_id = ui.id().with("details-menu");
        let shown = self.columns.visible();
        let Self { entries, selection, rename, sort, settings, search, icons, open_with, listing, marquee, anchor, columns, header_left, .. } = self;
        // Classic list views don't highlight rows on hover.
        ui.visuals_mut().widgets.hovered.bg_fill = theme::color::WINDOW;
        ui.spacing_mut().item_spacing.x = 0.0;
        let pitch = LIST_ROW_HEIGHT + ui.spacing().item_spacing.y;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let (press, mods) = ui.input(|i| (i.pointer.press_origin(), i.modifiers));
        let extend = mods.command || mods.shift;

        // Under the rows, so a click on a row still reaches the row. Not
        // under the header, where a press may grab a divider.
        let bg = ui.interact(viewport.with_min_y(header_rect.bottom()), ui.id().with("details-bg"), Sense::click_and_drag());
        if bg.clicked() {
            if !extend {
                actions.push(Action::Deselect);
            }
            actions.push(Action::Focus(Pane::List));
        }
        bg.context_menu(|ui| background_menu(ui, &folder, can_paste, actions));

        let header = |ui: &mut Ui, column: Column, actions: &mut Vec<Action>| {
            let label = column.caption();
            let right = column.right_aligned();
            let rect = ui.max_rect();
            // Over a divider a press resizes instead of sorting.
            let sense = if header_hit.divider().is_some() { Sense::hover() } else { Sense::click() };
            let resp = ui.interact(rect, ui.id().with(label), sense);
            let p = ui.painter();
            p.rect_filled(rect, 0.0, theme::color::FACE);
            let down = resp.is_pointer_button_down_on();
            theme::bevel(p, rect, if down { Bevel::SunkenThin } else { Bevel::Raised });
            let off = if down { 1.0 } else { 0.0 };
            let font = egui::TextStyle::Body.resolve(ui.style());
            let (pos, align) = if right {
                (egui::pos2(rect.right() - 6.0 + off, rect.center().y + off), egui::Align2::RIGHT_CENTER)
            } else {
                (egui::pos2(rect.left() + 6.0 + off, rect.center().y + off), egui::Align2::LEFT_CENTER)
            };
            let text = p.text(pos, align, label, font, theme::color::TEXT);
            if sort.column == column {
                // A small sort triangle beside the caption.
                let x = if right { text.left() - 10.0 } else { text.right() + 6.0 };
                let c = egui::pos2(x + 3.0, rect.center().y + off);
                let pts = if sort.ascending {
                    vec![egui::pos2(c.x - 3.0, c.y + 2.0), egui::pos2(c.x + 3.0, c.y + 2.0), egui::pos2(c.x, c.y - 2.0)]
                } else {
                    vec![egui::pos2(c.x - 3.0, c.y - 2.0), egui::pos2(c.x + 3.0, c.y - 2.0), egui::pos2(c.x, c.y + 2.0)]
                };
                p.add(egui::Shape::convex_polygon(pts, theme::color::SHADOW, egui::Stroke::NONE));
            }
            if resp.clicked() {
                actions.push(Action::Sort(column));
            }
            resp.context_menu(|ui| header_menu(ui, columns, actions));
        };

        // Where the rows are: row 0's top on screen, the icon's left edge and
        // the Name column's right edge. Taken from a drawn row, for the marquee.
        let mut geometry: Option<(f32, f32, f32)> = None;
        let h_out = egui::ScrollArea::horizontal().id_salt("details-h").auto_shrink(false).scroll_bar_visibility(hidden).show(ui, |ui| {
            let mut table = TableBuilder::new(ui);
            if let Some(row) = reveal {
                table = table.scroll_to_row(row, None);
            }
            // Fixed widths; the last column fills the rest of the header with
            // an empty button, as `Header_Draw` does.
            let mut table = table
                .scroll_bar_visibility(hidden)
                .auto_shrink([false, visible.is_empty()])
                .striped(false)
                .sense(Sense::click_and_drag())
                .cell_layout(Layout::left_to_right(Align::Center));
            for &column in &shown {
                table = table.column(TableColumn::exact(columns.width(column)).clip(true));
            }
            let table = table.column(TableColumn::remainder().at_least(0.0).clip(true)).header(LIST_HEADER_HEIGHT, |mut row| {
                for (i, &column) in shown.iter().enumerate() {
                    row.col(|ui| {
                        if i == 0 {
                            *header_left = ui.max_rect().left();
                        }
                        header(ui, column, actions);
                    });
                }
                row.col(|ui| {
                    let rect = ui.max_rect();
                    ui.painter().rect_filled(rect, 0.0, theme::color::FACE);
                    theme::bevel(ui.painter(), rect, Bevel::Raised);
                    let resp = ui.interact(rect, ui.id().with("header-rest"), Sense::click());
                    resp.context_menu(|ui| header_menu(ui, columns, actions));
                });
            });

            if visible.is_empty() {
                let out = table.body(|_| {});
                if !*listing {
                    empty_message(ui, search);
                }
                return out;
            }

            table.body(|mut body| {
                if marquee.is_some() {
                    let shown = body.ui_mut().clip_rect();
                    edge_scroll(body.ui_mut(), shown, LIST_ROW_HEIGHT, 1);
                }
                body.rows(LIST_ROW_HEIGHT, visible.len(), |mut row| {
                    let pos = row.index();
                    let index = visible[pos];
                    let entry = &entries[index];
                    let selected = selection.contains(&entry.path);
                    // The icon and label, the only part of the row that hits the
                    // item: elsewhere is empty space (`CListView::HandleMouse`).
                    let mut item_rect = egui::Rect::NOTHING;
                    let mut label_rect = egui::Rect::NOTHING;
                    let mut name_column = (0.0, 0.0);

                    for &column in &shown {
                        row.col(|ui| {
                            if column != Column::Name {
                                let text = cell_text(entry, column);
                                if column.right_aligned() {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.add_space(6.0);
                                        ui.label(text);
                                    });
                                } else {
                                    ui.add_space(6.0);
                                    ui.label(text);
                                }
                                return;
                            }
                            ui.add_space(4.0);
                            let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
                            icons.paint(ui, icon_rect, entry, IconState::of(selected, focused, cut.contains(&entry.path)));
                            ui.add_space(3.0);
                            let label = match rename.as_mut().filter(|r| r.path == entry.path) {
                                Some(r) => {
                                    rename_editor(ui, r, false, actions);
                                    ui.min_rect()
                                }
                                None => {
                                    let name = entry.display_name(settings.show_extensions);
                                    selectable_name(ui, name, name_color(entry, settings, selected && focused), selected, focused)
                                }
                            };
                            item_rect = egui::Rect::from_x_y_ranges(icon_rect.left()..=label.right(), ui.max_rect().y_range());
                            label_rect = label;
                            name_column = (icon_rect.left(), ui.max_rect().right());
                        });
                    }
                    row.col(|_| {});

                    let resp = row.response();
                    let top = resp.rect.top() - pos as f32 * pitch;
                    geometry.get_or_insert((top, name_column.0, name_column.1));
                    let on_item = resp.interact_pointer_pos().is_some_and(|p| item_rect.contains(p));
                    if on_item {
                        if resp.clicked() || resp.secondary_clicked() && !selection.contains(&entry.path) {
                            let on_label = resp.clicked() && resp.interact_pointer_pos().is_some_and(|p| label_rect.contains(p));
                            actions.push(Action::Click { index, toggle: mods.command, extend: mods.shift, on_label });
                        }
                        if resp.double_clicked() {
                            actions.push(Action::Open(entry.path.clone()));
                        }
                    } else if resp.clicked() || resp.secondary_clicked() {
                        if resp.clicked() && !extend {
                            actions.push(Action::Deselect);
                        }
                        actions.push(Action::Focus(Pane::List));
                    }
                    if resp.secondary_clicked() {
                        resp.ctx.data_mut(|d| d.insert_temp(menu_id, on_item.then_some(index)));
                    }
                    if resp.drag_started()
                        && let Some(p) = press.filter(|p| !item_rect.contains(*p))
                    {
                        actions.push(Action::MarqueeBegin { origin: (p.x, p.y - top), extend });
                    }
                    resp.context_menu(|ui| match ui.data(|d| d.get_temp::<Option<usize>>(menu_id)).flatten() {
                        Some(i) => item_menu(ui, &entries[i], can_paste, open_with, actions),
                        None => background_menu(ui, &folder, can_paste, actions),
                    });
                });
            })
        });
        let out = &h_out.inner;
        // Content coordinates: screen x, y from row 0's top.
        if let Some((top, icon_left, name_right)) = geometry {
            if bg.drag_started()
                && let Some(p) = press
            {
                actions.push(Action::MarqueeBegin { origin: (p.x, p.y - top), extend });
            }
            if let Some(m) = marquee.as_mut() {
                // Select bounds: the icon and the label's highlight box, as drawn above.
                let candidates = |r: &layout::Rect| {
                    let first = ((r.min.1 / pitch).floor().max(0.0) as usize).min(visible.len());
                    let last = ((r.max.1 / pitch).ceil().max(0.0) as usize).min(visible.len());
                    (first..last)
                        .map(|pos| {
                            let name = entries[visible[pos]].display_name(settings.show_extensions);
                            let w = ui.fonts_mut(|f| f.layout_no_wrap(name.to_owned(), font.clone(), theme::color::TEXT)).size().x;
                            let y = pos as f32 * pitch;
                            let right = (icon_left + 16.0 + 3.0 + w + 2.0).min(name_right);
                            (visible[pos], layout::Rect { min: (icon_left, y), max: (right, y + LIST_ROW_HEIGHT) })
                        })
                        .collect()
                };
                if !drag_marquee(ui, m, egui::pos2(0.0, top), out.inner_rect, candidates, f32::INFINITY, entries, selection, anchor) {
                    *marquee = None;
                }
            }
        }
        bars.end_split(ui, &h_out, out);
    }

    /// Divider drags in the details header, handled before the table is
    /// laid out so a new width shows the same frame. After the comctl32
    /// header control (`docs/file-browser-findings.md` §2.6). Returns what
    /// the pointer is over, a divider for as long as one is dragged.
    fn header_input(&mut self, ui: &mut Ui, header: egui::Rect, visible: &[usize]) -> HeaderHit {
        let left = self.header_left;
        if self.column_drag.is_none() {
            let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|&p| header.contains(p) && ui.rect_contains_pointer(header)) else {
                return HeaderHit::Nowhere;
            };
            let widths = self.columns.visible_widths();
            let hit = columns::hit_test(&widths, header.right() - left, p.x - left);
            let Some(column) = hit.divider() else { return hit };
            // The system has no "split open" cursor; both zones get the same one.
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            if !ui.input(|i| i.pointer.primary_pressed()) {
                return hit;
            }
            self.column_drag = Some((ColumnDrag::new(&widths, column, p.x - left), matches!(hit, HeaderHit::Divider(_))));
        }
        let Some((drag, on_divider)) = self.column_drag else { return HeaderHit::Nowhere };
        // Columns can't change while a divider is held (the menu needs a
        // right click, which cancels the drag), but be safe.
        let Some(&column) = self.columns.visible().get(drag.column) else {
            self.column_drag = None;
            return HeaderHit::Nowhere;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
        // Esc or the right button puts the width back (`Header_WndProc`).
        let cancel = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape) || i.pointer.secondary_pressed());
        if cancel {
            self.columns.set_width(column, drag.original);
            self.column_drag = None;
            return HeaderHit::Divider(drag.column);
        }
        let (down, pos, double) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos(), i.pointer.button_double_clicked(egui::PointerButton::Primary)));
        if let Some(p) = pos {
            self.columns.set_width(column, drag.width_at(p.x - left));
        }
        if !down {
            self.column_drag = None;
            if double && on_divider {
                let width = self.auto_width(ui, column, visible);
                self.columns.set_width(column, width);
            }
        }
        HeaderHit::Divider(drag.column)
    }

    /// The width a divider double-click gives `column`: its widest cell, but
    /// no less than the caption plus 37 px (`SetColumnWidth(LVSCW_AUTOSIZE)`,
    /// `_ValidateMinWidth`). Cell padding is ours, matching how cells draw.
    fn auto_width(&self, ui: &Ui, column: Column, visible: &[usize]) -> f32 {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let width = |text: String| ui.fonts_mut(|f| f.layout_no_wrap(text, font.clone(), theme::color::TEXT).size().x);
        // Caption plus 6 px each side (3 × SM_CXEDGE), plus 25.
        let caption = width(column.caption().into()) + 12.0 + 25.0;
        visible
            .iter()
            .map(|&i| {
                let e = &self.entries[i];
                match column {
                    // 4 + icon 16 + 3, the label's 2 px highlight, 6 to spare.
                    Column::Name => width(e.display_name(self.settings.show_extensions).into()) + 4.0 + 16.0 + 3.0 + 2.0 + 6.0,
                    _ => match cell_text(e, column) {
                        t if t.is_empty() => 0.0,
                        t => width(t) + 12.0,
                    },
                }
            })
            .fold(caption, f32::max)
            .ceil()
    }

    /// Large Icons: 32 px icons in 75 × 75 slots, labels wrapped underneath
    /// (`docs/file-browser-findings.md` §2.5).
    fn icons_view(&mut self, ui: &mut Ui, visible: &[usize], actions: &mut Vec<Action>) {
        let rect = ui.available_rect_before_wrap();
        let inner = theme::sunken_field(ui.painter(), rect);
        ui.allocate_rect(rect, Sense::hover());
        let grid = IconGrid::new(visible.len(), inner.width(), inner.height(), scrollbar::WIDTH);
        self.nav_layout = Some(NavLayout::Icons { grid, client_height: inner.height() });
        // Guess: the arrow-click scroll step wasn't traced.
        let (bars, viewport) = Bars::begin(ui, ui.id().with("icons-bars"), inner, [false, true], LIST_ROW_HEIGHT);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down(Align::Min)));
        let ui = &mut ui;
        let hidden = scrollbar::hidden(ui);
        if let Some(err) = &self.load_error {
            ui.centered_and_justified(|ui| ui.label(err));
            return;
        }
        let focused = self.focus == Pane::List && window_focused(ui);
        let can_paste = self.clipboard.is_some();
        let folder = self.current().to_path_buf();
        let cut: &[PathBuf] = self.clipboard.as_ref().filter(|c| c.cut).map_or(&[], |c| &c.paths);
        let reveal = self.reveal.take().and_then(|r| visible.iter().position(|&v| v == r));
        let font = egui::TextStyle::Body.resolve(ui.style());
        let line_height = ui.fonts_mut(|f| f.row_height(&font));
        let menu_id = ui.id().with("icons-menu");
        let Self { entries, selection, rename, settings, search, icons, open_with, listing, focus_item, marquee, anchor, .. } = self;

        let out = egui::ScrollArea::vertical()
            .id_salt("icons")
            .auto_shrink(false)
            .scroll_bar_visibility(hidden)
            .show_viewport(ui, |ui, view| {
                let origin = ui.max_rect().min;
                ui.set_min_size(egui::vec2(viewport.width(), grid.content_height()));
                if visible.is_empty() {
                    if !*listing {
                        empty_message(ui, search);
                    }
                    return;
                }
                let at = |(x, y): (f32, f32)| origin + egui::vec2(x, y);
                let screen = |r: layout::Rect| egui::Rect::from_min_max(at(r.min), at(r.max));
                if let Some(pos) = reveal {
                    ui.scroll_to_rect(egui::Rect::from_min_size(at(grid.slot(pos)), egui::Vec2::splat(layout::SLOT)), None);
                }
                let shown = egui::Rect::from_min_size(origin + view.min.to_vec2(), view.size());
                if marquee.is_some() {
                    edge_scroll(ui, shown, LIST_ROW_HEIGHT, 1);
                }
                let resp = ui.interact(shown, ui.id().with("icons-hit"), Sense::click_and_drag());
                let content = |p: egui::Pos2| (p.x - origin.x, p.y - origin.y);
                let pointer = resp.interact_pointer_pos().or(resp.hover_pos()).map(content);
                let press = resp.drag_started().then(|| ui.input(|i| i.pointer.press_origin())).flatten().map(content);

                let mut hit = None;
                let mut on_label = false;
                let mut press_hit = false;
                // The focused item's whole label is drawn last, over its neighbors.
                let mut on_top = None;
                for pos in grid.items_in(view.min.y, view.max.y) {
                    let index = visible[pos];
                    let entry = &entries[index];
                    let selected = selection.contains(&entry.path);
                    let unfolded = focused && *focus_item == Some(index);
                    let color = name_color(entry, settings, selected && focused);
                    let galley = icon_label(ui, entry.display_name(settings.show_extensions), &font, color, unfolded);
                    let size = layout::label_size((galley.size().x, galley.size().y), line_height, !unfolded);
                    let cell = IconCell::new(grid.slot(pos), size);
                    icons.paint(ui, screen(cell.image), entry, IconState::of(selected, focused, cut.contains(&entry.path)));
                    if pointer.is_some_and(|p| cell.hit(p)) {
                        hit = Some(pos);
                        on_label = pointer.is_some_and(|p| cell.label.contains(p));
                    }
                    press_hit |= press.is_some_and(|p| cell.hit(p));
                    let renaming = rename.as_ref().is_some_and(|r| r.path == entry.path);
                    if unfolded {
                        on_top = Some((screen(cell.label), galley, selected, renaming));
                    } else if !renaming {
                        paint_label(ui, screen(cell.label), galley, selected, focused);
                    }
                }
                if let Some((label, galley, selected, false)) = on_top {
                    paint_label(ui, label, galley, selected, focused);
                }
                if let Some(r) = rename.as_mut()
                    && let Some(pos) = visible.iter().position(|&i| entries[i].path == r.path)
                {
                    // A wrapping edit box under the icon, as wide as the label area.
                    let slot = at(grid.slot(pos));
                    let top = slot.y + layout::ICON_TOP + layout::ICON + layout::LABEL_GAP;
                    let rect = egui::Rect::from_min_size(egui::pos2(slot.x, top), egui::vec2(layout::SLOT, 3.0 * line_height + 4.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Center)), |ui| {
                        rename_editor(ui, r, true, actions)
                    });
                }
                if let Some(origin) = press
                    && !press_hit
                {
                    let m = ui.input(|i| i.modifiers);
                    actions.push(Action::MarqueeBegin { origin, extend: m.command || m.shift });
                }
                if let Some(m) = marquee.as_mut() {
                    // Select bounds: the image and the label (`LVIR_SELECTBOUNDS`),
                    // laid out as drawn above.
                    let candidates = |r: &layout::Rect| {
                        grid.items_in(r.min.1, r.max.1)
                            .map(|pos| {
                                let index = visible[pos];
                                let unfolded = focused && *focus_item == Some(index);
                                let galley = icon_label(ui, entries[index].display_name(settings.show_extensions), &font, theme::color::TEXT, unfolded);
                                let size = layout::label_size((galley.size().x, galley.size().y), line_height, !unfolded);
                                let cell = IconCell::new(grid.slot(pos), size);
                                (index, cell.image.union(&cell.label))
                            })
                            .collect()
                    };
                    if !drag_marquee(ui, m, origin, shown, candidates, f32::INFINITY, entries, selection, anchor) {
                        *marquee = None;
                    }
                }
                item_clicks(ui, &resp, hit.map(|p| (visible[p], &entries[visible[p]])), on_label, selection, menu_id, actions);
                resp.context_menu(|ui| match ui.data(|d| d.get_temp::<Option<usize>>(menu_id)).flatten() {
                    Some(i) => item_menu(ui, &entries[i], can_paste, open_with, actions),
                    None => background_menu(ui, &folder, can_paste, actions),
                });
            });
        bars.end(ui, &out);
    }

    /// List: 16 px icons with the name to the right, filled top to bottom in
    /// columns as wide as the widest name (`docs/file-browser-findings.md` §2.5).
    fn list_view(&mut self, ui: &mut Ui, visible: &[usize], actions: &mut Vec<Action>) {
        let rect = ui.available_rect_before_wrap();
        let inner = theme::sunken_field(ui.painter(), rect);
        ui.allocate_rect(rect, Sense::hover());
        let font = egui::TextStyle::Body.resolve(ui.style());
        let line_height = ui.fonts_mut(|f| f.row_height(&font));
        let key = (self.entries_version, self.settings.show_extensions, self.search.trim().to_lowercase());
        let widest = match &self.widest_name {
            Some((k, w)) if *k == key => *w,
            _ => {
                let w = visible
                    .iter()
                    .map(|&i| {
                        let name = self.entries[i].display_name(self.settings.show_extensions).to_owned();
                        ui.fonts_mut(|f| f.layout_no_wrap(name, font.clone(), theme::color::TEXT).size().x)
                    })
                    .fold(0.0, f32::max);
                self.widest_name = Some((key, w));
                w
            }
        };
        let row_height = layout::list_row_height(line_height);
        let columns = ListColumns::new(visible.len(), layout::list_column_width(widest), row_height, inner.width(), inner.height(), scrollbar::WIDTH);
        self.nav_layout = Some(NavLayout::List { columns, client_width: inner.width() });
        // Guess: an arrow click scrolls one column.
        let (bars, viewport) = Bars::begin(ui, ui.id().with("list-bars"), inner, [true, false], columns.column_width);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down(Align::Min)));
        let ui = &mut ui;
        let hidden = scrollbar::hidden(ui);
        if let Some(err) = &self.load_error {
            ui.centered_and_justified(|ui| ui.label(err));
            return;
        }
        let focused = self.focus == Pane::List && window_focused(ui);
        let can_paste = self.clipboard.is_some();
        let folder = self.current().to_path_buf();
        let cut: &[PathBuf] = self.clipboard.as_ref().filter(|c| c.cut).map_or(&[], |c| &c.paths);
        let reveal = self.reveal.take().and_then(|r| visible.iter().position(|&v| v == r));
        let menu_id = ui.id().with("list-menu");
        let Self { entries, selection, rename, settings, search, icons, open_with, listing, marquee, anchor, .. } = self;
        // An item's icon rect and label box (text plus `SM_CXEDGE` each side,
        // within the column), in content coordinates. Unverified: that the
        // label box hugs the text as in Large Icons (`CLVListView::v_ItemHitTest`
        // wasn't traced).
        let item_rects = |ui: &Ui, pos: usize| {
            let (x, y) = columns.cell(pos);
            let icon = layout::Rect::from_min_size((x, y + (row_height - layout::SMALL_ICON) / 2.0), (layout::SMALL_ICON, layout::SMALL_ICON));
            let name = entries[visible[pos]].display_name(settings.show_extensions).to_owned();
            let text = ui.fonts_mut(|f| f.layout_no_wrap(name, font.clone(), theme::color::TEXT).size().x);
            let width = (text + 2.0 * layout::LABEL_PAD_X).min(columns.column_width - layout::SMALL_ICON);
            (icon, layout::Rect::from_min_size((x + layout::SMALL_ICON, y), (width, row_height)))
        };

        let out = egui::ScrollArea::horizontal()
            .id_salt("list")
            .auto_shrink(false)
            .scroll_bar_visibility(hidden)
            .show_viewport(ui, |ui, view| {
                let origin = ui.max_rect().min;
                ui.set_min_size(egui::vec2(columns.content_width(), viewport.height()));
                if visible.is_empty() {
                    if !*listing {
                        empty_message(ui, search);
                    }
                    return;
                }
                let at = |(x, y): (f32, f32)| origin + egui::vec2(x, y);
                let screen = |r: layout::Rect| egui::Rect::from_min_max(at(r.min), at(r.max));
                if let Some(pos) = reveal {
                    let cell = egui::Rect::from_min_size(at(columns.cell(pos)), egui::vec2(columns.column_width, row_height));
                    ui.scroll_to_rect(cell, None);
                }
                let shown = egui::Rect::from_min_size(origin + view.min.to_vec2(), view.size());
                if marquee.is_some() {
                    edge_scroll(ui, shown, columns.column_width, 0);
                }
                let resp = ui.interact(shown, ui.id().with("list-hit"), Sense::click_and_drag());
                let content = |p: egui::Pos2| (p.x - origin.x, p.y - origin.y);
                let pointer = resp.interact_pointer_pos().or(resp.hover_pos()).map(content);
                let press = resp.drag_started().then(|| ui.input(|i| i.pointer.press_origin())).flatten().map(content);

                let mut hit = None;
                let mut on_label = false;
                let mut press_hit = false;
                for pos in columns.items_in(view.min.x, view.max.x) {
                    let index = visible[pos];
                    let entry = &entries[index];
                    let selected = selection.contains(&entry.path);
                    let (icon, label) = item_rects(ui, pos);
                    icons.paint(ui, screen(icon), entry, IconState::of(selected, focused, cut.contains(&entry.path)));
                    let hits = |p: (f32, f32)| icon.contains(p) || label.contains(p);
                    if pointer.is_some_and(hits) {
                        hit = Some(pos);
                        on_label = pointer.is_some_and(|p| label.contains(p));
                    }
                    press_hit |= press.is_some_and(hits);
                    if let Some(r) = rename.as_mut().filter(|r| r.path == entry.path) {
                        let (x, y) = columns.cell(pos);
                        let area = layout::Rect::from_min_size((x + layout::SMALL_ICON, y), (columns.column_width - layout::SMALL_ICON, row_height));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(screen(area)).layout(Layout::left_to_right(Align::Center)), |ui| {
                            rename_editor(ui, r, false, actions)
                        });
                        continue;
                    }
                    let color = name_color(entry, settings, selected && focused);
                    let name = entry.display_name(settings.show_extensions).to_owned();
                    let galley = ui.fonts_mut(|f| f.layout_no_wrap(name, font.clone(), color));
                    paint_label(ui, screen(label), galley, selected, focused);
                }
                if let Some(origin) = press
                    && !press_hit
                {
                    let m = ui.input(|i| i.modifiers);
                    actions.push(Action::MarqueeBegin { origin, extend: m.command || m.shift });
                }
                if let Some(m) = marquee.as_mut() {
                    // Select bounds: the icon and the label box.
                    let candidates = |r: &layout::Rect| {
                        columns
                            .items_in(r.min.0, r.max.0)
                            .map(|pos| {
                                let (icon, label) = item_rects(ui, pos);
                                (visible[pos], icon.union(&label))
                            })
                            .collect()
                    };
                    if !drag_marquee(ui, m, origin, shown, candidates, layout::SMALL_ICON, entries, selection, anchor) {
                        *marquee = None;
                    }
                }
                item_clicks(ui, &resp, hit.map(|p| (visible[p], &entries[visible[p]])), on_label, selection, menu_id, actions);
                resp.context_menu(|ui| match ui.data(|d| d.get_temp::<Option<usize>>(menu_id)).flatten() {
                    Some(i) => item_menu(ui, &entries[i], can_paste, open_with, actions),
                    None => background_menu(ui, &folder, can_paste, actions),
                });
            });
        bars.end(ui, &out);
    }

    fn status_bar(&self, ui: &mut Ui, visible: &[usize]) {
        let selected: Vec<&Entry> = self.entries.iter().filter(|e| self.selection.contains(&e.path)).collect();
        let (objects, bytes) = if selected.is_empty() {
            let n = visible.len();
            let bytes: u64 = visible.iter().filter_map(|&i| self.entries[i].size).sum();
            // Explorer appends the volume's free space only when nothing is selected.
            let free = self.free_space.map(|f| format!(" (Disk free space: {})", format_byte_size(f))).unwrap_or_default();
            (format!("{n} object(s){free}"), bytes)
        } else {
            (format!("{} object(s) selected", selected.len()), selected.iter().filter_map(|e| e.size).sum())
        };
        let text = self.status.clone().unwrap_or(objects);
        let size = format_byte_size(bytes);

        let full = ui.available_rect_before_wrap();
        let rect = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), 20.0));
        ui.allocate_rect(rect, Sense::hover());
        let size_w = 90.0;
        let place_w = 150.0;
        let right = egui::Rect::from_min_max(egui::pos2(rect.right() - place_w, rect.top()), rect.max);
        let mid = egui::Rect::from_min_max(egui::pos2(right.left() - 2.0 - size_w, rect.top()), egui::pos2(right.left() - 2.0, rect.bottom()));
        let left = egui::Rect::from_min_max(rect.min, egui::pos2(mid.left() - 2.0, rect.bottom()));
        let p = ui.painter();
        let font = egui::TextStyle::Body.resolve(ui.style());
        for r in [left, mid, right] {
            theme::bevel(p, r, Bevel::SunkenThin);
        }
        let color = if self.status.is_some() { egui::Color32::from_rgb(0x80, 0, 0) } else { theme::color::TEXT };
        p.with_clip_rect(left.shrink(2.0)).text(egui::pos2(left.left() + 4.0, left.center().y), egui::Align2::LEFT_CENTER, text, font.clone(), color);
        p.text(egui::pos2(mid.right() - 4.0, mid.center().y), egui::Align2::RIGHT_CENTER, size, font.clone(), theme::color::TEXT);
        let icon = egui::Rect::from_min_size(egui::pos2(right.left() + 4.0, right.center().y - 8.0), egui::vec2(16.0, 16.0));
        theme::paint_icon(p, icon, Icon::Drive, true);
        p.with_clip_rect(right.shrink(2.0)).text(
            egui::pos2(icon.right() + 4.0, right.center().y),
            egui::Align2::LEFT_CENTER,
            &self.computer_name,
            font,
            theme::color::TEXT,
        );
    }
}

impl eframe::App for Explorer {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.poll_loads();
        self.poll_lists();
        self.poll_ejects();
        self.poll_ops();
        self.icons.begin_frame();
        let visible = self.visible();
        self.slow_click_rename(ui);
        let mut actions = Vec::new();
        self.keyboard(ui, &visible, &mut actions);

        // The window frame: a raised 3D border around everything.
        let outer = ui.max_rect();
        ui.painter().rect_filled(outer, 0.0, theme::color::FACE);
        theme::bevel(ui.painter(), outer, Bevel::Raised);
        theme::resize_frame(ui, outer);
        let mut root = ui.new_child(egui::UiBuilder::new().max_rect(outer.shrink(4.0)).layout(Layout::top_down(Align::Min)));
        let ui = &mut root;
        let plain = egui::Frame::NONE.fill(theme::color::FACE);

        egui::Panel::top("chrome").frame(plain).show_separator_line(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            theme::title_bar(ui, &place_label(self.current()));
            self.menu_bar(ui, &mut actions);
            theme::etched_hline(ui);
            self.toolbar(ui, &mut actions);
            theme::etched_hline(ui);
            self.address_bar(ui, &mut actions);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").frame(plain).show_separator_line(false).show(ui, |ui| {
            ui.add_space(2.0);
            self.status_bar(ui, &visible);
        });
        if self.settings.show_folders {
            egui::Panel::left("folders")
                .frame(plain)
                .show_separator_line(false)
                .resizable(true)
                .default_size(220.0)
                .size_range(120.0..=600.0)
                .show(ui, |ui| self.folders_pane(ui, &mut actions));
        }
        egui::CentralPanel::default()
            .frame(plain.inner_margin(egui::Margin { left: if self.settings.show_folders { 3 } else { 0 }, ..Default::default() }))
            .show(ui, |ui| self.item_view(ui, &visible, &mut actions));

        self.apply(actions, &visible);
        if let Some(ms) = self.slow_click.wait_ms(now_ms(ui.ctx())) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(ms));
        }
        let ctx = ui.ctx().clone();
        self.show_properties(&ctx);
        if let Some(dialog) = &mut self.column_dialog {
            match dialog.show(&ctx) {
                column_dialog::Outcome::Keep => {}
                column_dialog::Outcome::Cancel => self.column_dialog = None,
                column_dialog::Outcome::Apply => {
                    dialog.apply(&mut self.columns);
                    self.column_dialog = None;
                }
            }
        }
    }
}

/// The in-place rename box: white with a thin black frame, like a classic
/// list view label edit. Initial selection comes from the Explorer rule.
/// The "Open With" submenu: the file's apps, then "Choose Program...".
/// The app list is looked up once per file and kept while the menu is open.
fn open_with_menu(ui: &mut Ui, path: &Path, cache: &mut Option<(PathBuf, Vec<PathBuf>)>, actions: &mut Vec<Action>) {
    ui.menu_button("Open With", |ui| {
        if cache.as_ref().is_none_or(|(p, _)| p != path) {
            *cache = Some((path.to_path_buf(), open_with::apps_for(path)));
        }
        let apps = cache.as_ref().map_or(&[][..], |(_, apps)| apps.as_slice());
        for app in apps {
            if ui.button(open_with::app_name(app)).clicked() {
                actions.push(Action::OpenWith(path.to_path_buf(), Some(app.clone())));
            }
        }
        if !apps.is_empty() {
            ui.separator();
        }
        if ui.button("Choose Program...").clicked() {
            actions.push(Action::OpenWith(path.to_path_buf(), None));
        }
    });
}

/// The context menu for an item.
fn item_menu(ui: &mut Ui, entry: &Entry, can_paste: bool, open_with: &mut Option<(PathBuf, Vec<PathBuf>)>, actions: &mut Vec<Action>) {
    if ui.button(RichText::new("Open").font(theme::bold(theme::FONT_SIZE))).clicked() {
        actions.push(Action::Open(entry.path.clone()));
    }
    if open_with::SUPPORTED && entry.kind == RenameKind::File {
        open_with_menu(ui, &entry.path, open_with, actions);
    }
    if ui.button(platform::REVEAL_LABEL).clicked() {
        actions.push(Action::RevealInFinder(entry.path.clone()));
    }
    if is_ejectable(&entry.path) && ui.button("Eject").clicked() {
        actions.push(Action::Eject(entry.path.clone()));
    }
    ui.separator();
    if ui.button("Cut").clicked() {
        actions.push(Action::Cut);
    }
    if ui.button("Copy").clicked() {
        actions.push(Action::Copy);
    }
    if ui.button("Copy as Path").clicked() {
        actions.push(Action::CopyAsPath);
    }
    if entry.kind == RenameKind::Folder && ui.add_enabled(can_paste, egui::Button::new("Paste")).clicked() {
        actions.push(Action::Paste(Some(entry.path.clone())));
    }
    ui.separator();
    if ui.button("Delete").clicked() {
        actions.push(Action::Delete);
    }
    if ui.button("Rename").clicked() {
        actions.push(Action::BeginRename);
    }
    ui.separator();
    if ui.button("Properties").clicked() {
        actions.push(Action::Properties(entry.path.clone()));
    }
}

/// The context menu for empty space in the item view.
fn background_menu(ui: &mut Ui, folder: &Path, can_paste: bool, actions: &mut Vec<Action>) {
    if ui.add_enabled(can_paste, egui::Button::new("Paste")).clicked() {
        actions.push(Action::Paste(None));
    }
    ui.separator();
    if ui.button("Refresh").clicked() {
        actions.push(Action::Refresh);
    }
    ui.separator();
    if ui.button("Properties").clicked() {
        actions.push(Action::Properties(folder.to_path_buf()));
    }
}

fn empty_message(ui: &mut Ui, search: &str) {
    let msg = if search.is_empty() { "This folder is empty." } else { "No items match your search." };
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(msg).color(theme::color::GRAY_TEXT));
    });
}

/// Turns clicks on the Large Icons or List view into actions. `hit` is the item
/// under the pointer (`entries` index) and `on_label` whether the pointer is
/// on its label rather than its icon. A click on empty space clears the selection; a
/// right-click remembers its target under `menu_id` for the context menu.
fn item_clicks(
    ui: &Ui,
    resp: &egui::Response,
    hit: Option<(usize, &Entry)>,
    on_label: bool,
    selection: &BTreeSet<PathBuf>,
    menu_id: egui::Id,
    actions: &mut Vec<Action>,
) {
    let m = ui.input(|i| i.modifiers);
    match hit {
        Some((index, entry)) => {
            if resp.clicked() || resp.secondary_clicked() && !selection.contains(&entry.path) {
                actions.push(Action::Click { index, toggle: m.command, extend: m.shift, on_label: on_label && resp.clicked() });
            }
            if resp.double_clicked() {
                actions.push(Action::Open(entry.path.clone()));
            }
        }
        None => {
            if resp.clicked() && !m.command && !m.shift {
                actions.push(Action::Deselect);
            }
            if resp.clicked() || resp.secondary_clicked() {
                actions.push(Action::Focus(Pane::List));
            }
        }
    }
    if resp.secondary_clicked() {
        ui.data_mut(|d| d.insert_temp(menu_id, hit.map(|(index, _)| index)));
    }
}

/// One frame of a marquee drag (`CLVSelectionManager::DragSelect`): moves
/// the band to the pointer, clamped to `shown` (the visible part of the
/// view, on screen), and selects what its edge crossed. `origin` is the
/// screen position of the content's (0, 0). `candidates` lists the items
/// (`entries` index and select bounds, in content coordinates) that may lie
/// in the given rectangle; `max_inset_x` goes to `marquee::hit_bounds`.
/// Paints the band. Returns false once the button is
/// up, or after Escape, which clears the selection.
#[allow(clippy::too_many_arguments)]
fn drag_marquee(
    ui: &Ui,
    m: &mut Marquee,
    origin: egui::Pos2,
    shown: egui::Rect,
    candidates: impl FnOnce(&layout::Rect) -> Vec<(usize, layout::Rect)>,
    max_inset_x: f32,
    entries: &[Entry],
    selection: &mut BTreeSet<PathBuf>,
    anchor: &mut Option<usize>,
) -> bool {
    let (pointer, down, escape, mods) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.key_pressed(Key::Escape), i.modifiers));
    if escape {
        selection.clear();
        return false;
    }
    let content = |p: egui::Pos2| (p.x - origin.x, p.y - origin.y);
    let clip = layout::Rect { min: content(shown.min), max: content(shown.max) };
    if let Some(p) = pointer {
        let old = m.drag_to(content(p), clip);
        if old != m.rect {
            let mut inside = Vec::new();
            for (index, bounds) in candidates(&old.union(&m.rect)) {
                let bounds = marquee::hit_bounds(bounds, max_inset_x);
                let path = &entries[index].path;
                match m.item_state(&old, &bounds, selection.contains(path), mods.command) {
                    Some(true) => _ = selection.insert(path.clone()),
                    Some(false) => _ = selection.remove(path),
                    None => {}
                }
                if m.rect.intersects(&bounds) {
                    inside.push((index, bounds));
                }
            }
            if !mods.command && !mods.shift
                && let Some(a) = marquee::anchor(content(p), inside)
            {
                *anchor = Some(a);
            }
        }
    }
    let at = |(x, y): (f32, f32)| origin + egui::vec2(x, y);
    theme::focus_rect(&ui.painter().with_clip_rect(shown), egui::Rect::from_min_max(at(m.rect.min), at(m.rect.max)));
    down
}

/// While a marquee is dragged with the pointer at or past the top or bottom
/// of `shown` (`axis` 1), or its left or right for the sideways List view
/// (`axis` 0), scrolls the enclosing scroll area toward it
/// (`CLVScrollingManager::ShouldScroll`). Guess: the speed, about 20
/// `line`s a second, wasn't traced; nor was the List view's horizontal case.
fn edge_scroll(ui: &Ui, shown: egui::Rect, line: f32, axis: usize) {
    let (pointer, dt) = ui.input(|i| (i.pointer.latest_pos(), i.stable_dt.min(0.1)));
    let Some(p) = pointer else { return };
    let dir = if p[axis] >= shown.max[axis] - 1.0 {
        -1.0
    } else if p[axis] <= shown.min[axis] {
        1.0
    } else {
        return;
    };
    let mut delta = egui::Vec2::ZERO;
    delta[axis] = dir * 20.0 * line * dt;
    ui.scroll_with_delta_animation(delta, egui::style::ScrollAnimation::none());
    ui.ctx().request_repaint();
}

/// Lays out a Large Icons label: centered, wrapped in `layout::LABEL_WRAP`,
/// cut to `layout::FOLDED_LINES` lines with "…" unless `unfolded`.
fn icon_label(ui: &Ui, text: &str, font: &egui::FontId, color: egui::Color32, unfolded: bool) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), color, layout::LABEL_WRAP);
    job.halign = Align::Center;
    if !unfolded {
        job.wrap.max_rows = layout::FOLDED_LINES;
        job.wrap.overflow_character = Some('…');
    }
    ui.fonts_mut(|f| f.layout_job(job))
}

/// An item label's text color: white on the focused selection, else the
/// special file color (kept on the unfocused gray selection, like Explorer's
/// compressed-file blue), else black.
fn name_color(entry: &Entry, settings: &Settings, highlighted: bool) -> egui::Color32 {
    use macsplorer_core::highlight::{Highlight, classify};
    if highlighted {
        return theme::color::SELECTION_TEXT;
    }
    match classify(entry).filter(|_| settings.color_names) {
        Some(Highlight::Executable) => theme::color::EXECUTABLE,
        Some(Highlight::Link) => theme::color::LINK,
        Some(Highlight::Archive | Highlight::BrokenLink) => theme::color::ARCHIVE,
        None => theme::color::TEXT,
    }
}

/// Paints an item label in its box: the text alone, or on the selection
/// color (button face while the view isn't focused).
fn paint_label(ui: &Ui, rect: egui::Rect, galley: std::sync::Arc<egui::Galley>, selected: bool, focused: bool) {
    if selected {
        ui.painter().rect_filled(rect, 0.0, if focused { theme::color::SELECTION } else { theme::color::FACE });
    }
    // Centered galleys are laid out around x = 0; left-aligned ones start there.
    let x = if galley.job.halign == Align::Center { rect.center().x } else { rect.left() + layout::LABEL_PAD_X };
    let y = rect.top() + (rect.height() - galley.size().y) / 2.0;
    ui.painter().with_clip_rect(rect).galley(egui::pos2(x, y), galley, theme::color::TEXT);
}

/// `wrapping` makes it the Large Icons box: centered text wrapped in the
/// label width, Enter still commits.
fn rename_editor(ui: &mut Ui, r: &mut RenameState, wrapping: bool, actions: &mut Vec<Action>) {
    let id = ui.make_persistent_id(("rename", &r.path));
    let frame = egui::Frame::NONE
        .fill(theme::color::WINDOW)
        .stroke(egui::Stroke::new(1.0, theme::color::TEXT))
        .inner_margin(egui::Margin::symmetric(2, 0));
    let edit = if wrapping {
        TextEdit::multiline(&mut r.text)
            .return_key(None)
            .desired_rows(1)
            .desired_width(layout::LABEL_WRAP)
            .horizontal_align(Align::Center)
    } else {
        TextEdit::singleline(&mut r.text).desired_width(f32::INFINITY)
    };
    let resp = frame.show(ui, |ui| ui.add(edit.id(id).frame(egui::Frame::NONE))).inner;
    if resp.changed() {
        r.text.retain(|c| !is_invalid_char(c) && c != '\n');
    }
    if wrapping && resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        resp.surrender_focus();
        actions.push(Action::CommitRename);
    }
    if std::mem::take(&mut r.needs_init) {
        resp.request_focus();
        let sel = initial_selection(&r.text, r.kind, r.hidden_ext.is_none());
        if let Some(mut state) = TextEdit::load_state(ui.ctx(), id) {
            state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(sel.start), CCursor::new(sel.end))));
            TextEdit::store_state(ui.ctx(), id, state);
        }
    }
    if resp.lost_focus() {
        // Escape cancels; Enter or clicking elsewhere commits, like Explorer.
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            actions.push(Action::CancelRename);
        } else {
            actions.push(Action::CommitRename);
        }
    }
}

/// An item label that highlights only its text when selected: navy with
/// white text when the pane has focus, button-face gray otherwise.
/// Returns the highlight box.
/// A details view cell's text, for every column but Name.
fn cell_text(entry: &Entry, column: Column) -> String {
    let date = |t: Option<std::time::SystemTime>| t.map_or_else(String::new, locale::short_date_time);
    match column {
        Column::Name => entry.display_name(true).to_owned(),
        Column::Size => entry.size.map_or_else(String::new, format_size),
        Column::Type => entry.kind_label(),
        Column::Modified => date(entry.modified),
        Column::Attributes => entry.attributes(),
        // One line, like a list view cell.
        Column::Comment => entry.comment.as_deref().map_or_else(String::new, |c| c.replace(['\n', '\r'], " ")),
        Column::Created => date(entry.created),
        Column::Accessed => date(entry.accessed),
        Column::Owner => entry.owner.clone(),
        Column::Group => entry.group.clone(),
    }
}

/// The column header's context menu (`CDefView::OnVisibleColumnMenu`):
/// a check per column, then "More...".
fn header_menu(ui: &mut Ui, columns: &ColumnSet, actions: &mut Vec<Action>) {
    let (items, more) = columns.menu();
    for item in items {
        let mut checked = item.checked;
        if ui.add_enabled(item.enabled, egui::Checkbox::new(&mut checked, item.column.caption())).clicked() {
            actions.push(Action::ToggleColumn(item.column));
            ui.close();
        }
    }
    if more {
        ui.separator();
        if ui.button("More...").clicked() {
            actions.push(Action::ChooseColumns);
            ui.close();
        }
    }
}

fn selectable_name(ui: &mut Ui, text: &str, color: egui::Color32, selected: bool, focused: bool) -> egui::Rect {
    let bg = ui.painter().add(egui::Shape::Noop);
    let resp = ui.add(egui::Label::new(RichText::new(text).color(color)).truncate().selectable(false).sense(Sense::hover()));
    let rect = resp.rect.expand2(egui::vec2(2.0, 1.0));
    if selected {
        let fill = if focused { theme::color::SELECTION } else { theme::color::FACE };
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 0.0, fill));
    }
    rect
}

/// A flat crumb in the address field that pops out when hovered.
fn crumb_width(ui: &Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().layout_no_wrap(text.to_owned(), font, theme::color::TEXT).size().x + 6.0
}

fn crumb_button(ui: &mut Ui, text: &str) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, theme::color::TEXT);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 6.0, 16.0), Sense::click());
    if resp.is_pointer_button_down_on() {
        theme::bevel(ui.painter(), rect, Bevel::SunkenThin);
    } else if resp.hovered() {
        theme::bevel(ui.painter(), rect, Bevel::RaisedThin);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, theme::color::TEXT);
    resp
}

/// egui's input clock in milliseconds.
fn now_ms(ctx: &egui::Context) -> u64 {
    ctx.input(|i| (i.time * 1000.0) as u64)
}

fn window_focused(ui: &Ui) -> bool {
    ui.ctx().input(|i| i.viewport().focused.unwrap_or(true))
}

/// Where to draw the dotted connector lines for one tree row.
struct Guide {
    /// For each ancestor level, whether its vertical line continues past this row.
    ancestors: Vec<bool>,
    /// Whether this row has a later sibling (its own line continues down).
    has_next: bool,
    /// Whether a row above connects to this one (false for a section's first root).
    has_prev: bool,
}

/// Computes connector lines with one backward pass. `next[d]` is true when
/// a later row at depth `d` follows before any shallower row, which is
/// exactly "this level's line continues". Sections are independent trees.
fn tree_guides(rows: &[Row]) -> Vec<Guide> {
    let mut out: Vec<Guide> = Vec::with_capacity(rows.len());
    let mut next: Vec<bool> = Vec::new();
    let mut section = None;
    for row in rows.iter().rev() {
        if section != Some(row.section) {
            section = Some(row.section);
            next.clear();
        }
        let d = row.depth;
        next.resize(d + 1, false);
        out.push(Guide { ancestors: next[..d].to_vec(), has_next: next[d], has_prev: true });
        next[d] = true;
        next.truncate(d + 1);
    }
    out.reverse();
    let mut section = None;
    for (g, row) in out.iter_mut().zip(rows) {
        if section != Some(row.section) {
            section = Some(row.section);
            g.has_prev = false;
        }
    }
    out
}

/// One Folders pane row in the Windows 2000 tree view style: dotted
/// connector lines, a +/− box, a folder icon and a label that highlights
/// only its text.
fn tree_row(ui: &mut Ui, icons: &mut FileIcons, row: &Row, guide: &Guide, scroll: bool, focused: bool, actions: &mut Vec<Action>) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_w = ui.painter().layout_no_wrap(row.label.clone(), font.clone(), theme::color::TEXT).size().x;
    let x0 = ui.max_rect().left() + 2.0;
    let label_x = x0 + (row.depth + 1) as f32 * TREE_INDENT + 16.0 + 4.0;
    let width = (label_x - ui.max_rect().left() + text_w + 6.0).max(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, TREE_ROW_HEIGHT), Sense::click());
    let p = ui.painter();
    let line = egui::Stroke::new(1.0, theme::color::TREE_LINE);
    // Single-pixel dots on even pixel coordinates, so the pattern lines up
    // between rows and stays crisp whatever the row's fractional offset.
    let dotted = |a: egui::Pos2, b: egui::Pos2| {
        let (lo, hi) = if a.x == b.x { (a.y, b.y) } else { (a.x, b.x) };
        let mut t = (lo / 2.0).ceil() * 2.0;
        while t < hi {
            let at = if a.x == b.x { egui::pos2(a.x - 0.5, t) } else { egui::pos2(t, a.y - 0.5) };
            theme::dot(p, at, line.color);
            t += 2.0;
        }
    };
    let col_x = |level: usize| (x0 + level as f32 * TREE_INDENT + TREE_INDENT / 2.0).floor() + 0.5;
    let mid_y = rect.center().y.floor() + 0.5;

    for (level, &continues) in guide.ancestors.iter().enumerate() {
        if continues {
            dotted(egui::pos2(col_x(level), rect.top()), egui::pos2(col_x(level), rect.bottom()));
        }
    }
    let cx = col_x(row.depth);
    dotted(egui::pos2(cx, if guide.has_prev { rect.top() } else { mid_y }), egui::pos2(cx, if guide.has_next { rect.bottom() } else { mid_y }));
    dotted(egui::pos2(cx, mid_y), egui::pos2(cx + TREE_INDENT / 2.0 + 1.0, mid_y));

    let boxr = egui::Rect::from_center_size(egui::pos2(cx, mid_y), egui::vec2(9.0, 9.0));
    if row.expandable {
        p.rect_filled(boxr, 0.0, theme::color::WINDOW);
        p.rect_stroke(boxr, 0.0, line, egui::StrokeKind::Inside);
        let s = egui::Stroke::new(1.0, theme::color::TEXT);
        p.hline(boxr.left() + 2.0..=boxr.right() - 2.0, mid_y, s);
        if !row.expanded {
            p.vline(cx, boxr.top() + 2.0..=boxr.bottom() - 2.0, s);
        }
    }

    let icon_rect = egui::Rect::from_min_size(egui::pos2(x0 + (row.depth + 1) as f32 * TREE_INDENT, rect.center().y - 8.0), egui::vec2(16.0, 16.0));
    if row.placeholder {
        p.text(egui::pos2(icon_rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, &row.label, font, theme::color::GRAY_TEXT);
        return;
    }
    if row.depth == 0 && row.section == Section::Locations {
        theme::paint_icon(p, icon_rect, Icon::Drive, true);
    } else {
        icons.paint_folder(ui, icon_rect, &row.key.path);
    }
    let text_rect = egui::Rect::from_min_size(egui::pos2(label_x - 2.0, rect.center().y - 7.0), egui::vec2(text_w + 4.0, 14.0));
    let text_color = if row.selected {
        let fill = if focused { theme::color::SELECTION } else { theme::color::FACE };
        p.rect_filled(text_rect, 0.0, fill);
        if focused { theme::color::SELECTION_TEXT } else { theme::color::TEXT }
    } else {
        theme::color::TEXT
    };
    p.text(egui::pos2(label_x, rect.center().y), egui::Align2::LEFT_CENTER, &row.label, font, text_color);
    if row.selected && scroll {
        ui.scroll_to_rect(rect, Some(Align::Center));
    }

    let resp = match &row.error {
        Some(e) => resp.on_hover_text(e),
        None => resp,
    };
    let hit_box = boxr.expand(3.0);
    let on_box = resp.interact_pointer_pos().is_some_and(|p| hit_box.contains(p));
    if resp.clicked() {
        if on_box && row.expandable {
            actions.push(Action::TreeToggle(row.key.clone()));
            actions.push(Action::Focus(Pane::Tree));
        } else {
            actions.push(Action::TreeSelect(row.key.clone()));
        }
    }
    if resp.double_clicked() && !on_box && row.expandable {
        actions.push(Action::TreeToggle(row.key.clone()));
    }
    resp.context_menu(|ui| {
        if row.expandable {
            let label = if row.expanded { "Collapse" } else { "Expand" };
            if ui.button(RichText::new(label).font(theme::bold(theme::FONT_SIZE))).clicked() {
                actions.push(Action::TreeToggle(row.key.clone()));
            }
        }
        if ui.button("Open").clicked() {
            actions.push(Action::TreeSelect(row.key.clone()));
        }
        if ui.button(platform::REVEAL_LABEL).clicked() {
            actions.push(Action::RevealInFinder(row.key.path.clone()));
        }
        if is_ejectable(&row.key.path) && ui.button("Eject").clicked() {
            actions.push(Action::Eject(row.key.path.clone()));
        }
        ui.separator();
        if ui.button("Properties").clicked() {
            actions.push(Action::Properties(row.key.path.clone()));
        }
    });
}

/// Arrow keys in the navigation pane, following tree view conventions.
fn tree_key_action(rows: &[Row], key: Key) -> Option<Action> {
    let real: Vec<usize> = (0..rows.len()).filter(|&i| !rows[i].placeholder).collect();
    let cur = real.iter().position(|&i| rows[i].selected);
    let select = |pos: usize| Some(Action::TreeSelect(rows[real[pos]].key.clone()));
    let Some(pos) = cur else {
        return matches!(key, Key::ArrowDown | Key::ArrowUp).then(|| select(0)).flatten();
    };
    let row = &rows[real[pos]];
    match key {
        Key::ArrowDown if pos + 1 < real.len() => select(pos + 1),
        Key::ArrowUp if pos > 0 => select(pos - 1),
        Key::ArrowRight if row.expandable && !row.expanded => Some(Action::TreeExpand(row.key.clone())),
        Key::ArrowRight => real
            .get(pos + 1)
            .filter(|&&i| rows[i].depth > row.depth)
            .map(|&i| Action::TreeSelect(rows[i].key.clone())),
        Key::ArrowLeft if row.expanded => Some(Action::TreeCollapse(row.key.clone())),
        Key::ArrowLeft => real[..pos]
            .iter()
            .rev()
            .find(|&&i| rows[i].depth + 1 == row.depth)
            .map(|&i| Action::TreeSelect(rows[i].key.clone())),
        _ => None,
    }
}

fn tree_roots() -> Vec<Root> {
    let favs = favorites();
    let vols = volumes();
    Root::from_places(&favs, Section::Favorites).chain(Root::from_places(&vols, Section::Locations)).collect()
}

fn select_all(ui: &Ui, id: egui::Id, text: &str) {
    if let Some(mut state) = TextEdit::load_state(ui.ctx(), id) {
        let end = text.chars().count();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
        TextEdit::store_state(ui.ctx(), id, state);
    }
}

fn subfolders(dir: &Path, show_hidden: bool) -> Vec<(String, PathBuf)> {
    let mut out: Vec<_> = list_dir(dir, show_hidden)
        .unwrap_or_default()
        .into_iter()
        .filter(Entry::is_folder)
        .map(|e| (e.name, e.path))
        .collect();
    out.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    out
}

/// Tallest the crumb chevron's menu grows before it scrolls.
const MENU_MAX_HEIGHT: f32 = 400.0;

/// A menu of `items` that scrolls with a classic scroll bar once it is
/// taller than [`MENU_MAX_HEIGHT`]. Returns the clicked item's path. The
/// size is worked out up front so the popup doesn't resize when the bar
/// appears.
fn scrolled_menu(ui: &mut Ui, items: &[(String, PathBuf)]) -> Option<PathBuf> {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let (pad, spacing, min_height) = (ui.spacing().button_padding, ui.spacing().item_spacing.y, ui.spacing().interact_size.y);
    let text_width = items.iter().map(|(name, _)| ui.painter().layout_no_wrap(name.clone(), font.clone(), theme::color::TEXT).size().x).fold(0.0, f32::max);
    let row = (ui.fonts_mut(|f| f.row_height(&font)) + 2.0 * pad.y).max(min_height) + spacing;
    let content = row * items.len() as f32 - spacing;
    let height = content.min(MENU_MAX_HEIGHT);
    let overflows = content > height;
    let width = text_width + 2.0 * pad.x + if overflows { scrollbar::WIDTH } else { 0.0 };
    let (outer, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());

    // The bar is known to be needed from the first frame.
    let id = ui.id().with("menu-bars");
    ui.data_mut(|d| d.insert_temp(id, egui::Vec2b::new(false, overflows)));
    let (bars, viewport) = Bars::begin(ui, id, outer, [false, true], row);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down_justified(Align::Min)));
    let mut clicked = None;
    let out = egui::ScrollArea::vertical().auto_shrink(false).scroll_bar_visibility(scrollbar::hidden(&mut inner)).show(&mut inner, |ui| {
        for (name, path) in items {
            if ui.button(name).clicked() {
                clicked = Some(path.clone());
            }
        }
    });
    bars.end(&mut inner, &out);
    clicked
}

fn place_label(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The view modes as radio items, for the View menu and the Views button.
fn view_choices(ui: &mut Ui, current: ViewMode, actions: &mut Vec<Action>) {
    for (mode, label) in [(ViewMode::LargeIcons, "Large Icons"), (ViewMode::List, "List"), (ViewMode::Details, "Details")] {
        if ui.radio(current == mode, label).clicked() {
            actions.push(Action::SetView(mode));
            ui.close();
        }
    }
}

fn expand_tilde(input: &str, home: Option<&Path>) -> PathBuf {
    match (input.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> FolderTree {
        let root = |label: &str, path: &str| Root { label: label.into(), path: path.into(), section: Section::Favorites };
        let node = |path: &str| Node {
            name: Path::new(path).file_name().unwrap().to_string_lossy().into(),
            path: path.into(),
            has_subfolders: true,
        };
        let mut t = FolderTree::new(vec![root("Home", "/u"), root("Apps", "/a")]);
        t.expand(NodeKey { root: 0, path: "/u".into() });
        t.loaded("/u".into(), Ok(vec![node("/u/x"), node("/u/y")]));
        t
    }

    fn target(a: Option<Action>) -> Option<(&'static str, PathBuf)> {
        match a? {
            Action::TreeSelect(k) => Some(("select", k.path)),
            Action::TreeExpand(k) => Some(("expand", k.path)),
            Action::TreeCollapse(k) => Some(("collapse", k.path)),
            _ => None,
        }
    }

    fn at(t: &mut FolderTree, root: usize, path: &str, key: Key) -> Option<(&'static str, PathBuf)> {
        t.select(NodeKey { root, path: path.into() });
        target(tree_key_action(&t.rows(), key))
    }

    #[test]
    fn arrows_follow_tree_view_rules() {
        let mut t = tree();
        assert_eq!(at(&mut t, 0, "/u", Key::ArrowDown), Some(("select", "/u/x".into())));
        assert_eq!(at(&mut t, 0, "/u/y", Key::ArrowDown), Some(("select", "/a".into())));
        assert_eq!(at(&mut t, 0, "/u", Key::ArrowUp), None);
        // Right expands a collapsed node, then moves into its first child.
        assert_eq!(at(&mut t, 0, "/u/x", Key::ArrowRight), Some(("expand", "/u/x".into())));
        assert_eq!(at(&mut t, 0, "/u", Key::ArrowRight), Some(("select", "/u/x".into())));
        // Left collapses an expanded node, otherwise jumps to the parent.
        assert_eq!(at(&mut t, 0, "/u", Key::ArrowLeft), Some(("collapse", "/u".into())));
        assert_eq!(at(&mut t, 0, "/u/y", Key::ArrowLeft), Some(("select", "/u".into())));
    }

    #[test]
    fn down_with_no_selection_picks_first_row() {
        let t = tree();
        assert_eq!(target(tree_key_action(&t.rows(), Key::ArrowDown)), Some(("select", "/u".into())));
    }
}

/// Renders the window offscreen to `target/screenshots/*.png` so the look can
/// be checked without a display. Run with `cargo test -p macsplorer -- --ignored`.
#[cfg(test)]
mod clipboard {
    use super::*;
    use std::fs;

    fn explorer(name: &str) -> (Explorer, PathBuf) {
        let dir = std::env::temp_dir().join(format!("macsplorer-clipboard-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), "hi").unwrap();
        let mut app = Explorer::new(egui::Context::default(), dir.clone());
        app.finish_listing();
        (app, dir)
    }

    fn wait(app: &mut Explorer) {
        for _ in 0..500 {
            app.poll_ops();
            if app.status.is_none() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.status.is_none(), "{:?}", app.status);
    }

    #[test]
    fn undo_reverses_rename_and_move() {
        let (mut app, dir) = explorer("undo");
        fs::rename(dir.join("a.txt"), dir.join("b.txt")).unwrap();
        app.undo.push(UndoOp::Rename { from: dir.join("a.txt"), to: dir.join("b.txt") });
        fs::create_dir(dir.join("c")).unwrap();
        app.reload();
        app.finish_listing();
        app.selection = BTreeSet::from([dir.join("c")]);
        app.apply(vec![Action::Cut, Action::Paste(Some(dir.join("sub")))], &[]);
        wait(&mut app);
        assert!(dir.join("sub/c").exists());
        assert_eq!(app.undo.peek().map(UndoOp::label), Some("Move"));
        app.apply(vec![Action::Undo], &[]);
        wait(&mut app);
        assert!(dir.join("c").exists() && !dir.join("sub/c").exists());
        app.apply(vec![Action::Undo], &[]);
        wait(&mut app);
        assert!(dir.join("a.txt").exists() && !dir.join("b.txt").exists());
        assert!(app.undo.peek().is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_paste_and_cut_paste() {
        let (mut app, dir) = explorer("ops");
        app.selection = BTreeSet::from([dir.join("a.txt")]);
        app.apply(vec![Action::Copy, Action::Paste(None)], &[]);
        wait(&mut app);
        assert_eq!(fs::read_to_string(dir.join("a - Copy.txt")).unwrap(), "hi");
        assert!(app.clipboard.is_some(), "a copy can be pasted again");

        app.selection = BTreeSet::from([dir.join("a.txt")]);
        app.apply(vec![Action::Cut, Action::Paste(Some(dir.join("sub")))], &[]);
        wait(&mut app);
        assert!(!dir.join("a.txt").exists());
        assert_eq!(fs::read_to_string(dir.join("sub/a.txt")).unwrap(), "hi");
        assert!(app.clipboard.is_none(), "a cut is pasted once");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn typing_selects_the_item_it_spells() {
        let (mut app, dir) = explorer("typeahead");
        fs::write(dir.join("apple.txt"), "").unwrap();
        fs::write(dir.join("Banana.txt"), "").unwrap();
        app.reload();
        app.finish_listing();
        let visible: Vec<usize> = (0..app.entries.len()).collect();
        let name = |app: &Explorer| app.focus_item.map(|i| app.entries[i].name.clone());
        let ty = |app: &mut Explorer, text: &str, time_ms| app.apply(vec![Action::TypeAhead { text: text.into(), time_ms }], &visible);

        ty(&mut app, "b", 0);
        assert_eq!(name(&app), Some("Banana.txt".into()));
        assert_eq!(app.selection, BTreeSet::from([dir.join("Banana.txt")]));
        assert_eq!(app.reveal, app.focus_item);

        // A first letter searches after the focus and wraps; the same letter
        // again steps on to the next match.
        ty(&mut app, "a", 2000);
        assert_eq!(name(&app), Some("a.txt".into()));
        ty(&mut app, "a", 2100);
        assert_eq!(name(&app), Some("apple.txt".into()));
        // A key that ends the text makes the next letter a first letter again.
        app.apply(vec![Action::EndTypeAhead], &visible);
        ty(&mut app, "b", 2200);
        assert_eq!(name(&app), Some("Banana.txt".into()));
        ty(&mut app, "zz", 9000);
        assert_eq!(app.selection.len(), 1, "no match keeps the selection");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paste_conflict_is_reported() {
        let (mut app, dir) = explorer("conflict");
        fs::write(dir.join("sub/a.txt"), "old").unwrap();
        app.selection = BTreeSet::from([dir.join("a.txt")]);
        app.apply(vec![Action::Cut, Action::Paste(Some(dir.join("sub")))], &[]);
        for _ in 0..500 {
            app.poll_ops();
            if !app.status.as_deref().is_some_and(|s| s.starts_with("Moving")) {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.status.as_deref().unwrap().contains("already an item named"));
        assert_eq!(fs::read_to_string(dir.join("sub/a.txt")).unwrap(), "old");
        assert!(dir.join("a.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod screenshot {
    use super::*;
    use egui_kittest::Harness;
    use std::fs;

    /// Built once per run: the tests run in parallel and share it.
    fn fixture() -> PathBuf {
        static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        DIR.get_or_init(build_fixture).clone()
    }

    fn build_fixture() -> PathBuf {
        crate::locale::install();
        let dir = std::env::temp_dir().join("macsplorer-screenshot").join("My Documents");
        let _ = fs::remove_dir_all(&dir);
        for d in ["Projects/2026", "Projects/Archive", "Pictures", "Setup.app"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        for name in ["Projects/plan.txt", "Projects/2026/budget.xlsx", "Projects/Archive/old.zip"] {
            fs::write(dir.join(name), vec![b'x'; 400_000]).unwrap();
        }
        for name in ["main.rs", "app.py", "index.ts", "README.md", "Makefile", "Cargo.toml", "styles.css", "package.json", "photo.png", "archive.zip", "Dockerfile", "notes.txt", "report.pdf", "query.sql", "script.sh", "budget.xlsx", "data.yaml", "Quarterly report for the board of directors.txt"] {
            fs::write(dir.join(name), vec![b'x'; 1_500]).unwrap();
        }
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.join("script.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        // A real app (via symlink) to show a native icon.
        let _ = std::os::unix::fs::symlink("/System/Applications/Calculator.app", dir.join("Calculator.app"));
        dir
    }

    fn shoot(name: &str, size: egui::Vec2, setup: impl FnOnce(&mut Explorer)) {
        shoot_driven(name, size, setup, |_| {});
    }

    /// Like `shoot`, with `drive` sending input once the window has settled.
    fn shoot_driven(name: &str, size: egui::Vec2, setup: impl FnOnce(&mut Explorer), drive: impl FnOnce(&mut Harness<'_, Explorer>)) {
        let start = fixture();
        let mut harness = Harness::builder()
            .with_size(size)
            .with_pixels_per_point(2.0)
            .wgpu()
            .build_eframe(|cc| {
                theme::install(&cc.egui_ctx);
                let mut app = Explorer::new(cc.egui_ctx.clone(), start.clone());
                app.finish_listing();
                setup(&mut app);
                app
            });
        // Let background tree loads arrive.
        for _ in 0..20 {
            harness.run();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drive(&mut harness);
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join(format!("{name}.png"))).unwrap();
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn window() {
        shoot("window", egui::vec2(900.0, 560.0), |app| {
            let pick = app.entries.iter().find(|e| e.name == "main.rs").map(|e| e.path.clone());
            app.selection.extend(pick);
        });
    }

    fn select(app: &mut Explorer, names: &[&str]) {
        for name in names {
            let i = app.entries.iter().position(|e| e.name == *name).unwrap();
            app.selection.insert(app.entries[i].path.clone());
            app.focus_item = Some(i);
        }
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn large_icons() {
        shoot("large-icons", egui::vec2(900.0, 560.0), |app| {
            app.settings.view = ViewMode::LargeIcons;
            select(app, &["main.rs", "Pictures"]);
        });
    }

    /// The focused item's long label unfolds over the row below.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn large_icons_unfolded() {
        shoot("large-icons-unfolded", egui::vec2(700.0, 400.0), |app| {
            app.settings.view = ViewMode::LargeIcons;
            select(app, &["Quarterly report for the board of directors.txt"]);
            app.reveal = app.focus_item;
        });
    }

    /// Presses at `from`, moves to `to` in steps and keeps the button down.
    fn drag(harness: &mut Harness<'_, Explorer>, from: egui::Pos2, to: egui::Pos2) {
        harness.hover_at(from);
        harness.run();
        harness.drag_at(from);
        harness.run();
        for step in 1..=8 {
            harness.hover_at(from + (to - from) * (step as f32 / 8.0));
            harness.run();
        }
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn list() {
        shoot("list", egui::vec2(900.0, 400.0), |app| {
            app.settings.view = ViewMode::List;
            select(app, &["main.rs", "Setup.app"]);
        });
    }

    /// A marquee dragged from empty space in Large Icons, button still down.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn marquee_icons() {
        shoot_driven(
            "marquee-icons",
            egui::vec2(900.0, 560.0),
            |app| app.settings.view = ViewMode::LargeIcons,
            |h| drag(h, egui::pos2(880.0, 140.0), egui::pos2(520.0, 250.0)),
        );
    }

    /// A marquee dragged from empty space below the last List column.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn marquee_list() {
        shoot_driven(
            "marquee-list",
            egui::vec2(900.0, 400.0),
            |app| app.settings.view = ViewMode::List,
            |h| drag(h, egui::pos2(800.0, 300.0), egui::pos2(380.0, 160.0)),
        );
    }

    /// A marquee started beside a row's name in Details.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn marquee_details() {
        shoot_driven("marquee-details", egui::vec2(900.0, 560.0), |_| {}, |h| drag(h, egui::pos2(700.0, 150.0), egui::pos2(300.0, 260.0)));
    }

    /// The Name column's divider dragged left, grabbed 3 px inside the
    /// column, button still down.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn column_resize() {
        shoot_driven("column-resize", egui::vec2(900.0, 560.0), |_| {}, |h| drag(h, egui::pos2(526.0, 111.0), egui::pos2(400.0, 140.0)));
    }

    /// The column header's context menu, opened on Type.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn header_menu() {
        shoot_driven("header-menu", egui::vec2(900.0, 400.0), |_| {}, |h| {
            let pos = egui::pos2(650.0, 111.0);
            h.hover_at(pos);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
                h.run();
            }
            h.run();
        });
    }

    /// Type, Created and Owner turned on from the header menu.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn details_columns() {
        shoot("details-columns", egui::vec2(1200.0, 400.0), |app| {
            for column in [Column::Type, Column::Created, Column::Owner] {
                app.columns.toggle(column);
            }
        });
    }

    /// The header menu's More... dialog, Created checked and moved up.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn column_settings() {
        let mut harness = Harness::builder().with_size(crate::column_dialog::SIZE).with_pixels_per_point(2.0).wgpu().build_ui_state(
            |ui, dialog: &mut Option<ColumnDialog>| {
                let Some(dialog) = dialog else {
                    theme::install(ui.ctx());
                    let mut columns = ColumnSet::default();
                    columns.set_visible(&[Column::Name, Column::Size, Column::Type, Column::Created, Column::Modified]);
                    *dialog = Some(ColumnDialog::new(&columns));
                    return;
                };
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                dialog.ui(&mut window);
            },
            None,
        );
        for _ in 0..3 {
            harness.run();
        }
        // Select Created.
        harness.hover_at(egui::pos2(60.0, 159.0));
        harness.run();
        for pressed in [true, false] {
            let (pos, button, modifiers) = (egui::pos2(60.0, 159.0), egui::PointerButton::Primary, egui::Modifiers::NONE);
            harness.event(egui::Event::PointerButton { pos, button, pressed, modifiers });
            harness.run();
        }
        harness.run();
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join("column-settings.png")).unwrap();
    }

    /// Undo enabled after a rename, and the Views menu open.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn views_menu() {
        shoot_driven(
            "views-menu",
            egui::vec2(700.0, 300.0),
            |app| app.undo.push(UndoOp::Rename { from: PathBuf::from("/a"), to: PathBuf::from("/b") }),
            |h| {
                use egui_kittest::kittest::Queryable;
                h.get_by_label("Views").click();
                h.run();
            },
        );
    }

    /// The Properties dialog for `name` in the fixture, once its folder walk
    /// has finished.
    fn shoot_properties(shot: &str, name: &str) {
        let path = fixture().join(name);
        let mut harness = Harness::builder()
            .with_size(crate::properties::SIZE)
            .with_pixels_per_point(2.0)
            .wgpu()
            .build_ui_state(
                |ui, (dialog, icons): &mut (Option<Properties>, FileIcons)| {
                    // Fonts installed now apply from the next frame.
                    let Some(dialog) = dialog else {
                        theme::install(ui.ctx());
                        let entry = list_dir(path.parent().unwrap(), false).unwrap().into_iter().find(|e| e.path == path).unwrap();
                        *dialog = Some(Properties::new(ui.ctx(), entry, true, true));
                        return;
                    };
                    // The harness's panel has a margin; the window has none.
                    let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                    dialog.ui(&mut window, icons);
                },
                (None, FileIcons::new()),
            );
        for _ in 0..20 {
            harness.run();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join(format!("{shot}.png"))).unwrap();
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn properties_folder() {
        shoot_properties("properties-folder", "Projects");
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn properties_file() {
        shoot_properties("properties-file", "Quarterly report for the board of directors.txt");
    }

    /// The type name is Finder's Kind for the file.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn properties_kind() {
        shoot_properties("properties-kind", "budget.xlsx");
    }

    /// The last crumb's chevron menu in a folder with too many subfolders to fit.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn chevron_menu() {
        let dir = std::env::temp_dir().join("macsplorer-screenshot-menu").join("Many folders");
        let _ = fs::remove_dir_all(&dir);
        for i in 1..=40 {
            fs::create_dir_all(dir.join(format!("Folder {i:02}"))).unwrap();
        }
        let mut harness = Harness::builder().with_size(egui::vec2(500.0, 520.0)).with_pixels_per_point(2.0).wgpu().build_eframe(|cc| {
            theme::install(&cc.egui_ctx);
            let mut app = Explorer::new(cc.egui_ctx.clone(), dir.clone());
            app.finish_listing();
            app
        });
        for _ in 0..10 {
            harness.run();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let pos = egui::pos2(169.0, 85.0);
        harness.hover_at(pos);
        harness.run();
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            harness.run();
        }
        for _ in 0..3 {
            harness.run();
        }
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join("chevron-menu.png")).unwrap();
    }

    /// A window small enough that both panes need classic scroll bars.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn scroll_bars() {
        shoot("scroll-bars", egui::vec2(560.0, 300.0), |_| {});
    }
}
