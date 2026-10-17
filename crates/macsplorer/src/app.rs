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
use macsplorer_core::fileops::{self, Choice, Progress, transfer_with};
use macsplorer_core::focus::{self, Stop};
use macsplorer_core::git;
use macsplorer_core::history::History;
use macsplorer_core::isearch::{self, IncrementalSearch};
use macsplorer_core::menu_delay;
use macsplorer_core::layout::{self, IconCell, IconGrid, ListColumns, Nav};
use macsplorer_core::dragdrop::{self, Allowed, Effect};
use macsplorer_core::marquee::{self, Marquee};
use macsplorer_core::listing::{Entry, SortSpec, entry_at, format_byte_size, format_size, list_dir, list_dir_with, sort_entries};
#[cfg(unix)]
use macsplorer_core::listing::free_space;
use macsplorer_core::natural::natural_cmp;
use macsplorer_core::preferences::{Preferences, SortOrder, ViewMode};
use macsplorer_core::places::{favorites, volumes};
#[cfg(unix)]
use macsplorer_core::places::{computer_name, is_ejectable};
#[cfg(unix)]
use macsplorer_core::properties::set_permissions;
use macsplorer_core::rename::{RenameKind, initial_selection, is_invalid_char, rename_no_replace, validate_name};
use macsplorer_core::slow_click::{self, SlowClick};
use macsplorer_core::text_size;
use macsplorer_core::tree::{FolderTree, Node, NodeKey, Root, Row, Scroll, Section, load_children};
use macsplorer_core::undo::{Trash, UndoOp, UndoStack};
use macsplorer_core::watch::FolderWatcher;

use crate::column_dialog::{self, ColumnDialog};
use crate::about_dialog::{AboutDialog, Outcome as AboutOutcome};
use crate::conflict_dialog::{self, ConflictDialog};
use crate::pasteboard::{self, Pasteboard};
use crate::icons::{self, FileIcons, IconState};
use crate::item_drag::{self, DragFrame, ItemDrag, Mark, Pointer, Step};
use crate::native_drag;
use crate::locale;
use crate::open_with;
use crate::open_with_dialog::{self, OpenWithDialog};
use crate::platform;
use crate::progress_dialog::ProgressDialog;
use crate::properties::{Outcome, Properties};
use crate::scrollbar::{self, Bars};
use crate::theme::{self, Bevel, Icon};
use crate::trash::SystemTrash;

const LIST_ROW_HEIGHT: f32 = 17.0;
const LIST_HEADER_HEIGHT: f32 = 18.0;
const TREE_ROW_HEIGHT: f32 = 18.0;
const TREE_INDENT: f32 = 19.0;
/// How long a new window size has to stay before it is written, so a
/// resize writes the settings file once, at its end.
const WINDOW_SAVE_DELAY: f64 = 0.5;

type LoadResult = (PathBuf, bool, Result<Vec<Node>, String>);
/// The Git branch glyph beside text (status bar, Branch column): the room
/// it takes and how much `icons::paint_git_branch` enlarges it from its
/// 16 px folder size.
const GIT_ICON_WIDTH: f32 = 12.0;
const GIT_ICON_SCALE: f32 = 1.6;
/// A details-view listing, tagged with the request it answers.
type ListResult = (u64, Result<Vec<Entry>, String>, Option<u64>);
/// What a Git request found out, tagged with the request it answers.
type GitResult = (u64, GitUpdate);

enum GitUpdate {
    /// Still in the working tree whose summary is up: nothing to change.
    Keep,
    /// The folder's summary, `None` outside a working tree.
    Set(Option<git::Status>),
}
/// Search-filtered rows and the (entries version, extensions shown, needle) they came from.
type VisibleCache = ((u64, bool, String), Rc<[usize]>);

/// Last frame's Large Icons or List layout, for arrow keys handled before
/// the view is drawn.
#[derive(Clone, Copy)]
enum NavLayout {
    Icons { grid: IconGrid, client_height: f32 },
    List { columns: ListColumns, client_width: f32 },
    /// Details: how many rows fit under the header.
    Details { rows: usize },
}

struct Settings {
    view: ViewMode,
    show_extensions: bool,
    show_hidden: bool,
    natural_sort: bool,
    /// Finder-style ordering: folders sorted in among the files.
    mix_folders: bool,
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
    /// New > Folder in the open folder.
    NewFolder,
    /// A path typed in the address bar: a folder is opened, a file is shown selected in its folder.
    GoTo(PathBuf),
    Open(PathBuf),
    /// Open a file with an app; `None` asks the user to pick one.
    OpenWith(PathBuf, Option<PathBuf>),
    /// A click on an item. `on_label`: a left click on its label text,
    /// which may start a slow-click rename.
    Click { index: usize, toggle: bool, extend: bool, on_label: bool },
    /// Scroll the item view so this entry is visible.
    Reveal(usize),
    /// Characters typed in the item view at `time_ms`. `extend`: Shift was
    /// down, which turns a Space into a range selection.
    TypeAhead { text: String, time_ms: u64, extend: bool },
    /// Characters typed in the Folders pane.
    TreeTypeAhead { text: String, time_ms: u64 },
    /// A key that ends the typed text (Enter, Esc, Tab, Backspace, navigation keys).
    EndTypeAhead,
    /// Clear the selection (a click on empty space in the view).
    Deselect,
    /// Tab, F6 and the address and search keys: give that pane the keyboard.
    FocusStop(Stop),
    /// The keyboard was used to move around: draw focus rectangles from now on.
    ShowFocus,
    /// Start a marquee at this content point; `extend` (Cmd or Shift) keeps
    /// the selection.
    MarqueeBegin { origin: (f32, f32), extend: bool },
    /// A press on this item became a drag: drag it, with the rest of the
    /// selection if it is selected.
    DragItems(usize),
    /// A drag of a Folders pane row.
    DragFolder(PathBuf),
    /// The drag was dropped on this folder.
    Drop { dir: PathBuf, effect: Effect },
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
    /// Start another Macsplorer process on the open folder.
    NewWindow,
    RevealInFinder(PathBuf),
    /// Help > Open config file: the settings file in a text editor.
    OpenConfig,
    /// Nautilus' "Open in Terminal": a terminal window in this folder.
    #[cfg(unix)]
    OpenTerminal(PathBuf),
    #[cfg(unix)]
    Eject(PathBuf),
    Cut,
    Copy,
    /// Explorer's "Copy as path": the selection's quoted full paths as text.
    CopyAsPath,
    /// Paste into a folder; `None` is the open folder.
    Paste(Option<PathBuf>),
    /// Finder's "Move Item Here" (Option+Cmd+V): paste as a move, even a copy.
    MoveHere(Option<PathBuf>),
    Delete,
    TreeSelect(NodeKey),
    /// A tree selection made with the keyboard, which opens after a pause.
    TreeKeySelect(NodeKey),
    TreeToggle(NodeKey),
    TreeExpand(NodeKey),
    TreeCollapse(NodeKey),
    Focus(Pane),
    ToggleFolders,
    SelectAll,
    InvertSelection,
    Undo,
    SetView(ViewMode),
    SetDark(bool),
    /// View > Text Size, also Cmd +/-/0.
    SetZoom(f32),
    /// Open the Properties dialog for an item or folder.
    Properties(PathBuf),
    /// Properties for an item of the list: with several items selected, one
    /// dialog for the whole selection.
    ItemProperties(PathBuf),
}

/// What this window's Cut or Copy remembered. Where the system pasteboard
/// carries files (`pasteboard::SUPPORTED`) they are on it too, so other windows
/// and Finder can paste them, and this is only the cache of that: `change` is the
/// pasteboard's change count after the write, and the cache is dropped once it
/// moves on. Elsewhere the paths' text is put on the system clipboard instead:
/// egui only reports Cmd+V when it holds text, and a paste only counts while
/// that text is still the clipboard's, i.e. nothing newer was copied elsewhere.
struct Clipboard {
    paths: Vec<PathBuf>,
    cut: bool,
    token: String,
    change: isize,
}

/// The items the clipboard has cut, which the list draws dimmed.
fn cut_paths(clipboard: &Option<Clipboard>) -> &[PathBuf] {
    clipboard.as_ref().filter(|c| c.cut).map_or(&[], |c| &c.paths)
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
    /// The paste's progress dialog (`ProgressDialog::id`).
    transfer: Option<u64>,
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
    /// Whether the latest listing was asked to look for branches, which costs
    /// a look for `.git` in every folder, so it is only done while the Branch
    /// column is shown.
    list_branches: bool,
    /// The open folder's Git status. `git status` can take a while in a big
    /// repository, so it runs on a worker thread, and the summary is kept
    /// while you move around inside the working tree: it is asked for again
    /// only when you enter one, or by a refresh, a file operation and the
    /// like (`reload`).
    git: Option<git::Status>,
    git_tx: Sender<GitResult>,
    git_rx: Receiver<GitResult>,
    git_request: u64,
    /// The request whose summary `git` holds; older answers are dropped.
    git_applied: u64,
    sort: SortSpec,
    settings: Settings,
    selection: BTreeSet<PathBuf>,
    /// Where Shift+click and Shift+arrow selections start.
    anchor: Option<usize>,
    /// The focused item, which arrow keys move from (`entries` index).
    focus_item: Option<usize>,
    /// Focus rectangles are drawn once the keyboard has been used to move
    /// around, like Windows' "hide keyboard navigation indicators" (the
    /// `UISF_HIDEFOCUS` state that Tab, F6 and the arrow keys clear).
    show_focus: bool,
    /// An entry to scroll into view on the next frame.
    reveal: Option<usize>,
    /// Letters typed in the item view, which select the item they spell.
    type_ahead: IncrementalSearch,
    /// The same for the Folders pane.
    tree_type_ahead: IncrementalSearch,
    /// Rows that fit in the Folders pane last frame, for Page Up and Down.
    tree_page: usize,
    nav_layout: Option<NavLayout>,
    /// The widest visible name in the List view, keyed like `visible_cache`.
    widest_name: Option<((u64, bool, String), f32)>,
    /// The rubber band while it's being dragged in the item view.
    marquee: Option<Marquee>,
    /// Items being dragged inside the window (`item_drag`).
    drag: Option<ItemDrag>,
    /// The drop zones drawn this frame, and how the items were drawn.
    drag_frame: DragFrame,
    /// Details view columns and their widths, shared by every folder and
    /// remembered between runs. (Explorer kept them per folder.)
    columns: ColumnSet,
    /// The columns as last read from or written to the settings file, once
    /// `restore_preferences` has run; `None` keeps tests off the user's file.
    saved_columns: Option<ColumnSet>,
    /// The open "More..." dialog.
    column_dialog: Option<ColumnDialog>,
    /// "Choose Program...", while it is open.
    open_with_dialog: Option<OpenWithDialog>,
    about_dialog: Option<AboutDialog>,
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
    /// Give the search field the keyboard, its text selected, next frame.
    search_needs_focus: bool,
    status: Option<String>,
    tree: FolderTree,
    /// Tree folder listings run on worker threads and report back here.
    /// Each result carries the hidden-items setting it was loaded with, so
    /// stale results from before a toggle are dropped.
    load_tx: Sender<LoadResult>,
    load_rx: Receiver<LoadResult>,
    /// `platform::eject` runs on a worker thread; (volume, error text).
    #[cfg_attr(not(unix), allow(dead_code))]
    eject_tx: Sender<(PathBuf, Result<(), String>)>,
    eject_rx: Receiver<(PathBuf, Result<(), String>)>,
    clipboard: Option<Clipboard>,
    /// The system pasteboard Cut and Copy go through where it carries files.
    pb: Pasteboard,
    /// Its change count when `pb_files` was last looked up.
    pb_seen: isize,
    /// Whether it holds files to paste.
    pb_files: bool,
    /// Paste, delete and undo run on worker threads and report back here.
    op_tx: Sender<OpDone>,
    op_rx: Receiver<OpDone>,
    /// A paste asks here when the destination has an item of the same name,
    /// and waits for the answer.
    conflict_tx: Sender<conflict_dialog::Ask>,
    conflict_rx: Receiver<conflict_dialog::Ask>,
    /// The question on screen; the others wait in the channel.
    conflict: Option<ConflictDialog>,
    /// A progress dialog for each paste that is running, or has just ended
    /// and is still on screen.
    transfers: Vec<ProgressDialog>,
    transfer_count: u64,
    /// Reports changes on disk to the open folder and the Folders pane's
    /// expanded folders, which then reload.
    watcher: FolderWatcher,
    /// Set by `restore_preferences`: changes to the view, the sort and the
    /// window size are written to the settings file. Off in tests.
    persist: bool,
    /// The window size as last read from or written to the settings file.
    saved_window: Option<[f32; 2]>,
    /// A new window size and when it was first seen; written once it has
    /// stayed the same for `WINDOW_SAVE_DELAY`.
    window_pending: Option<([f32; 2], f64)>,
    undo: UndoStack,
    pending_select: BTreeSet<PathBuf>,
    /// The listing that brings `pending_select` also starts renaming it (a new folder).
    rename_pending_select: bool,
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
        let (git_tx, git_rx) = channel();
        let (eject_tx, eject_rx) = channel();
        let (op_tx, op_rx) = channel();
        let (conflict_tx, conflict_rx) = channel();
        let wake = ctx.clone();
        let mut app = Self {
            history: History::new(start),
            entries: Vec::new(),
            entries_version: 0,
            visible_cache: None,
            load_error: None,
            free_space: None,
            #[cfg(unix)]
            computer_name: computer_name().unwrap_or_else(|| "My Computer".into()),
            #[cfg(not(unix))]
            computer_name: "My Computer".into(),
            list_tx,
            list_rx,
            list_request: 0,
            listing: false,
            list_branches: false,
            git: None,
            git_tx,
            git_rx,
            git_request: 0,
            git_applied: 0,
            sort: SortSpec::default(),
            settings: Settings { view: ViewMode::Details, show_extensions: true, show_hidden: false, natural_sort: true, mix_folders: false, expand_to_open: true, show_folders: true, color_names: true },
            selection: BTreeSet::new(),
            anchor: None,
            focus_item: None,
            show_focus: false,
            reveal: None,
            type_ahead: IncrementalSearch::default(),
            tree_type_ahead: IncrementalSearch::default(),
            tree_page: 1,
            nav_layout: None,
            widest_name: None,
            marquee: None,
            drag: None,
            drag_frame: DragFrame::default(),
            columns: ColumnSet::default(),
            saved_columns: None,
            column_dialog: None,
            open_with_dialog: None,
            about_dialog: None,
            column_drag: None,
            header_left: 0.0,
            rename: None,
            slow_click: SlowClick::default(),
            address_edit: None,
            address_needs_focus: false,
            search: String::new(),
            search_needs_focus: false,
            status: None,
            tree: FolderTree::new(tree_roots()),
            load_tx,
            load_rx,
            eject_tx,
            eject_rx,
            clipboard: None,
            pb: Pasteboard::general(),
            pb_seen: -1,
            pb_files: false,
            op_tx,
            op_rx,
            conflict_tx,
            conflict_rx,
            conflict: None,
            transfers: Vec::new(),
            transfer_count: 0,
            watcher: FolderWatcher::new(move || wake.request_repaint()),
            persist: false,
            saved_window: None,
            window_pending: None,
            undo: UndoStack::default(),
            pending_select: BTreeSet::new(),
            rename_pending_select: false,
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
        // A press that moves this far is a drag, not a click (`SM_CXDRAG`).
        app.ctx.options_mut(|o| o.input_options.max_click_dist = dragdrop::DRAG_DISTANCE);
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
    #[cfg(unix)]
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
        if pasteboard::SUPPORTED {
            if self.pb.write(&paths, cut) {
                let change = self.pb.change_count().unwrap_or_default();
                (self.pb_seen, self.pb_files) = (change, true);
                self.clipboard = Some(Clipboard { paths, cut, token: String::new(), change });
            }
            return;
        }
        let token = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
        self.ctx.copy_text(token.clone());
        self.clipboard = Some(Clipboard { paths, cut, token, change: 0 });
    }

    /// Notices that something else changed the system pasteboard: whether it
    /// still holds files, and dropping what this window remembered.
    fn refresh_clipboard(&mut self) {
        let Some(change) = self.pb.change_count() else { return };
        if change == self.pb_seen {
            return;
        }
        self.pb_seen = change;
        self.pb_files = self.pb.has_files();
        if self.clipboard.as_ref().is_some_and(|c| c.change != change) {
            self.clipboard = None;
        }
    }

    /// Whether there is something to paste.
    fn can_paste(&self) -> bool {
        if pasteboard::SUPPORTED { self.pb_files } else { self.clipboard.is_some() }
    }

    /// Whether egui's paste event, carrying the clipboard's text, is a paste of
    /// the files this app cut or copied.
    fn is_file_paste(&self, text: &str) -> bool {
        if pasteboard::SUPPORTED { self.pb_files } else { self.clipboard.as_ref().is_some_and(|c| c.token == text) }
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
    /// `move_here` moves even what was only copied, and leaves the copy on the
    /// clipboard like Finder's "Move Item Here".
    fn paste(&mut self, dest: PathBuf, move_here: bool) {
        self.refresh_clipboard();
        let (paths, was_cut) = match &self.clipboard {
            Some(clip) => (clip.paths.clone(), clip.cut),
            // Cut or copied in another window, or in Finder.
            None => match pasteboard::SUPPORTED.then(|| self.pb.read()).flatten() {
                Some(files) => (files.paths, files.cut),
                None => return,
            },
        };
        let cut = was_cut || move_here;
        if was_cut {
            // Like Explorer, a cut can be pasted once.
            self.clipboard = None;
            if pasteboard::SUPPORTED {
                self.pb.clear();
                (self.pb_seen, self.pb_files) = (self.pb.change_count().unwrap_or_default(), false);
            }
        }
        self.transfer(paths, dest, cut);
    }

    /// Copies or (with `cut`) moves `paths` into `dest` on a worker thread,
    /// with the progress dialog and the Replace question: a paste or a drop.
    fn transfer(&mut self, paths: Vec<PathBuf>, dest: PathBuf, cut: bool) {
        self.status = Some(format!("{} {} item(s)...", if cut { "Moving" } else { "Copying" }, paths.len()));
        self.transfer_count += 1;
        let (id, progress) = (self.transfer_count, std::sync::Arc::new(Progress::default()));
        self.transfers.push(ProgressDialog::new(id, progress.clone(), cut));
        let (tx, ctx, ask_tx) = (self.op_tx.clone(), self.ctx.clone(), self.conflict_tx.clone());
        thread::spawn(move || {
            let ask_ctx = ctx.clone();
            let ask = |conflict: &macsplorer_core::fileops::Conflict| {
                let (reply, answer) = channel();
                let folder = conflict.existing.parent().and_then(entry_at);
                let ask = conflict_dialog::Ask { conflict: conflict.clone(), folder, existing: entry_at(&conflict.existing), source: entry_at(&conflict.source), reply };
                if ask_tx.send(ask).is_err() {
                    return Choice::Cancel;
                }
                ask_ctx.request_repaint();
                // The window closing drops the dialog, and with it the sender.
                answer.recv().unwrap_or(Choice::Cancel)
            };
            let result = transfer_with(&paths, &dest, cut, &SystemTrash, ask, &progress);
            let mut dirs = vec![dest.clone()];
            if cut {
                dirs.extend(paths.iter().filter_map(|p| p.parent()).map(Path::to_path_buf));
            }
            let error = result.error.map(|(path, e)| {
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                format!("Couldn't {} \"{name}\": {e}", if cut { "move" } else { "copy" })
            });
            let done = OpDone { dirs, select: result.placed, error, undo: result.undo, transfer: Some(id) };
            if tx.send(done).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// The Open With dialog; OK opens the file and, when asked, remembers
    /// the app for the file's type.
    fn show_open_with(&mut self) {
        let Some(dialog) = &mut self.open_with_dialog else { return };
        match dialog.show(&self.ctx, &mut self.icons) {
            open_with_dialog::Outcome::Keep => {}
            open_with_dialog::Outcome::Cancel => self.open_with_dialog = None,
            open_with_dialog::Outcome::Open { app, always } => {
                if let Some(dialog) = self.open_with_dialog.take() {
                    let file = dialog.file().to_path_buf();
                    if always {
                        open_with::remember(&file, Some(&app));
                    }
                    self.open_with(file, Some(app));
                }
            }
        }
    }

    /// Shows the question a paste is waiting on, taking the next one when
    /// it is answered.
    fn show_conflict(&mut self) {
        if self.conflict.is_none() {
            self.conflict = self.conflict_rx.try_recv().ok().map(ConflictDialog::new);
        }
        let Some(dialog) = &mut self.conflict else { return };
        if let Some(choice) = dialog.show(&self.ctx, &mut self.icons) {
            if let Some(dialog) = self.conflict.take() {
                dialog.answer(choice);
            }
            self.ctx.request_repaint();
        }
    }

    /// The progress dialogs that are due. Their clocks stop while a paste
    /// waits for an answer to a question.
    fn show_transfers(&mut self) {
        let now = self.ctx.input(|i| i.time);
        let paused = self.conflict.is_some();
        let ctx = self.ctx.clone();
        self.transfers.retain_mut(|dialog| dialog.show(&ctx, now, paused));
    }

    /// Makes shortcuts (symbolic links) to `paths` in `dest` on a worker thread.
    #[cfg_attr(not(unix), allow(unused_variables))]
    fn link(&mut self, paths: Vec<PathBuf>, dest: PathBuf) {
        let (tx, ctx) = (self.op_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            #[cfg(unix)]
            let result = fileops::link_into(&paths, &dest);
            #[cfg(not(unix))]
            let result = fileops::Transferred::default();
            let error = result.error.map(|(path, e)| {
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                format!("Couldn't make a shortcut to \"{name}\": {e}")
            });
            let done = OpDone { dirs: vec![dest], select: result.placed, error, undo: result.undo, transfer: None };
            if tx.send(done).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// Files dropped on the window from another app, where the platform has no
    /// `native_drag` (Linux, through winit's X11 file drop, which says neither
    /// where the pointer is nor which keys are down): they go into the open
    /// folder, moved when they are on the same disk and copied otherwise, like
    /// a drop on the folder without keys.
    fn drop_files(&mut self, paths: Vec<PathBuf>) {
        let dest = self.current().to_path_buf();
        match dragdrop::drop_effect(&paths, &dest, None, Allowed::ALL) {
            Some(Effect::Copy) => self.transfer(paths, dest, false),
            Some(Effect::Move) => self.transfer(paths, dest, true),
            Some(Effect::Link) => self.link(paths, dest),
            None => {}
        }
    }

    /// One frame of an item drag, after the views drew their drop zones:
    /// the target under the pointer, the drop, the drag image. A drag from
    /// another app comes through `native_drag`, and one of ours that leaves
    /// the window goes to AppKit there.
    fn drag_step(&mut self, ui: &Ui, actions: &mut Vec<Action>) {
        let incoming = native_drag::incoming();
        match (&incoming, &self.drag) {
            (Some(i), None) => self.drag = Some(ItemDrag::external(i.paths.clone())),
            (None, Some(d)) if d.native => self.drag = None,
            _ => {}
        }
        let Some(drag) = &mut self.drag else { return };
        let (pointer, forced, allowed) = match incoming.filter(|_| drag.native) {
            // AppKit has already narrowed what the source allows to what the keys ask for.
            Some(i) => (Pointer { pos: Some(i.pos), down: !i.dropped, escape: false }, None, i.allowed),
            None => {
                let mods = ui.input(|i| i.modifiers);
                // The keys of the platform's own drags.
                #[cfg(target_os = "macos")]
                let forced = dragdrop::mac_keys(mods.alt, mods.mac_cmd);
                #[cfg(not(target_os = "macos"))]
                let forced = dragdrop::windows_keys(mods.shift, mods.ctrl, mods.alt);
                (Pointer::of(ui), forced, Allowed::ALL)
            }
        };
        let outside = pointer.pos.is_none_or(|p| !ui.ctx().content_rect().contains(p));
        if !drag.native && pointer.down && outside && native_drag::begin(ui.ctx(), &drag.paths) {
            self.drag = None;
            return;
        }
        let native = drag.native;
        let step = drag.step(ui, &self.drag_frame, pointer, |paths, dir| dragdrop::drop_effect(paths, dir, forced, allowed));
        if native {
            native_drag::set_answer(drag.over.as_ref().map(|o| o.effect));
        }
        match step {
            Step::Dragging => drag.paint(ui.ctx(), &mut self.icons, &self.entries),
            Step::Expand(key) => {
                drag.paint(ui.ctx(), &mut self.icons, &self.entries);
                actions.push(Action::TreeExpand(key));
            }
            Step::Drop { dir, effect } => {
                actions.push(Action::Drop { dir, effect });
                native_drag::set_answer(None);
            }
            Step::Cancel => {
                self.drag = None;
                native_drag::set_answer(None);
            }
        }
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
            let mut done = OpDone { dirs: vec![dir], select: Vec::new(), error: None, undo: None, transfer: None };
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
            let mut done = OpDone { dirs: op.dirs(), select: Vec::new(), error: None, undo: None, transfer: None };
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
            for dialog in &mut self.transfers {
                dialog.finished |= done.transfer == Some(dialog.id);
            }
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

    /// Reloads the folders that changed on disk, and watches the ones on
    /// screen now.
    fn poll_watcher(&mut self) {
        let now = self.ctx.input(|i| i.time);
        self.poll_watcher_at(now);
    }

    fn poll_watcher_at(&mut self, now: f64) {
        let current = self.current().to_path_buf();
        let mut dirs: BTreeSet<PathBuf> = self.tree.expanded_folders().map(Path::to_path_buf).collect();
        dirs.insert(current.clone());
        self.watcher.watch_only(&dirs);
        for dir in self.watcher.poll(now) {
            if dir == current {
                self.reload();
            }
            if let Some(path) = self.tree.refresh_path(&dir) {
                self.load([path]);
            }
        }
        if let Some(due) = self.watcher.next_due() {
            self.ctx.request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.0)));
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

    /// Lists the open folder and asks for its Git status afresh.
    fn reload(&mut self) {
        self.reload_list();
        self.reload_git(false);
    }

    /// Lists the open folder on a worker thread; `poll_lists` applies the
    /// result. The current entries stay up until then.
    fn reload_list(&mut self) {
        self.list_request += 1;
        self.listing = true;
        self.list_branches = self.columns.is_visible(Column::Branch);
        let (request, dir, show_hidden, branches) = (self.list_request, self.current().to_path_buf(), self.settings.show_hidden, self.list_branches);
        let (tx, ctx) = (self.list_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = list_dir_with(&dir, show_hidden, branches).map_err(|e| platform::open_error(&dir, &e));
            #[cfg(unix)]
            let free = free_space(&dir);
            #[cfg(not(unix))]
            let free = None;
            if tx.send((request, result, free)).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// Asks a worker thread for the open folder's Git status. With `reuse`
    /// (after navigating) the summary that is up stays if the folder is in the
    /// same working tree.
    fn reload_git(&mut self, reuse: bool) {
        self.git_request += 1;
        let known = self.git.as_ref().filter(|_| reuse).map(|g| g.root.clone());
        let (request, dir) = (self.git_request, self.current().to_path_buf());
        let (tx, ctx) = (self.git_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let update = match known {
                Some(root) if git::repo_root(&dir).as_ref() == Some(&root) => GitUpdate::Keep,
                _ => GitUpdate::Set(git::status(&dir)),
            };
            if tx.send((request, update)).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// The Branch column was turned on, or git turned up, after the listing was
    /// made without branches: list again with them.
    fn list_branches_if_missing(&mut self) {
        if self.columns.is_visible(Column::Branch) && !self.list_branches {
            self.reload_list();
        }
    }

    fn poll_git(&mut self) {
        while let Ok(result) = self.git_rx.try_recv() {
            self.git_received(result);
        }
    }

    /// A summary replaces the one up unless a newer one has been applied
    /// already. A `Keep` changes nothing, so it can't cancel a refresh that
    /// is still running.
    fn git_received(&mut self, (request, update): GitResult) {
        if let GitUpdate::Set(status) = update
            && request > self.git_applied
        {
            self.git = status;
            self.git_applied = request;
        }
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
        // A reload (after a change on disk, say) can move items around:
        // focus and anchor stay on the same items.
        let path_at = |i: Option<usize>| i.and_then(|i| self.entries.get(i)).map(|e| e.path.clone());
        let (focus, anchor) = (path_at(self.focus_item), path_at(self.anchor));
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
        let index_of = |p: Option<PathBuf>| p.and_then(|p| self.entries.iter().position(|e| e.path == p));
        (self.focus_item, self.anchor) = (index_of(focus), index_of(anchor));
        let present: BTreeSet<_> = self.entries.iter().map(|e| &e.path).collect();
        self.selection.retain(|p| present.contains(p));
        let rename_picked = std::mem::take(&mut self.rename_pending_select);
        let picked: Vec<PathBuf> = std::mem::take(&mut self.pending_select).into_iter().filter(|p| present.contains(p)).collect();
        if !picked.is_empty() {
            self.selection = picked.into_iter().collect();
            // Pasted or dropped items: the first one gets the focus and is
            // scrolled into view, as Explorer selects them
            // (`SVSI_FOCUSED | SVSI_ENSUREVISIBLE`, from memory).
            let first = self.entries.iter().position(|e| self.selection.contains(&e.path));
            (self.focus_item, self.anchor, self.reveal) = (first, first, first);
            if rename_picked {
                self.begin_rename();
            }
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
        // Inside the same working tree the summary stays up until the new one arrives.
        if self.git.as_ref().is_some_and(|g| !self.current().starts_with(&g.root)) {
            self.git = None;
        }
        self.reload_list();
        self.reload_git(true);
        if !std::mem::take(&mut self.navigating_from_tree) {
            // Going somewhere else drops a tree selection still waiting to open.
            self.tree.take_open();
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

    /// New > Folder: makes "New Folder" in the open folder and starts renaming
    /// it once the listing shows it, as Explorer does (the name is left to the
    /// user). Like Explorer's, the creation has no undo step.
    fn new_folder(&mut self) {
        let path = fileops::new_folder_name(self.current());
        match std::fs::create_dir(&path) {
            Ok(()) => {
                self.status = None;
                self.pending_select = BTreeSet::from([path]);
                self.rename_pending_select = true;
                self.reload();
            }
            Err(e) => self.status = Some(format!("Couldn't create the folder: {e}")),
        }
    }

    /// Address bar entry: a folder is opened, an existing file is revealed
    /// (its folder opens with the file selected), anything else reports the error.
    fn go_to(&mut self, path: PathBuf) {
        if !path.is_dir() && path.symlink_metadata().is_ok() {
            self.reveal_in_view(path);
        } else {
            self.navigate(path);
        }
    }

    /// Goes to the folder holding `path` and selects it there, as the pasted
    /// items are (`pending_select`, taken when the listing arrives).
    fn reveal_in_view(&mut self, path: PathBuf) {
        let Some(folder) = path.parent().map(Path::to_path_buf) else { return };
        self.pending_select = BTreeSet::from([path]);
        self.navigate(folder);
    }

    fn open(&mut self, path: PathBuf) {
        let is_folder = self.entries.iter().find(|e| e.path == path).map_or(path.is_dir(), Entry::is_folder);
        if is_folder {
            self.navigate(path);
        } else if let Some(app) = open_with::chosen_app(&path) {
            self.open_with(path, Some(app));
        } else if let Err(e) = platform::open_default(&path) {
            self.status = Some(format!("Couldn't open {}: {e}", path.display()));
        }
    }

    fn open_with(&mut self, path: PathBuf, app: Option<PathBuf>) {
        let Some(app) = app else {
            // Windows' "Choose Program..." dialog.
            if self.open_with_dialog.is_none() {
                let apps = open_with::apps_for(&path);
                self.open_with_dialog = Some(OpenWithDialog::new(path, apps));
            }
            return;
        };
        if let Err(e) = Command::new("open").arg("-a").arg(&app).arg(&path).spawn() {
            self.status = Some(format!("Couldn't open {} with {}: {e}", path.display(), open_with::app_name(&app)));
        }
    }

    fn tree_select(&mut self, key: NodeKey) {
        self.focus = Pane::Tree;
        let path = key.path.clone();
        self.tree.take_open();
        self.tree.select(key);
        self.open_from_tree(path);
    }

    /// Selects a tree row from the keyboard. The highlight moves now, the
    /// folder opens once the keys pause (`tree::OPEN_DELAY_MS`).
    fn tree_key_select(&mut self, key: NodeKey) {
        self.focus = Pane::Tree;
        self.tree.select_from_keyboard(key, now_ms(&self.ctx));
        self.ctx.request_repaint();
    }

    /// Opens a keyboard selection in the tree once its delay is over.
    fn open_due_tree_selection(&mut self) {
        let now = now_ms(&self.ctx);
        if let Some(path) = self.tree.due_open(now) {
            self.open_from_tree(path);
        } else if let Some(ms) = self.tree.open_wait_ms(now) {
            self.ctx.request_repaint_after(std::time::Duration::from_millis(ms));
        }
    }

    fn open_from_tree(&mut self, path: PathBuf) {
        if path != self.current() {
            self.navigating_from_tree = true;
            self.navigate(path);
            self.navigating_from_tree = false;
        }
    }

    /// Which pane has the keyboard, as a stop of the Tab cycle.
    fn current_stop(&self) -> Stop {
        match self.ctx.memory(|m| m.focused()) {
            Some(id) if id == address_id() => Stop::Address,
            Some(id) if id == search_id() => Stop::Search,
            _ if self.focus == Pane::Tree => Stop::Folders,
            _ => Stop::View,
        }
    }

    /// Gives a pane the keyboard (`CShellBrowser::_CycleFocus`). The fields
    /// take it with their text selected, as an edit control does on Tab.
    fn focus_stop(&mut self, stop: Stop, visible: &[usize]) {
        self.show_focus = true;
        self.slow_click.cancel();
        match stop {
            Stop::View | Stop::Folders => {
                self.ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                self.focus = if stop == Stop::Folders { Pane::Tree } else { Pane::List };
                // Something to show the focus on. **Inferred**: the list view
                // focuses its first item when it gets the focus with none.
                if stop == Stop::View && self.focus_item.is_none_or(|f| !visible.contains(&f)) {
                    self.focus_item = visible.first().copied();
                }
            }
            Stop::Address => {
                if self.address_edit.is_none() {
                    self.address_edit = Some(self.current().display().to_string());
                }
                self.address_needs_focus = true;
            }
            Stop::Search => self.search_needs_focus = true,
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
            // The new name sorts elsewhere: the listing on its way scrolls to it.
            self.pending_select = BTreeSet::from([target.clone()]);
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

    /// Like Explorer, several selected items get one dialog with their
    /// combined size; `path` is the item the command was given on.
    fn open_item_properties(&mut self, path: PathBuf) {
        if self.selection.len() > 1 && self.selection.contains(&path) {
            let entries: Vec<Entry> = self.entries.iter().filter(|e| self.selection.contains(&e.path)).cloned().collect();
            if entries.len() > 1 {
                self.properties.push(Properties::new_multi(&self.ctx, entries, self.settings.show_extensions));
                return;
            }
        }
        self.open_properties(path);
    }

    fn open_properties(&mut self, path: PathBuf) {
        let entry = self.entries.iter().find(|e| e.path == path).cloned().unwrap_or_else(|| Properties::folder_entry(&path));
        // The root and mounted volumes can't be renamed from here.
        #[cfg(unix)]
        let editable = path.parent().is_some() && !is_ejectable(&path);
        #[cfg(not(unix))]
        let editable = path.parent().is_some();
        self.properties.push(Properties::new(&self.ctx, entry, self.settings.show_extensions, editable));
    }

    /// Draws the open dialogs and carries out their OK, Cancel and Apply.
    fn show_properties(&mut self, ctx: &egui::Context) {
        let mut dialogs = std::mem::take(&mut self.properties);
        dialogs.retain_mut(|dialog| match dialog.show(ctx, &mut self.icons) {
            Outcome::Keep => true,
            Outcome::Close => false,
            Outcome::Apply { close } => {
                // Like a failed PSN_APPLY, the dialog stays open when a step fails.
                if let Some(name) = dialog.name_to_apply() {
                    let path = dialog.path().to_path_buf();
                    match self.rename_item(&path, &name) {
                        Some(target) => dialog.renamed(target),
                        None => return true,
                    }
                }
                #[cfg(unix)]
                if let Some(bits) = dialog.new_permissions() {
                    match set_permissions(dialog.path(), bits) {
                        Ok(()) => {
                            self.status = None;
                            dialog.permissions_applied();
                            self.reload();
                        }
                        Err(e) => {
                            self.status = Some(format!("Couldn't change the permissions: {e}"));
                            return true;
                        }
                    }
                }
                if let Some(style) = dialog.new_style() {
                    match macsplorer_core::folder_style::write(dialog.path(), &style) {
                        Ok(()) => {
                            self.status = None;
                            dialog.style_applied();
                            self.icons.forget_native(dialog.path());
                            self.reload();
                        }
                        Err(e) => {
                            self.status = Some(format!("Couldn't customize the folder: {e}"));
                            return true;
                        }
                    }
                }
                !close
            }
        });
        // Dialogs opened while these were drawn come after them.
        dialogs.append(&mut self.properties);
        self.properties = dialogs;
    }

    /// Takes the stored preferences and from now on writes changes back.
    /// (Dark mode is applied by `main`, before the window exists.)
    pub fn restore_preferences(&mut self, prefs: &Preferences) {
        if let Some(columns) = &prefs.columns {
            self.columns = columns.clone();
        }
        self.saved_columns = Some(self.columns.clone());
        self.settings.view = prefs.view;
        if let Some(sort) = prefs.sort {
            (self.sort.column, self.sort.ascending) = (sort.column, sort.ascending);
            self.resort();
        }
        self.saved_window = prefs.window;
        self.persist = true;
        // Finding git can take a moment (and `main` has told the lookup where
        // it was last time): not on this thread. When it is done the Branch
        // column can appear, and the settings file learns where git is.
        let (known, ctx) = (prefs.git.clone(), self.ctx.clone());
        thread::spawn(move || {
            let found = git::executable().map(Path::to_path_buf);
            if found != known {
                Preferences::update(|p| p.git = found);
            }
            ctx.request_repaint();
        });
    }

    /// Writes the window's size once a resize has ended. Not while the
    /// window is maximized or full screen, which isn't a size to come back
    /// to. The size is in macOS points: Text Size zooms egui's points.
    fn save_window_size(&mut self) {
        if !self.persist {
            return;
        }
        let (rect, maximized, now) = self.ctx.input(|i| (i.viewport().inner_rect, i.viewport().maximized == Some(true) || i.viewport().fullscreen == Some(true), i.time));
        let Some(rect) = rect.filter(|_| !maximized) else { return };
        let zoom = self.ctx.zoom_factor();
        let size = [(rect.width() * zoom).round(), (rect.height() * zoom).round()];
        if Some(size) == self.saved_window {
            self.window_pending = None;
            return;
        }
        match self.window_pending {
            Some((pending, since)) if pending == size => {
                let waited = now - since;
                if waited >= WINDOW_SAVE_DELAY {
                    Preferences::update(|p| p.window = Some(size));
                    self.saved_window = Some(size);
                    self.window_pending = None;
                } else {
                    self.ctx.request_repaint_after(std::time::Duration::from_secs_f64(WINDOW_SAVE_DELAY - waited));
                }
            }
            _ => {
                self.window_pending = Some((size, now));
                self.ctx.request_repaint_after(std::time::Duration::from_secs_f64(WINDOW_SAVE_DELAY));
            }
        }
    }

    /// Writes the column layout when it changed, but not while a divider is
    /// being dragged: only the width the drag ends on is worth keeping.
    fn save_columns(&mut self) {
        if self.column_drag.is_some() || self.saved_columns.as_ref().is_none_or(|saved| *saved == self.columns) {
            return;
        }
        let columns = self.columns.clone();
        Preferences::update(|p| p.columns = Some(columns.clone()));
        self.saved_columns = Some(columns);
    }

    fn apply(&mut self, actions: Vec<Action>, visible: &[usize]) {
        for action in actions {
            match action {
                Action::Navigate(p) => self.navigate(p),
                Action::GoTo(p) => self.go_to(p),
                Action::NewFolder => self.new_folder(),
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
                Action::EndTypeAhead => {
                    self.type_ahead.reset();
                    self.tree_type_ahead.reset();
                }
                Action::TypeAhead { text, time_ms, extend } => {
                    let show_extensions = self.settings.show_extensions;
                    for c in text.chars() {
                        let focus = self.focus_item.and_then(|f| visible.iter().position(|&v| v == f));
                        let names: Vec<&str> = visible.iter().map(|&i| self.entries[i].display_name(show_extensions)).collect();
                        match self.type_ahead.type_char(c, time_ms, &names, focus) {
                            isearch::Outcome::Found(pos) => {
                                self.click(visible[pos], false, false, visible);
                                self.reveal = Some(visible[pos]);
                            }
                            // Space toggles the focused item; Shift+Space selects
                            // from the anchor to it (`CLVKeyboardManager::OnKey`).
                            isearch::Outcome::Space => {
                                if let Some(pos) = focus {
                                    self.slow_click.cancel();
                                    self.click(visible[pos], !extend, extend, visible);
                                    self.show_focus = true;
                                }
                            }
                            isearch::Outcome::NotFound | isearch::Outcome::Ignored => {}
                        }
                    }
                }
                Action::TreeTypeAhead { text, time_ms } => {
                    let rows = self.tree.rows();
                    let real: Vec<&Row> = rows.iter().filter(|r| !r.placeholder).collect();
                    for c in text.chars() {
                        let focus = real.iter().position(|r| r.selected);
                        let names: Vec<&str> = real.iter().map(|r| r.label.as_str()).collect();
                        if let isearch::Outcome::Found(pos) = self.tree_type_ahead.type_char(c, time_ms, &names, focus) {
                            self.tree_key_select(real[pos].key.clone());
                            break;
                        }
                    }
                }
                Action::FocusStop(stop) => self.focus_stop(stop, visible),
                Action::ShowFocus => self.show_focus = true,
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
                Action::DragItems(index) => {
                    self.slow_click.cancel();
                    self.focus = Pane::List;
                    // A press on an item outside the selection drags that item alone.
                    if !self.selection.contains(&self.entries[index].path) {
                        self.click(index, false, false, visible);
                    }
                    let paths = visible.iter().map(|&i| &self.entries[i].path).filter(|p| self.selection.contains(*p)).cloned().collect();
                    self.drag = Some(ItemDrag::new(&self.ctx, paths, &self.drag_frame));
                }
                Action::DragFolder(path) => self.drag = Some(ItemDrag::new(&self.ctx, vec![path], &self.drag_frame)),
                Action::Drop { dir, effect } => {
                    if let Some(drag) = self.drag.take() {
                        match effect {
                            Effect::Copy => self.transfer(drag.paths, dir, false),
                            Effect::Move => self.transfer(drag.paths, dir, true),
                            Effect::Link => self.link(drag.paths, dir),
                        }
                    }
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
                    if self.persist {
                        let sort = SortOrder { column: self.sort.column, ascending: self.sort.ascending };
                        Preferences::update(|p| p.sort = Some(sort));
                    }
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
                Action::NewWindow => {
                    let frame = self.ctx.input(|i| i.viewport().outer_rect);
                    if let Err(e) = platform::new_window(self.current(), frame) {
                        self.status = Some(format!("Could not open a new window: {e}"));
                    }
                }
                Action::RevealInFinder(p) => {
                    thread::spawn(move || platform::reveal(&p));
                }
                Action::OpenConfig => {
                    let opened = Preferences::file_path().and_then(|file| platform::open_text(&file));
                    if let Err(e) = opened {
                        self.status = Some(format!("Couldn't open the config file: {e}"));
                    }
                }
                #[cfg(unix)]
                Action::OpenTerminal(p) => {
                    if let Err(e) = platform::open_terminal(&p, Preferences::load().terminal.as_deref()) {
                        self.status = Some(format!("Couldn't open a terminal in {}: {e}", p.display()));
                    }
                }
                #[cfg(unix)]
                Action::Eject(p) => self.eject(p),
                Action::Cut => self.set_clipboard(true),
                Action::Copy => self.set_clipboard(false),
                Action::CopyAsPath => self.copy_as_path(),
                Action::Paste(dir) => {
                    let dir = dir.unwrap_or_else(|| self.current().to_path_buf());
                    self.paste(dir, false);
                }
                Action::MoveHere(dir) => {
                    let dir = dir.unwrap_or_else(|| self.current().to_path_buf());
                    self.paste(dir, true);
                }
                Action::Delete => self.delete_selection(),
                Action::TreeSelect(key) => self.tree_select(key),
                Action::TreeKeySelect(key) => self.tree_key_select(key),
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
                Action::ItemProperties(path) => self.open_item_properties(path),
                Action::SetDark(dark) => {
                    theme::set_dark(&self.ctx, dark);
                    Preferences::update(|p| p.dark = dark);
                    self.icons.palette_changed();
                }
                Action::SetZoom(zoom) => {
                    self.ctx.set_zoom_factor(zoom);
                    Preferences::update(|p| p.zoom = zoom);
                }
                Action::SetView(mode) => {
                    self.settings.view = mode;
                    if self.persist {
                        Preferences::update(|p| p.view = mode);
                    }
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
        // Cmd +/-/0 work everywhere, like egui's own zoom keys they replace
        // (turned off in `main`), but step through the Text Size choices.
        use egui::gui_zoom::kb_shortcuts as keys;
        let zoom = ui.ctx().zoom_factor();
        let target = ui.input_mut(|i| {
            if i.consume_shortcut(&keys::ZOOM_RESET) {
                Some(text_size::NORMAL)
            } else if i.consume_shortcut(&keys::ZOOM_IN) || i.consume_shortcut(&keys::ZOOM_IN_SECONDARY) {
                Some(text_size::larger(zoom))
            } else if i.consume_shortcut(&keys::ZOOM_OUT) {
                Some(text_size::smaller(zoom))
            } else {
                None
            }
        });
        if let Some(target) = target.filter(|&t| t != zoom) {
            actions.push(Action::SetZoom(target));
        }
        // Tab and F6 move between the panes, not through egui's widgets
        // (which would park the keyboard on an invisible button).
        ui.ctx().memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        let (cycle, address, find) = ui.input(|i| {
            let m = i.modifiers;
            let cycle = focus::direction(i.key_pressed(Key::Tab) || i.key_pressed(Key::F6), m.ctrl, m.alt || m.mac_cmd, m.shift);
            // Explorer's F4 and Alt+D (below), and the browsers' Cmd+L.
            let address = i.key_pressed(Key::F4) || (m.command && i.key_pressed(Key::L));
            // Explorer's F3 and Ctrl+F, Finder's Cmd+F.
            let find = i.key_pressed(Key::F3) || (m.command && i.key_pressed(Key::F));
            (cycle, address, find)
        });
        if let Some(forward) = cycle
            && self.rename.is_none()
        {
            let show_folders = self.settings.show_folders;
            actions.push(Action::FocusStop(focus::next(self.current_stop(), forward, |s| s != Stop::Folders || show_folders)));
        }
        if address {
            actions.push(Action::FocusStop(Stop::Address));
        }
        if find {
            actions.push(Action::FocusStop(Stop::Search));
        }
        // Text fields own the rest of the keyboard while they are focused.
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return;
        }
        ui.input(|i| {
            let cmd = i.modifiers.command;
            let alt = i.modifiers.alt;
            if alt && !cmd && i.key_pressed(Key::D) {
                actions.push(Action::FocusStop(Stop::Address));
                return;
            }
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
            // Option+Cmd+V is Finder's "Move Item Here"; egui reports it as a Paste too.
            let pasted = i.events.iter().any(|e| matches!(e, egui::Event::Paste(text) if self.is_file_paste(text)));
            if pasted {
                actions.push(if alt { Action::MoveHere(None) } else { Action::Paste(None) });
            }
            // New Window: Win+E in Explorer, so Cmd+E on macOS and Super+E on Linux
            // (egui has no Super modifier there, so look at the key itself). Cmd+N
            // is the Finder's and keeps working.
            let win_mod = if cfg!(target_os = "macos") { cmd } else { i.key_down(Key::SuperLeft) || i.key_down(Key::SuperRight) };
            if !i.modifiers.shift && ((win_mod && i.key_pressed(Key::E)) || (cmd && i.key_pressed(Key::N))) {
                actions.push(Action::NewWindow);
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
                actions.push(Action::ItemProperties(self.properties_target(in_list)));
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
                    let mut moved = false;
                    for key in TREE_KEYS {
                        if i.key_pressed(key) {
                            moved = true;
                            actions.extend(tree_key_action(&rows, key, self.tree_page));
                        }
                    }
                    if moved || [Key::Enter, Key::Escape, Key::Backspace].into_iter().any(|k| i.key_pressed(k)) {
                        actions.push(Action::EndTypeAhead);
                    }
                    if moved {
                        actions.push(Action::ShowFocus);
                    } else {
                        let text: String = i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
                        if !text.is_empty() {
                            actions.push(Action::TreeTypeAhead { text, time_ms: (i.time * 1000.0) as u64 });
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
            // Explorer's Ctrl+arrow and Ctrl+Space (focus only, toggle) are
            // left out: on the Mac they are system shortcuts.
            if cmd || i.modifiers.ctrl || visible.is_empty() {
                return;
            }
            let text: String = i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
            if !text.is_empty() {
                actions.push(Action::TypeAhead { text, time_ms: (i.time * 1000.0) as u64, extend: i.modifiers.shift });
            }
            if alt {
                return;
            }
            let Some(nav) = KEYS.iter().find(|(k, _)| i.key_pressed(*k)).map(|&(_, n)| n) else { return };
            if let Some(to) = self.navigate_items(nav, visible) {
                actions.push(Action::Click { index: visible[to], toggle: false, extend: i.modifiers.shift, on_label: false });
                actions.push(Action::Reveal(visible[to]));
            }
            actions.push(Action::ShowFocus);
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
            (ViewMode::Details, layout) => {
                // A page is one row less than fit, as in the tree view
                // (`TV_KeyDown`). **Inferred**: the report view's paging wasn't traced.
                let page = match layout {
                    Some(NavLayout::Details { rows }) => rows.saturating_sub(1).max(1),
                    _ => 1,
                };
                match nav {
                    Nav::Up => Some(from.saturating_sub(1)),
                    Nav::Down => Some((from + 1).min(last)),
                    Nav::Home => Some(0),
                    Nav::End => Some(last),
                    Nav::PageUp => Some(from.saturating_sub(page)),
                    Nav::PageDown => Some((from + page).min(last)),
                    Nav::Left | Nav::Right => None,
                }
            }
            _ => None,
        }
    }

    // ---- menu bar, toolbar and address bar --------------------------------

    fn menu_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::MenuBar::new().config(egui::containers::menu::MenuConfig::new().style(theme::menu_style)).ui(ui, |ui| {
            bar_menu_button(ui, "File", |ui| {
                if ui.add(gutter_item(ui, "New Window", platform::NEW_WINDOW_KEYS.to_owned())).clicked() {
                    actions.push(Action::NewWindow);
                }
                ui.separator();
                if ui.add(gutter_item(ui, "Properties", platform::shortcut(false, "I"))).clicked() {
                    actions.push(Action::ItemProperties(self.properties_target(self.focus == Pane::List)));
                }
                ui.separator();
                if ui.add(gutter_item(ui, "Close", String::new())).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            bar_menu_button(ui, "Edit", |ui| {
                let undo = self.undo.peek().map_or_else(|| "Undo".to_owned(), |op| format!("Undo {}", op.label()));
                if ui.add_enabled(self.undo.peek().is_some(), gutter_item(ui, &undo, platform::shortcut(false, "Z"))).clicked() {
                    actions.push(Action::Undo);
                }
                ui.separator();
                let has_selection = !self.selection.is_empty();
                if ui.add_enabled(has_selection, gutter_item(ui, "Cut", platform::shortcut(false, "X"))).clicked() {
                    actions.push(Action::Cut);
                }
                if ui.add_enabled(has_selection, gutter_item(ui, "Copy", platform::shortcut(false, "C"))).clicked() {
                    actions.push(Action::Copy);
                }
                if ui.add_enabled(has_selection, gutter_item(ui, "Copy as Path", platform::shortcut(true, "C"))).clicked() {
                    actions.push(Action::CopyAsPath);
                }
                if ui.add_enabled(self.can_paste(), gutter_item(ui, "Paste", platform::shortcut(false, "V"))).clicked() {
                    actions.push(Action::Paste(None));
                }
                if ui.add_enabled(self.can_paste(), gutter_item(ui, "Move Here", platform::shortcut(true, "V"))).clicked() {
                    actions.push(Action::MoveHere(None));
                }
                ui.separator();
                if ui.add_enabled(has_selection, gutter_item(ui, "Delete", platform::shortcut(false, if cfg!(target_os = "macos") { "⌫" } else { "Backspace" }))).clicked() {
                    actions.push(Action::Delete);
                }
                ui.separator();
                if ui.add(gutter_item(ui, "Select All", platform::shortcut(false, "A"))).clicked() {
                    actions.push(Action::SelectAll);
                }
                if ui.add(gutter_item(ui, "Invert Selection", String::new())).clicked() {
                    actions.push(Action::InvertSelection);
                }
            });
            bar_menu_button(ui, "View", |ui| {
                delayed_menu_button(ui, "Explorer Bar", true, |ui| {
                    if egui_classic_menu::checkbox(ui, &mut self.settings.show_folders, "Folders").changed() {
                        ui.close();
                    }
                });
                ui.separator();
                view_choices(ui, self.settings.view, actions);
                ui.separator();
                delayed_menu_button(ui, "Text Size", true, |ui| {
                    let current = text_size::snap(ui.ctx().zoom_factor());
                    for (zoom, label) in text_size::SIZES {
                        if egui_classic_menu::radio(ui, current == zoom, label).clicked() {
                            actions.push(Action::SetZoom(zoom));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                let mut changed = false;
                changed |= egui_classic_menu::checkbox(ui, &mut self.settings.show_extensions, "File name extensions").changed();
                changed |= egui_classic_menu::checkbox(ui, &mut self.settings.show_hidden, "Hidden files and folders").changed();
                egui_classic_menu::checkbox(ui, &mut self.settings.color_names, "Special files in color");
                let mut dark = theme::is_dark();
                if egui_classic_menu::checkbox(ui, &mut dark, "Dark mode").changed() {
                    actions.push(Action::SetDark(dark));
                    ui.close();
                }
                if egui_classic_menu::checkbox(ui, &mut self.settings.expand_to_open, "Expand to open folder").changed() && self.settings.expand_to_open {
                    self.sync_tree();
                }
                if egui_classic_menu::checkbox(ui, &mut self.settings.natural_sort, "Natural number sorting").changed() {
                    self.sort.natural = self.settings.natural_sort;
                    self.resort();
                }
                if egui_classic_menu::checkbox(ui, &mut self.settings.mix_folders, "Mix folders and files").changed() {
                    self.sort.folders_first = !self.settings.mix_folders;
                    self.resort();
                }
                if changed {
                    self.rename = None;
                    self.reload();
                    let loads = self.tree.reset_children();
                    self.load(loads);
                }
                ui.separator();
                if ui.add(gutter_item(ui, "Refresh", platform::shortcut(false, "R"))).clicked() {
                    actions.push(Action::Refresh);
                }
            });
            bar_menu_button(ui, "Help", |ui| {
                if ui.add(gutter_item(ui, "Open config file", String::new())).clicked() {
                    actions.push(Action::OpenConfig);
                }
                ui.separator();
                if ui.add(gutter_item(ui, "About Macsplorer", String::new())).clicked() {
                    self.about_dialog.get_or_insert_with(AboutDialog::new);
                    ui.close();
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
            egui::Popup::menu(&arrow).style(theme::menu_style).show(|ui| {
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
            egui::Popup::menu(&arrow).style(theme::menu_style).show(|ui| {
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
            egui::Popup::menu(&views).style(theme::menu_style).show(|ui| view_choices(ui, self.settings.view, actions));
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
                    Some(text) => actions.push(Action::GoTo(expand_tilde(text.trim(), self.home.as_deref()))),
                    None => actions.push(Action::Refresh),
                }
            }
            ui.add_space(4.0);
            let (srect, _) = ui.allocate_exact_size(egui::vec2(search_w, 22.0), Sense::hover());
            let sinner = theme::sunken_field(ui.painter(), srect);
            let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(sinner.shrink2(egui::vec2(3.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
            let id = search_id();
            let resp = sui.add(TextEdit::singleline(&mut self.search).id(id).hint_text("Search this folder").frame(egui::Frame::NONE).desired_width(f32::INFINITY));
            if std::mem::take(&mut self.search_needs_focus) {
                resp.request_focus();
                select_all(&sui, id, &self.search);
            }
        });
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
            let id = address_id();
            let resp = ui.add(TextEdit::singleline(text).id(id).desired_width(f32::INFINITY).frame(egui::Frame::NONE));
            if std::mem::take(&mut self.address_needs_focus) {
                resp.request_focus();
                select_all(ui, id, text);
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    actions.push(Action::GoTo(expand_tilde(text.trim(), self.home.as_deref())));
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
            egui::Popup::menu(&overflow).style(theme::menu_style).show(|ui| {
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
            egui::Popup::menu(&chevron).style(theme::menu_style).show(|ui| {
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
        let focused = self.focus == Pane::Tree && window_focused(ui) && !text_field_focused(ui);
        let cue = focused && self.show_focus;
        let (bars, viewport) = Bars::begin(ui, ui.id().with("tree-bars"), ui.max_rect(), true, TREE_ROW_HEIGHT);
        self.tree_page = ((viewport.height() / TREE_ROW_HEIGHT).floor() as usize).max(1);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down(Align::Min)));
        let hidden = scrollbar::hidden(&mut ui);
        // Selections scroll into view in one jump, as in the tree view.
        let (drag, frame) = (self.drag.as_ref(), &mut self.drag_frame);
        let out = egui::ScrollArea::both().auto_shrink(false).animated(false).scroll_bar_visibility(hidden).show(&mut ui, |ui| {
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
                tree_row(ui, &mut self.icons, row, guide, scroll, focused, cue, (drag, &mut *frame), actions);
            }
            if drag.is_some() {
                let content = ui.min_rect().union(ui.max_rect().with_max_y(ui.max_rect().top()));
                item_drag::edge_scroll(ui, ui.clip_rect(), content, TREE_ROW_HEIGHT, 1);
            }
        });
        bars.end(&mut ui, &out);
    }

    // ---- item view -----------------------------------------------------

    fn item_view(&mut self, ui: &mut Ui, visible: &[usize], actions: &mut Vec<Action>) {
        // Empty space in the view drops into the open folder.
        let rect = ui.available_rect_before_wrap();
        self.drag_frame.zone(ui.clip_rect(), rect, Some(self.current().to_path_buf()), None, None);
        match self.settings.view {
            ViewMode::LargeIcons => self.icons_view(ui, visible, actions),
            ViewMode::List => self.list_view(ui, visible, actions),
            ViewMode::Details => self.details(ui, visible, actions),
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
        let pitch = LIST_ROW_HEIGHT + ui.spacing().item_spacing.y;
        self.nav_layout = Some(NavLayout::Details { rows: (((viewport.height() - LIST_HEADER_HEIGHT) / pitch).floor() as usize).max(1) });
        let focused = self.focus == Pane::List && window_focused(ui) && !text_field_focused(ui);
        let cue = focused && self.show_focus;
        let can_paste = self.can_paste();
        let folder = self.current().to_path_buf();
        let cut = cut_paths(&self.clipboard);
        let reveal = self.reveal.take().and_then(|r| visible.iter().position(|&v| v == r));
        let menu_id = ui.id().with("details-menu");
        let shown = self.columns.visible();
        let Self { entries, selection, rename, sort, settings, search, icons, open_with, listing, marquee, anchor, columns, header_left, focus_item, drag, drag_frame, .. } = self;
        // Classic list views don't highlight rows on hover.
        ui.visuals_mut().widgets.hovered.bg_fill = theme::color::window();
        ui.spacing_mut().item_spacing.x = 0.0;
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
        egui::Popup::context_menu(&bg).style(theme::menu_style).show(|ui| background_menu(ui, &folder, can_paste, actions));

        let header = |ui: &mut Ui, column: Column, actions: &mut Vec<Action>| {
            let label = column.caption();
            let right = column.right_aligned();
            let rect = ui.max_rect();
            // Over a divider a press resizes instead of sorting.
            let sense = if header_hit.divider().is_some() { Sense::hover() } else { Sense::click() };
            let resp = ui.interact(rect, ui.id().with(label), sense);
            let p = ui.painter();
            p.rect_filled(rect, 0.0, theme::color::face());
            let down = resp.is_pointer_button_down_on();
            theme::bevel(p, rect, if down { Bevel::SunkenThin } else { Bevel::Raised });
            let off = if down { 1.0 } else { 0.0 };
            let font = egui::TextStyle::Body.resolve(ui.style());
            let (pos, align) = if right {
                (egui::pos2(rect.right() - 6.0 + off, rect.center().y + off), egui::Align2::RIGHT_CENTER)
            } else {
                (egui::pos2(rect.left() + 6.0 + off, rect.center().y + off), egui::Align2::LEFT_CENTER)
            };
            let text = p.text(pos, align, label, font, theme::color::text());
            if sort.column == column {
                // A small sort triangle beside the caption.
                let x = if right { text.left() - 10.0 } else { text.right() + 6.0 };
                let c = egui::pos2(x + 3.0, rect.center().y + off);
                let pts = if sort.ascending {
                    vec![egui::pos2(c.x - 3.0, c.y + 2.0), egui::pos2(c.x + 3.0, c.y + 2.0), egui::pos2(c.x, c.y - 2.0)]
                } else {
                    vec![egui::pos2(c.x - 3.0, c.y - 2.0), egui::pos2(c.x + 3.0, c.y - 2.0), egui::pos2(c.x, c.y + 2.0)]
                };
                p.add(egui::Shape::convex_polygon(pts, theme::color::shadow(), egui::Stroke::NONE));
            }
            if resp.clicked() {
                actions.push(Action::Sort(column));
            }
            egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| header_menu(ui, columns, actions));
        };

        // Where the rows are: row 0's top on screen, the icon's left edge and
        // the Name column's right edge. Taken from a drawn row, for the marquee.
        let mut geometry: Option<(f32, f32, f32)> = None;
        let h_out = egui::ScrollArea::horizontal().id_salt("details-h").auto_shrink(false).scroll_bar_visibility(hidden).show(ui, |ui| {
            // Keyboard moves jump, as the list view does, instead of easing there.
            let mut table = TableBuilder::new(ui).animate_scrolling(false);
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
                    ui.painter().rect_filled(rect, 0.0, theme::color::face());
                    theme::bevel(ui.painter(), rect, Bevel::Raised);
                    let resp = ui.interact(rect, ui.id().with("header-rest"), Sense::click());
                    egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| header_menu(ui, columns, actions));
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
                let clip = body.ui_mut().clip_rect();
                if marquee.is_some() {
                    edge_scroll(body.ui_mut(), clip, LIST_ROW_HEIGHT, 1);
                }
                // Once, from the first row's Name cell, which knows where the rows start.
                let mut scroll_once = drag.is_some();
                body.rows(LIST_ROW_HEIGHT, visible.len(), |mut row| {
                    let pos = row.index();
                    let index = visible[pos];
                    let entry = &entries[index];
                    // The drop target is drawn selected.
                    let marked = drag.as_ref().is_some_and(|d| d.marks(&Mark::Item(entry.path.clone())));
                    let selected = selection.contains(&entry.path) || marked;
                    let focused = focused || marked;
                    // The icon and label, the only part of the row that hits the
                    // item: elsewhere is empty space (`CListView::HandleMouse`).
                    let mut item_rect = egui::Rect::NOTHING;
                    let mut label_rect = egui::Rect::NOTHING;
                    let mut image_rect = egui::Rect::NOTHING;
                    let mut name_column = (0.0, 0.0);

                    for &column in &shown {
                        row.col(|ui| {
                            if column != Column::Name {
                                let text = cell_text(entry, column);
                                if column == Column::Branch && entry.branch.is_some() {
                                    ui.add_space(4.0);
                                    let (icon, _) = ui.allocate_exact_size(egui::vec2(GIT_ICON_WIDTH, 16.0), Sense::hover());
                                    icons::paint_git_branch(ui.painter(), icon.center(), GIT_ICON_SCALE, theme::color::active_caption());
                                    ui.add_space(2.0);
                                    ui.label(text);
                                    return;
                                }
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
                            if std::mem::take(&mut scroll_once) {
                                let top = ui.max_rect().top() - pos as f32 * pitch;
                                let content = egui::Rect::from_x_y_ranges(clip.x_range(), top..=top + visible.len() as f32 * pitch);
                                item_drag::edge_scroll(ui, clip, content, LIST_ROW_HEIGHT, 1);
                            }
                            ui.add_space(4.0);
                            let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
                            icons.paint(ui, icon_rect, entry, IconState::of(selected, focused, entry.hidden || cut.contains(&entry.path)));
                            ui.add_space(3.0);
                            let label = match rename.as_mut().filter(|r| r.path == entry.path) {
                                Some(r) => {
                                    rename_editor(ui, r, false, actions);
                                    ui.min_rect()
                                }
                                None => {
                                    let name = entry.display_name(settings.show_extensions);
                                    let label = selectable_name(ui, name, name_color(entry, settings, selected && focused), selected, focused);
                                    if cue && *focus_item == Some(index) {
                                        paint_focus(ui, label, selected);
                                    }
                                    label
                                }
                            };
                            item_rect = egui::Rect::from_x_y_ranges(icon_rect.left()..=label.right(), ui.max_rect().y_range());
                            label_rect = label;
                            image_rect = icon_rect;
                            name_column = (icon_rect.left(), ui.max_rect().right());
                        });
                    }
                    row.col(|_| {});

                    let resp = row.response();
                    let top = resp.rect.top() - pos as f32 * pitch;
                    geometry.get_or_insert((top, name_column.0, name_column.1));
                    drag_frame.item(clip, item_rect, entry);
                    let name = entry.display_name(settings.show_extensions);
                    drag_frame.drawn(clip, &entry.path, image_rect, label_rect, || resp.ctx.fonts_mut(|f| f.layout_no_wrap(name.to_owned(), font.clone(), theme::color::selection_text())), false);
                    if resp.drag_started() && press.is_some_and(|p| item_rect.contains(p)) {
                        actions.push(Action::DragItems(index));
                    }
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
                    egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| match ui.data(|d| d.get_temp::<Option<usize>>(menu_id)).flatten() {
                        Some(i) => item_menu(ui, &entries[i], can_paste, open_with, icons, actions),
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
                            let w = ui.fonts_mut(|f| f.layout_no_wrap(name.to_owned(), font.clone(), theme::color::text())).size().x;
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
        let width = |text: String| ui.fonts_mut(|f| f.layout_no_wrap(text, font.clone(), theme::color::text()).size().x);
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
                        // The branch glyph and its padding come first.
                        t if column == Column::Branch => width(t) + 12.0 + GIT_ICON_WIDTH + 2.0,
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
        let focused = self.focus == Pane::List && window_focused(ui) && !text_field_focused(ui);
        let cue = focused && self.show_focus;
        let can_paste = self.can_paste();
        let folder = self.current().to_path_buf();
        let cut = cut_paths(&self.clipboard);
        let reveal = self.reveal.take().and_then(|r| visible.iter().position(|&v| v == r));
        let font = egui::TextStyle::Body.resolve(ui.style());
        let line_height = ui.fonts_mut(|f| f.row_height(&font));
        let menu_id = ui.id().with("icons-menu");
        let Self { entries, selection, rename, settings, search, icons, open_with, listing, focus_item, marquee, anchor, drag, drag_frame, .. } = self;

        let out = egui::ScrollArea::vertical()
            .id_salt("icons")
            .auto_shrink(false)
            .animated(false)
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
                if drag.is_some() {
                    let content = egui::Rect::from_min_size(origin, egui::vec2(viewport.width(), grid.content_height()));
                    item_drag::edge_scroll(ui, shown, content, LIST_ROW_HEIGHT, 1);
                }
                let resp = ui.interact(shown, ui.id().with("icons-hit"), Sense::click_and_drag());
                let content = |p: egui::Pos2| (p.x - origin.x, p.y - origin.y);
                let pointer = resp.interact_pointer_pos().or(resp.hover_pos()).map(content);
                let press = resp.drag_started().then(|| ui.input(|i| i.pointer.press_origin())).flatten().map(content);

                let mut hit = None;
                let mut on_label = false;
                let mut press_hit = None;
                // The focused item's whole label is drawn last, over its neighbors.
                let mut on_top = None;
                for pos in grid.items_in(view.min.y, view.max.y) {
                    let index = visible[pos];
                    let entry = &entries[index];
                    let marked = drag.as_ref().is_some_and(|d| d.marks(&Mark::Item(entry.path.clone())));
                    let selected = selection.contains(&entry.path) || marked;
                    let focused = focused || marked;
                    let unfolded = focused && *focus_item == Some(index);
                    let color = name_color(entry, settings, selected && focused);
                    let galley = icon_label(ui, entry.display_name(settings.show_extensions), &font, color, unfolded);
                    let size = layout::label_size((galley.size().x, galley.size().y), line_height, !unfolded);
                    let cell = IconCell::new(grid.slot(pos), size);
                    icons.paint_large(ui, screen(cell.image), entry, IconState::of(selected, focused, entry.hidden || cut.contains(&entry.path)));
                    if pointer.is_some_and(|p| cell.hit(p)) {
                        hit = Some(pos);
                        on_label = pointer.is_some_and(|p| cell.label.contains(p));
                    }
                    if press.is_some_and(|p| cell.hit(p)) {
                        press_hit = Some(index);
                    }
                    // Both parts take drops; the gap between them doesn't.
                    drag_frame.item(shown, screen(cell.image), entry);
                    drag_frame.item(shown, screen(cell.label), entry);
                    drag_frame.drawn(shown, &entry.path, screen(cell.image), screen(cell.label), || icon_label(ui, entry.display_name(settings.show_extensions), &font, theme::color::selection_text(), unfolded), false);
                    let renaming = rename.as_ref().is_some_and(|r| r.path == entry.path);
                    if unfolded {
                        on_top = Some((screen(cell.label), galley, selected, renaming));
                    } else if !renaming {
                        paint_label(ui, screen(cell.label), galley, selected, focused);
                    }
                }
                if let Some((label, galley, selected, false)) = on_top {
                    paint_label(ui, label, galley, selected, focused);
                    if cue {
                        paint_focus(ui, label, selected);
                    }
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
                match (press, press_hit) {
                    (_, Some(index)) => actions.push(Action::DragItems(index)),
                    (Some(origin), None) => {
                        let m = ui.input(|i| i.modifiers);
                        actions.push(Action::MarqueeBegin { origin, extend: m.command || m.shift });
                    }
                    (None, None) => {}
                }
                if let Some(m) = marquee.as_mut() {
                    // Select bounds: the image and the label (`LVIR_SELECTBOUNDS`),
                    // laid out as drawn above.
                    let candidates = |r: &layout::Rect| {
                        grid.items_in(r.min.1, r.max.1)
                            .map(|pos| {
                                let index = visible[pos];
                                let unfolded = focused && *focus_item == Some(index);
                                let galley = icon_label(ui, entries[index].display_name(settings.show_extensions), &font, theme::color::text(), unfolded);
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
                egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| match ui.data(|d| d.get_temp::<Option<usize>>(menu_id)).flatten() {
                    Some(i) => item_menu(ui, &entries[i], can_paste, open_with, icons, actions),
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
                        ui.fonts_mut(|f| f.layout_no_wrap(name, font.clone(), theme::color::text()).size().x)
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
        let focused = self.focus == Pane::List && window_focused(ui) && !text_field_focused(ui);
        let cue = focused && self.show_focus;
        let can_paste = self.can_paste();
        let folder = self.current().to_path_buf();
        let cut = cut_paths(&self.clipboard);
        let reveal = self.reveal.take().and_then(|r| visible.iter().position(|&v| v == r));
        let menu_id = ui.id().with("list-menu");
        let Self { entries, selection, rename, settings, search, icons, open_with, listing, marquee, anchor, focus_item, drag, drag_frame, .. } = self;
        // An item's icon rect and label box (text plus `SM_CXEDGE` each side,
        // within the column), in content coordinates. Unverified: that the
        // label box hugs the text as in Large Icons (`CLVListView::v_ItemHitTest`
        // wasn't traced).
        let item_rects = |ui: &Ui, pos: usize| {
            let (x, y) = columns.cell(pos);
            let icon = layout::Rect::from_min_size((x, y + (row_height - layout::SMALL_ICON) / 2.0), (layout::SMALL_ICON, layout::SMALL_ICON));
            let name = entries[visible[pos]].display_name(settings.show_extensions).to_owned();
            let text = ui.fonts_mut(|f| f.layout_no_wrap(name, font.clone(), theme::color::text()).size().x);
            let width = (text + 2.0 * layout::LABEL_PAD_X).min(columns.column_width - layout::SMALL_ICON);
            (icon, layout::Rect::from_min_size((x + layout::SMALL_ICON, y), (width, row_height)))
        };

        let out = egui::ScrollArea::horizontal()
            .id_salt("list")
            .auto_shrink(false)
            .animated(false)
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
                if drag.is_some() {
                    let content = egui::Rect::from_min_size(origin, egui::vec2(columns.content_width(), viewport.height()));
                    item_drag::edge_scroll(ui, shown, content, layout::SMALL_ICON, 0);
                }
                let resp = ui.interact(shown, ui.id().with("list-hit"), Sense::click_and_drag());
                let content = |p: egui::Pos2| (p.x - origin.x, p.y - origin.y);
                let pointer = resp.interact_pointer_pos().or(resp.hover_pos()).map(content);
                let press = resp.drag_started().then(|| ui.input(|i| i.pointer.press_origin())).flatten().map(content);

                let mut hit = None;
                let mut on_label = false;
                let mut press_hit = None;
                for pos in columns.items_in(view.min.x, view.max.x) {
                    let index = visible[pos];
                    let entry = &entries[index];
                    let marked = drag.as_ref().is_some_and(|d| d.marks(&Mark::Item(entry.path.clone())));
                    let selected = selection.contains(&entry.path) || marked;
                    let focused = focused || marked;
                    let (icon, label) = item_rects(ui, pos);
                    icons.paint(ui, screen(icon), entry, IconState::of(selected, focused, entry.hidden || cut.contains(&entry.path)));
                    let hits = |p: (f32, f32)| icon.contains(p) || label.contains(p);
                    if pointer.is_some_and(hits) {
                        hit = Some(pos);
                        on_label = pointer.is_some_and(|p| label.contains(p));
                    }
                    if press.is_some_and(hits) {
                        press_hit = Some(index);
                    }
                    drag_frame.item(shown, screen(icon), entry);
                    drag_frame.item(shown, screen(label), entry);
                    let name = entry.display_name(settings.show_extensions);
                    drag_frame.drawn(shown, &entry.path, screen(icon), screen(label), || ui.fonts_mut(|f| f.layout_no_wrap(name.to_owned(), font.clone(), theme::color::selection_text())), false);
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
                    if cue && *focus_item == Some(index) {
                        paint_focus(ui, screen(label), selected);
                    }
                }
                match (press, press_hit) {
                    (_, Some(index)) => actions.push(Action::DragItems(index)),
                    (Some(origin), None) => {
                        let m = ui.input(|i| i.modifiers);
                        actions.push(Action::MarqueeBegin { origin, extend: m.command || m.shift });
                    }
                    (None, None) => {}
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
                egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| match ui.data(|d| d.get_temp::<Option<usize>>(menu_id)).flatten() {
                    Some(i) => item_menu(ui, &entries[i], can_paste, open_with, icons, actions),
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
        let font = egui::TextStyle::Body.resolve(ui.style());
        // The Git summary gets a pane of its own, sized to its text, while the
        // open folder is in a working tree.
        let git = self.git.as_ref().map(|g| {
            let segments = g.segments();
            let text_w: f32 = ui.fonts_mut(|f| {
                segments.iter().map(|s| f.layout_no_wrap(s.text.clone(), font.clone(), theme::color::text()).size().x).sum()
            });
            // Icon, then the text, with 4 px on each side.
            let width = (4.0 + GIT_ICON_WIDTH + 4.0 + text_w + 6.0).min(rect.width() * 0.5);
            (segments, width)
        });
        let grip = !theme::is_maximized(ui.ctx());
        let right = egui::Rect::from_min_max(egui::pos2(rect.right() - place_w, rect.top()), rect.max);
        let mid = egui::Rect::from_min_max(egui::pos2(right.left() - 2.0 - size_w, rect.top()), egui::pos2(right.left() - 2.0, rect.bottom()));
        let git_pane = git.as_ref().map(|(_, w)| egui::Rect::from_min_max(egui::pos2(mid.left() - 2.0 - w, rect.top()), egui::pos2(mid.left() - 2.0, rect.bottom())));
        let left = egui::Rect::from_min_max(rect.min, egui::pos2(git_pane.map_or(mid.left(), |r| r.left()) - 2.0, rect.bottom()));
        let p = ui.painter();
        for r in [left, mid, right].into_iter().chain(git_pane) {
            theme::bevel(p, r, Bevel::SunkenThin);
        }
        // The grip sits inside the last pane's border, in its bottom right corner.
        if grip {
            let inner = right.shrink(2.0);
            theme::size_grip(ui, egui::Rect::from_min_max(egui::pos2(inner.right() - theme::GRIP_WIDTH, inner.top()), inner.max));
        }
        let p = ui.painter();
        if let (Some((segments, _)), Some(pane)) = (git, git_pane) {
            let clip = p.with_clip_rect(pane.shrink(2.0));
            let icon = egui::pos2(pane.left() + 4.0 + GIT_ICON_WIDTH / 2.0, pane.center().y);
            icons::paint_git_branch(&clip, icon, GIT_ICON_SCALE, theme::color::active_caption());
            let mut x = pane.left() + 4.0 + GIT_ICON_WIDTH + 4.0;
            for s in segments {
                let rect = clip.text(egui::pos2(x, pane.center().y), egui::Align2::LEFT_CENTER, s.text, font.clone(), theme::color::git(s.tone));
                x = rect.right();
            }
        }
        let color = if self.status.is_some() { theme::color::error_text() } else { theme::color::text() };
        p.with_clip_rect(left.shrink(2.0)).text(egui::pos2(left.left() + 4.0, left.center().y), egui::Align2::LEFT_CENTER, text, font.clone(), color);
        p.text(egui::pos2(mid.right() - 4.0, mid.center().y), egui::Align2::RIGHT_CENTER, size, font.clone(), theme::color::text());
        let icon = egui::Rect::from_min_size(egui::pos2(right.left() + 4.0, right.center().y - 8.0), egui::vec2(16.0, 16.0));
        theme::paint_icon(p, icon, Icon::Drive, true);
        let name_clip = if grip { right.shrink(2.0).with_max_x(right.right() - 2.0 - theme::GRIP_WIDTH) } else { right.shrink(2.0) };
        p.with_clip_rect(name_clip).text(
            egui::pos2(icon.right() + 4.0, right.center().y),
            egui::Align2::LEFT_CENTER,
            &self.computer_name,
            font,
            theme::color::text(),
        );
    }
}

impl eframe::App for Explorer {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.poll_loads();
        self.poll_lists();
        self.poll_git();
        self.list_branches_if_missing();
        self.poll_ejects();
        self.poll_ops();
        self.poll_watcher();
        self.open_due_tree_selection();
        self.refresh_clipboard();
        self.icons.begin_frame();
        let visible = self.visible();
        self.slow_click_rename(ui);
        let mut actions = Vec::new();
        self.keyboard(ui, &visible, &mut actions);
        self.drag_frame.begin(ui, self.drag.is_some());

        // The window frame: a raised 3D border around everything.
        let outer = ui.max_rect();
        ui.painter().rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(ui.painter(), outer, Bevel::Raised);
        theme::resize_frame(ui, outer);
        let mut root = ui.new_child(egui::UiBuilder::new().max_rect(outer.shrink(4.0)).layout(Layout::top_down(Align::Min)));
        let ui = &mut root;
        let plain = egui::Frame::NONE.fill(theme::color::face());

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

        self.drag_step(ui, &mut actions);
        if !cfg!(target_os = "macos") {
            let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
            if !dropped.is_empty() {
                self.drop_files(dropped);
            }
        }
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
        self.show_open_with();
        match self.about_dialog.as_mut().map(|d| d.show(&ctx)) {
            Some(AboutOutcome::Close) => self.about_dialog = None,
            Some(AboutOutcome::Reveal(path)) => {
                self.about_dialog = None;
                self.reveal_in_view(path);
            }
            Some(AboutOutcome::Open) | None => {}
        }
        self.show_conflict();
        self.show_transfers();
        self.save_columns();
        self.save_window_size();
    }
}

#[cfg(unix)]
const TERMINAL_LABEL: &str = "Open in Terminal";

/// A menu bar entry. While one bar menu is open, resting the pointer on
/// another entry opens that one instead and closes the first, as in Windows
/// and macOS. egui's `menu_button` only opens on a click.
fn bar_menu_button<R>(ui: &mut Ui, text: &str, content: impl FnOnce(&mut Ui) -> R) {
    use egui::Popup;
    let ctx = ui.ctx().clone();
    let popup = ui.next_auto_id().with("popup");
    let response = ui.menu_button(text, content).response;
    let siblings_key = egui::Id::new("bar_menu_popups");
    let mut ids: Vec<egui::Id> = ctx.data(|d| d.get_temp(siblings_key)).unwrap_or_default();
    if !ids.contains(&popup) {
        ids.push(popup);
        ctx.data_mut(|d| d.insert_temp(siblings_key, ids.clone()));
    }
    let other_open = ids.iter().any(|&id| id != popup && Popup::is_id_open(&ctx, id));
    if response.hovered() && other_open {
        Popup::open_id(&ctx, popup);
        ctx.request_repaint();
    }
}

/// A submenu item that opens when the pointer has rested on it for
/// [`menu_delay::SHOW_DELAY_MS`], as Windows' menus do (`MenuShowDelay`); a
/// click opens it at once. egui's own `menu_button` opens on first contact.
/// `gutter` indents the label like a marked item. Otherwise this is egui's
/// `SubMenuButton`.
fn delayed_menu_button<R>(ui: &mut Ui, text: &str, gutter: bool, content: impl FnOnce(&mut Ui) -> R) {
    use egui::containers::menu::{MenuState, SubMenu, SubMenuButton};
    let id = ui.next_auto_id();
    let open = MenuState::from_ui(ui, |state, _| state.open_item == Some(SubMenu::id_from_widget_id(id)));
    let inactive = ui.style().visuals.widgets.inactive;
    if open {
        ui.style_mut().visuals.widgets.inactive = ui.style().visuals.widgets.open;
    }
    let response = if gutter {
        egui_classic_menu::submenu_row(ui, text, open)
    } else {
        ui.add(egui::Button::new(text).right_text(SubMenuButton::RIGHT_ARROW))
    };
    ui.style_mut().visuals.widgets.inactive = inactive;

    // Remember when the rest began and in which pass it was last seen: a gap
    // means the menu was closed in between, so the wait starts over.
    let key = response.id.with("hover_since");
    let (now, pass) = (ui.input(|i| i.time), ui.ctx().cumulative_pass_nr());
    let mut due = false;
    if response.hovered() {
        let (since, _) = ui.data(|d| d.get_temp::<(f64, u64)>(key)).filter(|(_, seen)| seen + 1 >= pass).unwrap_or((now, pass));
        ui.data_mut(|d| d.insert_temp(key, (since, pass)));
        match menu_delay::wait_left(now - since) {
            Some(left) => ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left)),
            None => due = true,
        }
    } else {
        ui.data_mut(|d| d.remove_temp::<(f64, u64)>(key));
    }
    if open || due || response.clicked() {
        SubMenu::new().show(ui, &response, content);
    }
}

/// The "Open With" submenu: the file's apps, then "Choose Program...".
/// The app list is looked up once per file and kept while the menu is open.
fn open_with_menu(ui: &mut Ui, path: &Path, cache: &mut Option<(PathBuf, Vec<PathBuf>)>, icons: &mut FileIcons, actions: &mut Vec<Action>) {
    delayed_menu_button(ui, "Open With", false, |ui| {
        if cache.as_ref().is_none_or(|(p, _)| p != path) {
            *cache = Some((path.to_path_buf(), open_with::apps_for(path)));
        }
        let apps = cache.as_ref().map_or(&[][..], |(_, apps)| apps.as_slice());
        for app in apps {
            let name = open_with::app_name(app);
            let button = match icons.menu_icon(ui.ctx(), app) {
                Some(icon) => egui::Button::image_and_text(egui::Image::new((icon, egui::vec2(16.0, 16.0))), name),
                None => egui::Button::new(name),
            };
            if ui.add(button).clicked() {
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
fn item_menu(ui: &mut Ui, entry: &Entry, can_paste: bool, open_with: &mut Option<(PathBuf, Vec<PathBuf>)>, icons: &mut FileIcons, actions: &mut Vec<Action>) {
    if ui.button(RichText::new("Open").font(theme::bold(theme::FONT_SIZE))).clicked() {
        actions.push(Action::Open(entry.path.clone()));
    }
    // After Finder's -[TBaseBrowserViewController cmdShowPackageContents:]:
    // the package opens as a folder in the same window.
    if entry.kind == RenameKind::Package && ui.button("Show Package Contents").clicked() {
        actions.push(Action::Navigate(entry.path.clone()));
    }
    if open_with::SUPPORTED && entry.kind == RenameKind::File {
        open_with_menu(ui, &entry.path, open_with, icons, actions);
    }
    if ui.button(platform::REVEAL_LABEL).clicked() {
        actions.push(Action::RevealInFinder(entry.path.clone()));
    }
    #[cfg(unix)]
    if entry.kind == RenameKind::Folder && ui.button(TERMINAL_LABEL).clicked() {
        actions.push(Action::OpenTerminal(entry.path.clone()));
    }
    #[cfg(unix)]
    if is_ejectable(&entry.path) && ui.button("Eject").clicked() {
        actions.push(Action::Eject(entry.path.clone()));
    }
    ui.separator();
    if ui.add(menu_item("Cut", platform::shortcut(false, "X"))).clicked() {
        actions.push(Action::Cut);
    }
    if ui.add(menu_item("Copy", platform::shortcut(false, "C"))).clicked() {
        actions.push(Action::Copy);
    }
    if ui.add(menu_item("Copy as Path", platform::shortcut(true, "C"))).clicked() {
        actions.push(Action::CopyAsPath);
    }
    if entry.kind == RenameKind::Folder && ui.add_enabled(can_paste, menu_item("Paste", platform::shortcut(false, "V"))).clicked() {
        actions.push(Action::Paste(Some(entry.path.clone())));
    }
    if entry.kind == RenameKind::Folder && ui.add_enabled(can_paste, menu_item("Move Here", platform::shortcut(true, "V"))).clicked() {
        actions.push(Action::MoveHere(Some(entry.path.clone())));
    }
    ui.separator();
    if ui.add(menu_item("Delete", platform::shortcut(false, if cfg!(target_os = "macos") { "⌫" } else { "Backspace" }))).clicked() {
        actions.push(Action::Delete);
    }
    if ui.add(menu_item("Rename", "F2".to_owned())).clicked() {
        actions.push(Action::BeginRename);
    }
    ui.separator();
    if ui.add(menu_item("Properties", platform::shortcut(false, "I"))).clicked() {
        actions.push(Action::ItemProperties(entry.path.clone()));
    }
}

/// The context menu for empty space in the item view.
fn background_menu(ui: &mut Ui, folder: &Path, can_paste: bool, actions: &mut Vec<Action>) {
    if ui.add_enabled(can_paste, menu_item("Paste", platform::shortcut(false, "V"))).clicked() {
        actions.push(Action::Paste(None));
    }
    if ui.add_enabled(can_paste, menu_item("Move Here", platform::shortcut(true, "V"))).clicked() {
        actions.push(Action::MoveHere(None));
    }
    ui.separator();
    if ui.add(menu_item("Refresh", platform::shortcut(false, "R"))).clicked() {
        actions.push(Action::Refresh);
    }
    #[cfg(unix)]
    if ui.button(TERMINAL_LABEL).clicked() {
        actions.push(Action::OpenTerminal(folder.to_path_buf()));
    }
    ui.separator();
    delayed_menu_button(ui, "New", false, |ui| {
        if ui.button("Folder").clicked() {
            actions.push(Action::NewFolder);
        }
    });
    ui.separator();
    if ui.add(menu_item("Properties", platform::shortcut(false, "I"))).clicked() {
        actions.push(Action::Properties(folder.to_path_buf()));
    }
}

fn empty_message(ui: &mut Ui, search: &str) {
    let msg = if search.is_empty() { "This folder is empty." } else { "No items match your search." };
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(msg).color(theme::color::gray_text()));
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
        return theme::color::selection_text();
    }
    match classify(entry).filter(|_| settings.color_names) {
        Some(Highlight::Executable) => theme::color::executable(),
        Some(Highlight::Link) => theme::color::link(),
        Some(Highlight::Archive | Highlight::BrokenLink) => theme::color::archive(),
        None => theme::color::text(),
    }
}

/// The dotted focus rectangle on an item label's box.
fn paint_focus(ui: &Ui, rect: egui::Rect, selected: bool) {
    theme::focus_rect_over(ui.painter(), rect, if selected { theme::color::selection() } else { theme::color::window() });
}

/// Whether the address or search field has the keyboard, which takes the
/// focus from the panes (their selection turns gray, as in Windows).
fn text_field_focused(ui: &Ui) -> bool {
    ui.ctx().memory(|m| m.focused()).is_some_and(|id| id == address_id() || id == search_id())
}

/// Paints an item label in its box: the text alone, or on the selection
/// color (button face while the view isn't focused).
fn paint_label(ui: &Ui, rect: egui::Rect, galley: std::sync::Arc<egui::Galley>, selected: bool, focused: bool) {
    if selected {
        ui.painter().rect_filled(rect, 0.0, if focused { theme::color::selection() } else { theme::color::face() });
    }
    // Centered galleys are laid out around x = 0; left-aligned ones start there.
    let x = if galley.job.halign == Align::Center { rect.center().x } else { rect.left() + layout::LABEL_PAD_X };
    let y = rect.top() + (rect.height() - galley.size().y) / 2.0;
    ui.painter().with_clip_rect(rect).galley(egui::pos2(x, y), galley, theme::color::text());
}

/// The in-place rename box: white with a thin black frame, like a classic
/// list view label edit. Initial selection comes from the Explorer rule.
/// `wrapping` makes it the Large Icons box: centered text wrapped in the
/// label width, Enter still commits.
fn rename_editor(ui: &mut Ui, r: &mut RenameState, wrapping: bool, actions: &mut Vec<Action>) {
    let id = ui.make_persistent_id(("rename", &r.path));
    let frame = egui::Frame::NONE
        .fill(theme::color::window())
        .stroke(egui::Stroke::new(1.0, theme::color::text()))
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
        Column::Branch => entry.branch.clone().unwrap_or_default(),
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
        let fill = if focused { theme::color::selection() } else { theme::color::face() };
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 0.0, fill));
    }
    rect
}

/// A flat crumb in the address field that pops out when hovered.
fn crumb_width(ui: &Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().layout_no_wrap(text.to_owned(), font, theme::color::text()).size().x + 6.0
}

fn crumb_button(ui: &mut Ui, text: &str) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, theme::color::text());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 6.0, 16.0), Sense::click());
    if resp.is_pointer_button_down_on() {
        theme::bevel(ui.painter(), rect, Bevel::SunkenThin);
    } else if resp.hovered() {
        theme::bevel(ui.painter(), rect, Bevel::RaisedThin);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, theme::color::text());
    resp
}

/// egui's input clock in milliseconds.
fn now_ms(ctx: &egui::Context) -> u64 {
    ctx.input(|i| (i.time * 1000.0) as u64)
}

/// The address bar's text field, which keys can give the focus to.
fn address_id() -> egui::Id {
    egui::Id::new("address-edit")
}

/// The search field.
fn search_id() -> egui::Id {
    egui::Id::new("search-field")
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
/// only its text. `drag`: the item drag going on, if any, and the frame's
/// drop zones.
#[allow(clippy::too_many_arguments)]
fn tree_row(
    ui: &mut Ui,
    icons: &mut FileIcons,
    row: &Row,
    guide: &Guide,
    scroll: Option<Scroll>,
    focused: bool,
    cue: bool,
    (drag, frame): (Option<&ItemDrag>, &mut DragFrame),
    actions: &mut Vec<Action>,
) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_w = ui.painter().layout_no_wrap(row.label.clone(), font.clone(), theme::color::text()).size().x;
    let x0 = ui.max_rect().left() + 2.0;
    let label_x = x0 + (row.depth + 1) as f32 * TREE_INDENT + 16.0 + 4.0;
    let width = (label_x - ui.max_rect().left() + text_w + 6.0).max(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, TREE_ROW_HEIGHT), Sense::click_and_drag());
    let p = ui.painter();
    let line = egui::Stroke::new(1.0, theme::color::tree_line());
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
        p.rect_filled(boxr, 0.0, theme::color::window());
        p.rect_stroke(boxr, 0.0, line, egui::StrokeKind::Inside);
        let s = egui::Stroke::new(1.0, theme::color::text());
        p.hline(boxr.left() + 2.0..=boxr.right() - 2.0, mid_y, s);
        if !row.expanded {
            p.vline(cx, boxr.top() + 2.0..=boxr.bottom() - 2.0, s);
        }
    }

    let icon_rect = egui::Rect::from_min_size(egui::pos2(x0 + (row.depth + 1) as f32 * TREE_INDENT, rect.center().y - 8.0), egui::vec2(16.0, 16.0));
    if row.placeholder {
        p.text(egui::pos2(icon_rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, &row.label, font, theme::color::gray_text());
        return;
    }
    if row.depth == 0 && row.section == Section::Locations {
        theme::paint_icon(p, icon_rect, Icon::Drive, true);
    } else {
        icons.paint_tree_folder(ui, icon_rect, &row.key.path);
    }
    let text_rect = egui::Rect::from_min_size(egui::pos2(label_x - 2.0, rect.center().y - 7.0), egui::vec2(text_w + 4.0, 14.0));
    // The whole row takes drops (`TVHT_ONITEMROW`, unverified for drags);
    // the drop target is drawn selected and focused.
    let mark = Mark::Tree(row.key.clone());
    let marked = drag.is_some_and(|d| d.marks(&mark));
    let expand = (row.expandable && !row.expanded).then(|| row.key.clone());
    frame.zone(ui.clip_rect(), rect, Some(row.key.path.clone()), Some(mark), expand);
    frame.drawn(ui.clip_rect(), &row.key.path, icon_rect, text_rect, || p.layout_no_wrap(row.label.clone(), font.clone(), theme::color::selection_text()), true);
    if resp.drag_started() && !boxr.expand(3.0).contains(ui.input(|i| i.pointer.press_origin()).unwrap_or_default()) {
        actions.push(Action::DragFolder(row.key.path.clone()));
    }
    let focused = focused || marked;
    // While a row is the drop target, the selected one isn't highlighted
    // (as remembered from the tree view; unverified).
    let highlighted = marked || row.selected && !drag.is_some_and(ItemDrag::over_tree);
    let text_color = if highlighted {
        let fill = if focused { theme::color::selection() } else { theme::color::face() };
        p.rect_filled(text_rect, 0.0, fill);
        if focused { theme::color::selection_text() } else { theme::color::text() }
    } else {
        theme::color::text()
    };
    p.text(egui::pos2(label_x, rect.center().y), egui::Align2::LEFT_CENTER, &row.label, font, text_color);
    // The tree's focus is its selected row.
    if row.selected && cue {
        theme::focus_rect_over(p, text_rect, theme::color::selection());
    }
    // Keyboard moves scroll only as far as needed (`TVM_ENSUREVISIBLE`).
    match scroll {
        Some(Scroll::Center) if row.selected => ui.scroll_to_rect(rect, Some(Align::Center)),
        Some(Scroll::Nearest) if row.selected => ui.scroll_to_rect(rect, None),
        _ => {}
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
    egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| {
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
        #[cfg(unix)]
        if ui.button(TERMINAL_LABEL).clicked() {
            actions.push(Action::OpenTerminal(row.key.path.clone()));
        }
        #[cfg(unix)]
        if is_ejectable(&row.key.path) && ui.button("Eject").clicked() {
            actions.push(Action::Eject(row.key.path.clone()));
        }
        ui.separator();
        if ui.button("Properties").clicked() {
            actions.push(Action::Properties(row.key.path.clone()));
        }
    });
}

/// The keys the Folders pane handles, see [`tree_key_action`].
const TREE_KEYS: [Key; 10] =
    [Key::ArrowDown, Key::ArrowUp, Key::ArrowLeft, Key::ArrowRight, Key::Home, Key::End, Key::PageUp, Key::PageDown, Key::Plus, Key::Minus];

/// Keys in the navigation pane, after the tree view's `TV_KeyDown`
/// (comctl32): arrows walk the shown rows, Right expands or steps into a
/// folder, Left collapses or steps out, Home and End go to the first and
/// last row, Page Up and Down move by one row less than `page` (the rows
/// that fit), + and - expand and collapse (the keypad keys in Windows).
fn tree_key_action(rows: &[Row], key: Key, page: usize) -> Option<Action> {
    let real: Vec<usize> = (0..rows.len()).filter(|&i| !rows[i].placeholder).collect();
    let cur = real.iter().position(|&i| rows[i].selected);
    let select = |pos: usize| real.get(pos).map(|&i| Action::TreeKeySelect(rows[i].key.clone()));
    let Some(pos) = cur else {
        let moves = matches!(key, Key::ArrowDown | Key::ArrowUp | Key::Home | Key::End | Key::PageUp | Key::PageDown);
        return moves.then(|| select(0)).flatten();
    };
    let row = &rows[real[pos]];
    let step = page.saturating_sub(1).max(1);
    match key {
        Key::ArrowDown if pos + 1 < real.len() => select(pos + 1),
        Key::ArrowUp if pos > 0 => select(pos - 1),
        Key::Home if pos > 0 => select(0),
        Key::End if pos + 1 < real.len() => select(real.len() - 1),
        Key::PageUp if pos > 0 => select(pos.saturating_sub(step)),
        Key::PageDown if pos + 1 < real.len() => select((pos + step).min(real.len() - 1)),
        Key::Plus if row.expandable && !row.expanded => Some(Action::TreeExpand(row.key.clone())),
        Key::Minus if row.expanded => Some(Action::TreeCollapse(row.key.clone())),
        Key::ArrowRight if row.expandable && !row.expanded => Some(Action::TreeExpand(row.key.clone())),
        Key::ArrowRight => real
            .get(pos + 1)
            .filter(|&&i| rows[i].depth > row.depth)
            .map(|&i| Action::TreeKeySelect(rows[i].key.clone())),
        Key::ArrowLeft if row.expanded => Some(Action::TreeCollapse(row.key.clone())),
        Key::ArrowLeft => real[..pos]
            .iter()
            .rev()
            .find(|&&i| rows[i].depth + 1 == row.depth)
            .map(|&i| Action::TreeKeySelect(rows[i].key.clone())),
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
    let text_width = items.iter().map(|(name, _)| ui.painter().layout_no_wrap(name.clone(), font.clone(), theme::color::text()).size().x).fold(0.0, f32::max);
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
/// A menu entry with its keyboard shortcut at the right edge in the weak
/// (muted) text color, like macOS menus.
fn menu_item(label: &str, keys: String) -> egui::Button<'_> {
    egui::Button::new(label).shortcut_text(RichText::new(keys).family(theme::shortcut_family()))
}

/// [`menu_item`] between the check mark and arrow gutters, so the labels and
/// shortcuts of a menu bar dropdown line up the way Windows' do. No shortcut
/// when `keys` is empty.
fn gutter_item(ui: &Ui, label: &str, keys: String) -> egui::Button<'static> {
    let shortcut = (!keys.is_empty()).then(|| RichText::new(keys).family(theme::shortcut_family()));
    egui_classic_menu::item(ui, label, shortcut)
}

fn view_choices(ui: &mut Ui, current: ViewMode, actions: &mut Vec<Action>) {
    for (mode, label) in [(ViewMode::LargeIcons, "Large Icons"), (ViewMode::List, "List"), (ViewMode::Details, "Details")] {
        if egui_classic_menu::radio(ui, current == mode, label).clicked() {
            actions.push(Action::SetView(mode));
            ui.close();
        }
    }
}

/// The typed or pasted address without the quotes a path copied from a shell
/// or a document often carries ('/Users/me/', "/Users/me/", or curly ones).
fn unquote(input: &str) -> &str {
    let input = input.trim();
    for (open, close) in [('\'', '\''), ('"', '"'), ('\u{2018}', '\u{2019}'), ('\u{201c}', '\u{201d}')] {
        if let Some(inner) = input.strip_prefix(open).and_then(|s| s.strip_suffix(close)) {
            return inner.trim();
        }
    }
    input
}

fn expand_tilde(input: &str, home: Option<&Path>) -> PathBuf {
    let input = unquote(input);
    match (input.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn pasted_paths_lose_their_quotes() {
        let home = Path::new("/Users/me");
        for typed in ["'/Users/me/'", "\"/Users/me/\"", " '/Users/me/' ", "\u{2018}/Users/me/\u{2019}", "\u{201c}/Users/me/\u{201d}"] {
            assert_eq!(expand_tilde(typed, Some(home)), PathBuf::from("/Users/me/"), "{typed}");
        }
        assert_eq!(expand_tilde("'~/Documents'", Some(home)), PathBuf::from("/Users/me/Documents"));
        assert_eq!(expand_tilde("/it's", Some(home)), PathBuf::from("/it's"));
        assert_eq!(expand_tilde("/a'b'", Some(home)), PathBuf::from("/a'b'"));
    }

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
            Action::TreeKeySelect(k) => Some(("select", k.path)),
            Action::TreeExpand(k) => Some(("expand", k.path)),
            Action::TreeCollapse(k) => Some(("collapse", k.path)),
            _ => None,
        }
    }

    fn at(t: &mut FolderTree, root: usize, path: &str, key: Key) -> Option<(&'static str, PathBuf)> {
        t.select(NodeKey { root, path: path.into() });
        target(tree_key_action(&t.rows(), key, 3))
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
        assert_eq!(target(tree_key_action(&t.rows(), Key::ArrowDown, 3)), Some(("select", "/u".into())));
        assert_eq!(target(tree_key_action(&t.rows(), Key::End, 3)), Some(("select", "/u".into())));
    }

    #[test]
    fn home_end_page_and_plus_minus_in_the_tree() {
        // Rows: /u, /u/x, /u/y, /a. A page of 3 rows moves by 2.
        let mut t = tree();
        assert_eq!(at(&mut t, 0, "/u/x", Key::Home), Some(("select", "/u".into())));
        assert_eq!(at(&mut t, 0, "/u", Key::End), Some(("select", "/a".into())));
        assert_eq!(at(&mut t, 0, "/u", Key::PageDown), Some(("select", "/u/y".into())));
        assert_eq!(at(&mut t, 0, "/u/y", Key::PageDown), Some(("select", "/a".into())));
        assert_eq!(at(&mut t, 1, "/a", Key::PageUp), Some(("select", "/u/x".into())));
        assert_eq!(at(&mut t, 0, "/u/x", Key::PageUp), Some(("select", "/u".into())));
        assert_eq!(at(&mut t, 0, "/u", Key::Home), None);
        assert_eq!(at(&mut t, 0, "/u/x", Key::Plus), Some(("expand", "/u/x".into())));
        assert_eq!(at(&mut t, 0, "/u", Key::Plus), None);
        assert_eq!(at(&mut t, 0, "/u", Key::Minus), Some(("collapse", "/u".into())));
        assert_eq!(at(&mut t, 0, "/u/x", Key::Minus), None);
    }

    #[test]
    fn stored_columns_are_used_and_tests_do_not_write_them() {
        let dir = std::env::temp_dir().join(format!("macsplorer-prefs-app-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut app = Explorer::new(egui::Context::default(), dir.clone());
        // Until `main` restores the preferences, nothing is persisted.
        assert!(app.saved_columns.is_none());
        app.columns.toggle(Column::Type);
        app.save_columns();
        assert!(app.saved_columns.is_none());
        assert!(!app.persist);

        let mut stored = ColumnSet::default();
        stored.set_visible(&[Column::Name, Column::Modified]);
        let sort = SortOrder { column: Column::Modified, ascending: false };
        app.restore_preferences(&Preferences { columns: Some(stored.clone()), view: ViewMode::List, sort: Some(sort), ..Default::default() });
        assert_eq!(app.columns, stored);
        assert_eq!(app.saved_columns, Some(stored));
        assert_eq!(app.settings.view, ViewMode::List);
        assert_eq!((app.sort.column, app.sort.ascending), (Column::Modified, false));
        fs::remove_dir_all(&dir).unwrap();
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
        // Not the user's clipboard.
        app.pb = Pasteboard::private();
        app.finish_listing();
        (app, dir)
    }

    #[test]
    fn reveal_in_view_goes_to_the_folder_and_selects_the_file() {
        let (mut app, dir) = explorer("reveal");
        fs::write(dir.join("sub").join("font.ttf"), "x").unwrap();
        fs::write(dir.join("sub").join("other.ttf"), "x").unwrap();
        app.reveal_in_view(dir.join("sub").join("font.ttf"));
        app.finish_listing();
        assert_eq!(app.current(), dir.join("sub"));
        assert_eq!(app.selection, BTreeSet::from([dir.join("sub").join("font.ttf")]));
        assert_eq!(app.focus_item.map(|i| app.entries[i].name.as_str()), Some("font.ttf"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn go_to_a_file_reveals_it_and_a_missing_path_reports_an_error() {
        let (mut app, dir) = explorer("goto");
        fs::write(dir.join("sub").join("font.ttf"), "x").unwrap();
        app.go_to(dir.join("sub").join("font.ttf"));
        app.finish_listing();
        assert_eq!(app.current(), dir.join("sub"));
        assert_eq!(app.selection, BTreeSet::from([dir.join("sub").join("font.ttf")]));
        app.go_to(dir.join("nope"));
        assert_eq!(app.current(), dir.join("sub"));
        assert!(app.status.as_deref().is_some_and(|s| s.contains("is not a folder")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn new_folder_is_created_selected_and_ready_to_rename() {
        let (mut app, dir) = explorer("newfolder");
        app.new_folder();
        app.finish_listing();
        let made = dir.join("New Folder");
        assert!(made.is_dir());
        assert_eq!(app.selection, BTreeSet::from([made.clone()]));
        assert_eq!(app.rename.as_ref().map(|r| (r.path.clone(), r.text.clone())), Some((made, "New Folder".to_string())));
        app.rename = None;
        app.new_folder();
        app.finish_listing();
        assert_eq!(app.rename.as_ref().map(|r| r.text.as_str()), Some("New Folder (2)"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renamed_item_is_scrolled_into_view() {
        let (mut app, dir) = explorer("renamescroll");
        app.new_folder();
        app.finish_listing();
        app.rename.as_mut().unwrap().text = "zzzz".into();
        app.commit_rename();
        app.finish_listing();
        let target = dir.join("zzzz");
        assert_eq!(app.selection, BTreeSet::from([target.clone()]));
        assert_eq!(app.reveal.map(|i| app.entries[i].path.clone()), Some(target));
        fs::remove_dir_all(&dir).unwrap();
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
    fn changes_on_disk_show_up() {
        let (mut app, dir) = explorer("watch");
        app.poll_watcher_at(0.0);
        // FSEvents may still report the fixture being made.
        thread::sleep(std::time::Duration::from_millis(300));
        app.poll_watcher_at(10.0);
        app.finish_listing();
        app.focus_item = app.entries.iter().position(|e| e.name == "a.txt");
        app.selection = BTreeSet::from([dir.join("a.txt")]);
        fs::write(dir.join("0 first.txt"), "new").unwrap();
        let start = std::time::Instant::now();
        while !app.entries.iter().any(|e| e.name == "0 first.txt") && start.elapsed().as_secs() < 10 {
            thread::sleep(std::time::Duration::from_millis(20));
            app.poll_watcher_at(20.0 + start.elapsed().as_secs_f64());
            app.poll_lists();
        }
        assert!(app.entries.iter().any(|e| e.name == "0 first.txt"));
        // Focus and selection stay on a.txt, which moved down a row.
        assert_eq!(app.focus_item.map(|i| app.entries[i].name.as_str()), Some("a.txt"));
        assert_eq!(app.selection, BTreeSet::from([dir.join("a.txt")]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_dropped_from_another_app_go_into_the_open_folder() {
        let (mut app, dir) = explorer("dropfiles");
        let elsewhere = std::env::temp_dir().join(format!("macsplorer-dropfiles-src-{}", std::process::id()));
        let _ = fs::remove_dir_all(&elsewhere);
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("b.txt"), "from outside").unwrap();
        // Same disk: moved.
        app.drop_files(vec![elsewhere.join("b.txt")]);
        wait(&mut app);
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "from outside");
        assert!(!elsewhere.join("b.txt").exists());
        // A file already in the open folder is left alone.
        app.drop_files(vec![dir.join("a.txt")]);
        assert!(app.transfers.len() == 1 && !dir.join("a - Copy.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&elsewhere).unwrap();
    }

    #[test]
    fn copy_paste_and_cut_paste() {
        let (mut app, dir) = explorer("ops");
        app.selection = BTreeSet::from([dir.join("a.txt")]);
        app.apply(vec![Action::Copy, Action::Paste(None)], &[]);
        wait(&mut app);
        assert_eq!(fs::read_to_string(dir.join("a - Copy.txt")).unwrap(), "hi");
        assert!(app.clipboard.is_some(), "a copy can be pasted again");
        // The pasted item is selected, focused and scrolled to once the folder reloads.
        app.finish_listing();
        let copy = app.entries.iter().position(|e| e.path == dir.join("a - Copy.txt"));
        assert!(copy.is_some() && app.reveal == copy && app.focus_item == copy);
        assert_eq!(app.selection, BTreeSet::from([dir.join("a - Copy.txt")]));
        // Over in well under a second: its progress dialog closes unseen.
        assert!(app.transfers.len() == 1 && app.transfers[0].finished);
        app.show_transfers();
        assert!(app.transfers.is_empty());

        app.selection = BTreeSet::from([dir.join("a.txt")]);
        app.apply(vec![Action::Cut, Action::Paste(Some(dir.join("sub")))], &[]);
        wait(&mut app);
        assert!(!dir.join("a.txt").exists());
        assert_eq!(fs::read_to_string(dir.join("sub/a.txt")).unwrap(), "hi");
        assert!(app.clipboard.is_none(), "a cut is pasted once");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Looking for `.git` in every folder is only done for the Branch column.
    #[test]
    fn branches_are_only_looked_up_while_the_branch_column_is_shown() {
        // Waits for the lookup that `main` starts in the background.
        if git::executable().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("macsplorer-branch-col-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("repo/.git")).unwrap();
        fs::write(dir.join("repo/.git/HEAD"), "ref: refs/heads/topic\n").unwrap();
        let branch = |app: &Explorer| app.entries.iter().find(|e| e.name == "repo").and_then(|e| e.branch.clone());

        let mut app = Explorer::new(egui::Context::default(), dir.clone());
        app.finish_listing();
        assert!(!app.list_branches);
        assert_eq!(branch(&app), None);

        app.columns.toggle(Column::Branch);
        app.list_branches_if_missing();
        app.finish_listing();
        assert!(app.list_branches);
        assert_eq!(branch(&app).as_deref(), Some("topic"));

        // Once listed with branches, nothing lists again.
        let request = app.list_request;
        app.list_branches_if_missing();
        assert_eq!(app.list_request, request);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The Git summary is asked for when a working tree is entered or the
    /// folder is reloaded, not again for every folder inside it.
    #[test]
    fn the_git_summary_is_kept_while_moving_inside_a_working_tree() {
        let Some(exe) = git::executable() else { return };
        let dir = std::env::temp_dir().join(format!("macsplorer-git-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        assert!(std::process::Command::new(exe).args(["init", "-q"]).current_dir(&dir).status().unwrap().success());
        fs::write(dir.join("a.txt"), "x").unwrap();

        let untracked = |app: &Explorer| app.git.as_ref().map(|g| g.working.added);
        let next = |app: &mut Explorer| {
            let result = app.git_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("git answer");
            app.git_received(result);
        };
        let mut app = Explorer::new(egui::Context::default(), dir.clone());
        app.finish_listing();
        while app.git_applied != app.git_request {
            next(&mut app);
        }
        assert_eq!(untracked(&app), Some(1));

        // Moving into a subfolder keeps the summary, even though the tree changed.
        fs::write(dir.join("b.txt"), "x").unwrap();
        app.navigate(dir.join("sub"));
        app.finish_listing();
        next(&mut app);
        assert_eq!(untracked(&app), Some(1));

        // A reload asks again.
        app.reload();
        while app.git_applied != app.git_request {
            next(&mut app);
        }
        assert_eq!(untracked(&app), Some(2));

        // Leaving the working tree drops it at once.
        app.navigate(dir.parent().unwrap().to_path_buf());
        assert_eq!(untracked(&app), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Another Macsplorer window is another process, so all it shares with this
    /// one is the pasteboard.
    #[test]
    fn a_second_window_pastes_what_the_first_copied_or_cut() {
        let (mut one, dir) = explorer("windows");
        let mut two = Explorer::new(egui::Context::default(), dir.join("sub"));
        two.pb = one.pb.clone();
        two.finish_listing();
        two.refresh_clipboard();
        assert!(!two.can_paste());

        one.selection = BTreeSet::from([dir.join("a.txt")]);
        one.apply(vec![Action::Copy], &[]);
        two.refresh_clipboard();
        assert_eq!(two.can_paste(), pasteboard::SUPPORTED);
        if !pasteboard::SUPPORTED {
            fs::remove_dir_all(&dir).unwrap();
            return;
        }
        two.apply(vec![Action::Paste(None)], &[]);
        wait(&mut two);
        assert_eq!(fs::read_to_string(dir.join("sub/a.txt")).unwrap(), "hi");
        assert!(dir.join("a.txt").exists());
        assert!(two.can_paste(), "a copy can be pasted again");

        // Move Item Here moves a copy and leaves it on the clipboard.
        fs::create_dir(dir.join("moved")).unwrap();
        two.apply(vec![Action::MoveHere(Some(dir.join("moved")))], &[]);
        wait(&mut two);
        assert!(dir.join("moved/a.txt").exists() && !dir.join("a.txt").exists());

        // A cut in one window is pasted once, as a move, in the other.
        fs::write(dir.join("b.txt"), "b").unwrap();
        one.reload();
        one.finish_listing();
        one.selection = BTreeSet::from([dir.join("b.txt")]);
        one.apply(vec![Action::Cut], &[]);
        two.refresh_clipboard();
        two.apply(vec![Action::Paste(None)], &[]);
        wait(&mut two);
        assert!(dir.join("sub/b.txt").exists() && !dir.join("b.txt").exists());
        two.refresh_clipboard();
        one.refresh_clipboard();
        assert!(!two.can_paste() && !one.can_paste(), "the cut was used up");
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
        let ty = |app: &mut Explorer, text: &str, time_ms| app.apply(vec![Action::TypeAhead { text: text.into(), time_ms, extend: false }], &visible);

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
    fn space_works_on_the_focused_item() {
        let (mut app, dir) = explorer("space");
        for name in ["a", "b", "c"] {
            fs::write(dir.join(name), "").unwrap();
        }
        app.reload();
        app.finish_listing();
        let visible: Vec<usize> = (0..app.entries.len()).collect();
        let index = |app: &Explorer, name: &str| app.entries.iter().position(|e| e.name == name).unwrap();
        let space = |app: &mut Explorer, extend| app.apply(vec![Action::TypeAhead { text: " ".into(), time_ms: 5000, extend }], &visible);
        let (a, b, c) = (index(&app, "a"), index(&app, "b"), index(&app, "c"));

        app.apply(vec![Action::Click { index: a, toggle: false, extend: false, on_label: false }], &visible);
        // The focus moved off the selection, as a click with ⌘ leaves it.
        app.focus_item = Some(b);
        // Space toggles the focused item in and out of the selection.
        space(&mut app, false);
        assert_eq!(app.selection, BTreeSet::from([dir.join("a"), dir.join("b")]));
        space(&mut app, false);
        assert_eq!(app.selection, BTreeSet::from([dir.join("a")]));
        // Shift+Space selects from the anchor to the focus.
        app.apply(vec![Action::Click { index: a, toggle: false, extend: false, on_label: false }], &visible);
        app.focus_item = Some(c);
        space(&mut app, true);
        // With the fixture's "sub" first and "a.txt" between a and b.
        assert_eq!(app.selection, BTreeSet::from(["a", "a.txt", "b", "c"].map(|n| dir.join(n))));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn focus_stops() {
        let (mut app, dir) = explorer("stops");
        fs::write(dir.join("a"), "").unwrap();
        app.reload();
        app.finish_listing();
        let visible: Vec<usize> = (0..app.entries.len()).collect();
        assert!(!app.show_focus);
        // The view gets a focused item to show the focus on.
        app.apply(vec![Action::FocusStop(Stop::View)], &visible);
        assert_eq!(app.focus_item, Some(0));
        assert!(app.show_focus && app.selection.is_empty());
        app.apply(vec![Action::FocusStop(Stop::Address)], &visible);
        assert_eq!(app.address_edit.as_deref(), Some(dir.to_str().unwrap()));
        assert!(app.address_needs_focus);
        app.apply(vec![Action::FocusStop(Stop::Search)], &visible);
        assert!(app.search_needs_focus);
        app.apply(vec![Action::FocusStop(Stop::Folders)], &visible);
        assert!(app.focus == Pane::Tree);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A paste onto a name the folder has asks, and No leaves both alone.
    /// (Yes would put the old file in the real Trash.)
    #[test]
    fn paste_conflict_asks() {
        let (mut app, dir) = explorer("conflict");
        fs::write(dir.join("sub/a.txt"), "old").unwrap();
        app.selection = BTreeSet::from([dir.join("a.txt")]);
        app.apply(vec![Action::Cut, Action::Paste(Some(dir.join("sub")))], &[]);
        let ask = app.conflict_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("a question");
        assert_eq!(ask.conflict.kind, macsplorer_core::fileops::ConflictKind::Item);
        assert_eq!(ask.conflict.existing, dir.join("sub/a.txt"));
        assert_eq!(ask.existing.as_ref().map(|e| e.size), Some(Some(3)));
        ConflictDialog::new(ask).answer(Choice::No);
        wait(&mut app);
        assert_eq!(app.status, None);
        assert_eq!(fs::read_to_string(dir.join("sub/a.txt")).unwrap(), "old");
        assert!(dir.join("a.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod screenshot {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
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
        for d in ["Projects/2026", "Projects/Archive", "Pictures", "Setup.app", ".config"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        for name in ["Projects/plan.txt", "Projects/2026/budget.xlsx", "Projects/Archive/old.zip"] {
            fs::write(dir.join(name), vec![b'x'; 400_000]).unwrap();
        }
        for name in ["main.rs", "app.py", "index.ts", "README.md", "Makefile", "Cargo.toml", "styles.css", "package.json", "photo.png", "archive.zip", "Dockerfile", "notes.txt", "report.pdf", "query.sql", "script.sh", "budget.xlsx", "data.yaml", "Quarterly report for the board of directors.txt"] {
            fs::write(dir.join(name), vec![b'x'; 1_500]).unwrap();
        }
        fs::write(dir.join(".gitignore"), "target\n").unwrap();
        make_repo(&dir.join("Projects"));
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.join("script.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        // A real app (via symlink) to show a native icon.
        let _ = std::os::unix::fs::symlink("/System/Applications/Calculator.app", dir.join("Calculator.app"));
        // A relative link to a file.
        let _ = std::os::unix::fs::symlink("notes.txt", dir.join("Link to notes.txt"));
        dir
    }

    /// Turns `dir` into a Git repository on `main` with one change staged, one
    /// not staged and a file that isn't tracked. Falls back to an empty `.git`
    /// folder (which still shows in the Branch column) if git can't run.
    fn make_repo(dir: &Path) {
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.name=Shot", "-c", "user.email=shot@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(dir)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        let ok = git(&["init", "-q", "-b", "main"])
            && git(&["add", "."])
            && git(&["commit", "-q", "-m", "first"])
            && fs::write(dir.join("plan.txt"), "changed\n").is_ok()
            && fs::write(dir.join("2026/budget.xlsx"), "changed\n").is_ok()
            && git(&["add", "2026/budget.xlsx"])
            && fs::write(dir.join("todo.txt"), "new\n").is_ok();
        if !ok {
            let _ = fs::create_dir_all(dir.join(".git"));
        }
    }

    /// Waits for the Git status of the open folder, which arrives on a worker thread.
    fn finish_git(app: &mut Explorer) {
        while let Ok(result) = app.git_rx.recv_timeout(std::time::Duration::from_secs(10)) {
            app.git_received(result);
            if app.git_applied == app.git_request {
                break;
            }
        }
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
                // `SHOT_DARK=1 cargo test -p macsplorer -- --ignored` renders the dark palette.
                theme::set_dark(&cc.egui_ctx, std::env::var_os("SHOT_DARK").is_some());
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

    /// The Git summary in the status bar, inside a working tree.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn git_status() {
        shoot("git-status", egui::vec2(900.0, 400.0), |app| {
            app.navigate(app.current().join("Projects"));
            app.finish_listing();
            finish_git(app);
        });
    }

    /// The Branch column, beside a folder that is a working tree.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn git_branch_column() {
        shoot("git-branch-column", egui::vec2(900.0, 400.0), |app| {
            // `main` does the lookup in the background; a test waits for it.
            git::executable();
            app.columns.toggle(Column::Branch);
        });
    }

    /// A folder of customized folders, next to the fixture: tag colors,
    /// colors of Macsplorer's own, emblems, and combinations.
    #[cfg(target_os = "macos")]
    fn customized_folders() -> PathBuf {
        use macsplorer_core::folder_style::{Emblem, FolderColor, FolderStyle, TagColor, write};
        let dir = fixture().parent().unwrap().join("Customized");
        let _ = fs::remove_dir_all(&dir);
        let styles = [
            ("Launch Plans", Some(FolderColor::Tag(TagColor::Red)), Some(Emblem::Emoji("🚀".into()))),
            ("Recipes", Some(FolderColor::Custom([0, 128, 128])), Some(Emblem::Emoji("🍲".into()))),
            ("Home", Some(FolderColor::Tag(TagColor::Purple)), Some(Emblem::Symbol("house.fill".into()))),
            ("Cycling", Some(FolderColor::Custom([255, 128, 192])), Some(Emblem::Symbol("figure.outdoor.cycle".into()))),
            ("Old Taxes", Some(FolderColor::Custom([128, 64, 0])), None),
            ("Invoices", Some(FolderColor::Tag(TagColor::Green)), None),
            ("Notes", None, Some(Emblem::Emoji("📝".into()))),
            ("Plain", None, None),
        ];
        for (name, color, emblem) in styles {
            fs::create_dir_all(dir.join(name)).unwrap();
            write(&dir.join(name), &FolderStyle { color, emblem }).unwrap();
        }
        dir
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn customized_large_icons() {
        shoot("customized-large-icons", egui::vec2(900.0, 400.0), |app| {
            app.navigate(customized_folders());
            app.finish_listing();
            app.settings.view = ViewMode::LargeIcons;
        });
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn customized_details() {
        shoot("customized-details", egui::vec2(900.0, 400.0), |app| {
            app.navigate(customized_folders());
            app.finish_listing();
            app.settings.view = ViewMode::Details;
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

    /// A window on a fresh folder with "sub" and "a.txt", for driving drags.
    fn drag_harness(name: &str) -> (Harness<'static, Explorer>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("macsplorer-drag-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), "hi").unwrap();
        let start = dir.clone();
        let harness = Harness::builder().with_size(egui::vec2(900.0, 560.0)).build_eframe(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = Explorer::new(cc.egui_ctx.clone(), start.clone());
            app.pb = Pasteboard::private();
            app.finish_listing();
            app
        });
        (harness, dir)
    }

    /// Runs frames until `done` or a few seconds have passed.
    fn run_until(harness: &mut Harness<'_, Explorer>, done: impl Fn(&Explorer) -> bool) {
        for _ in 0..300 {
            harness.run_steps(2);
            if done(harness.state()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out");
    }

    #[test]
    fn dragging_a_file_onto_a_folder_moves_it() {
        let (mut harness, dir) = drag_harness("move");
        harness.run_steps(2);
        let from = harness.get_by_label("a.txt").rect().center();
        let to = harness.get_by_label("sub").rect().center();
        drag_on(&mut harness, true, from, to);
        assert!(harness.state().drag.as_ref().is_some_and(|d| d.over.as_ref().is_some_and(|o| o.dir == dir.join("sub") && o.effect == Effect::Move)));
        harness.drop_at(to);
        run_until(&mut harness, |_| dir.join("sub/a.txt").exists());
        assert!(!dir.join("a.txt").exists());
        assert!(harness.state().drag.is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_drag_onto_a_file_or_itself_does_nothing() {
        let (mut harness, dir) = drag_harness("refuse");
        harness.run_steps(2);
        let file = harness.get_by_label("a.txt").rect().center();
        let sub = harness.get_by_label("sub").rect().center();
        // The folder onto itself, then onto the file.
        drag_on(&mut harness, true, sub, sub + egui::vec2(10.0, 0.0));
        assert!(harness.state().drag.as_ref().is_some_and(|d| d.over.is_none()));
        drag_on(&mut harness, false, sub + egui::vec2(10.0, 0.0), file);
        assert!(harness.state().drag.as_ref().is_some_and(|d| d.over.is_none()));
        harness.drop_at(file);
        harness.run_steps(2);
        assert!(harness.state().drag.is_none());
        assert!(dir.join("sub").is_dir() && dir.join("a.txt").is_file());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Presses at `from` (unless `press` is false: the button is already down) and
    /// moves the pointer to `to` in steps. Runs a fixed number of frames:
    /// the tree's background loads keep asking for more.
    fn drag_on(harness: &mut Harness<'_, Explorer>, press: bool, from: egui::Pos2, to: egui::Pos2) {
        if press {
            harness.hover_at(from);
            harness.run_steps(2);
            harness.drag_at(from);
            harness.run_steps(2);
        }
        for step in 1..=8 {
            harness.hover_at(from + (to - from) * (step as f32 / 8.0));
            harness.run_steps(2);
        }
    }

    /// After keyboard use the focused item gets a dotted rectangle, here
    /// on an item outside the selection.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn keyboard_focus() {
        shoot("keyboard-focus", egui::vec2(900.0, 400.0), |app| {
            select(app, &["main.rs"]);
            app.focus_item = app.entries.iter().position(|e| e.name == "Pictures");
            app.show_focus = true;
        });
    }

    /// The focus rectangle on a selected item, in the List view.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn keyboard_focus_list() {
        shoot("keyboard-focus-list", egui::vec2(900.0, 400.0), |app| {
            app.settings.view = ViewMode::List;
            select(app, &["main.rs"]);
            app.show_focus = true;
        });
    }

    /// With the Folders pane focused by Tab, its selected row has it.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn keyboard_focus_tree() {
        shoot("keyboard-focus-tree", egui::vec2(900.0, 400.0), |app| {
            select(app, &["main.rs"]);
            // A row in sight; the fixture folder is deep under the disk.
            let key = app.tree.rows()[1].key.clone();
            app.tree.select(key);
            app.focus = Pane::Tree;
            app.show_focus = true;
        });
    }

    /// End in the Folders pane jumps to the last row, scrolled into sight,
    /// while the view still shows the old folder until the open delay ends.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn keyboard_tree_end() {
        shoot_driven(
            "keyboard-tree-end",
            egui::vec2(900.0, 300.0),
            |app| {
                let key = app.tree.rows()[1].key.clone();
                app.tree.select(key);
                app.focus = Pane::Tree;
            },
            |h| {
                // The harness advances the clock a quarter second per
                // frame; hold it still so the delay doesn't run out.
                let now = h.ctx.input(|i| i.time);
                h.key_press(Key::End);
                for _ in 0..4 {
                    h.input_mut().time = Some(now);
                    h.step();
                }
            },
        );
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn list() {
        shoot("list", egui::vec2(900.0, 400.0), |app| {
            app.settings.view = ViewMode::List;
            select(app, &["main.rs", "Setup.app"]);
        });
    }

    /// Hidden items are ghosted like cut ones, in the list and the tree.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn hidden_items() {
        shoot("hidden-items", egui::vec2(900.0, 400.0), |app| {
            app.settings.show_hidden = true;
            app.reload();
            app.finish_listing();
        });
    }

    /// Two files dragged onto a folder, button still down: the drag image
    /// and the drop target drawn selected.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn drag_items() {
        shoot_driven(
            "drag-items",
            egui::vec2(900.0, 560.0),
            |app| select(app, &["notes.txt", "photo.png"]),
            |h| {
                let from = h.get_by_label("notes.txt").rect().center();
                let to = h.get_by_label("Pictures").rect().center() + egui::vec2(6.0, 2.0);
                drag(h, from, to);
            },
        );
    }

    /// The same over a collapsed folder in the Folders pane.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn drag_to_tree() {
        shoot_driven(
            "drag-to-tree",
            egui::vec2(900.0, 560.0),
            |app| select(app, &["notes.txt"]),
            |h| {
                let from = h.get_by_label("notes.txt").rect().center();
                drag(h, from, egui::pos2(110.0, 205.0));
            },
        );
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

    /// An item's context menu with the Open With submenu open, app icons beside the names.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    #[cfg(target_os = "macos")]
    fn open_with_menu() {
        shoot_driven("open-with-menu", egui::vec2(900.0, 500.0), |_| {}, |h| {
            let pos = egui::pos2(275.0, 378.0);
            h.hover_at(pos);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
                h.run();
            }
            h.run();
            let item = egui::pos2(315.0, 202.0);
            h.hover_at(item);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run();
            }
            for _ in 0..5 {
                h.run();
            }
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

    /// Confirm File Replace and Confirm Folder Replace for two real items.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn confirm_replace() {
        use macsplorer_core::fileops::{Conflict, ConflictKind};
        let dir = std::env::temp_dir().join(format!("macsplorer-shot-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for sub in ["old/Reports", "new/Reports"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        fs::write(dir.join("old/notes.md"), "x".repeat(2200)).unwrap();
        fs::write(dir.join("new/notes.md"), "x".repeat(3080)).unwrap();
        for (name, kind, item) in [("confirm-file-replace", ConflictKind::Item, "notes.md"), ("confirm-folder-replace", ConflictKind::Folder, "Reports")] {
            let (existing, source) = (dir.join("old").join(item), dir.join("new").join(item));
            let (reply, _answer) = channel();
            let ask = conflict_dialog::Ask {
                conflict: Conflict { kind, source: source.clone(), existing: existing.clone(), moving: false },
                folder: entry_at(&dir.join("old")),
                existing: entry_at(&existing),
                source: entry_at(&source),
                reply,
            };
            let dialog = ConflictDialog::new(ask);
            let mut harness = Harness::builder().with_size(dialog.size()).with_pixels_per_point(2.0).wgpu().build_ui_state(
                |ui, state: &mut (ConflictDialog, Option<FileIcons>)| {
                    let Some(icons) = &mut state.1 else {
                        theme::install(ui.ctx());
                        theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                        state.1 = Some(FileIcons::new());
                        return;
                    };
                    let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                    state.0.ui(&mut window, icons);
                },
                (dialog, None),
            );
            for _ in 0..3 {
                harness.run();
            }
            let image = harness.render().expect("render");
            let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
            fs::create_dir_all(&out).unwrap();
            image.save(out.join(format!("{name}.png"))).unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The About box.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn about_dialog() {
        let mut harness = Harness::builder().with_size(crate::about_dialog::SIZE).with_pixels_per_point(2.0).wgpu().build_ui_state(
            |ui, state: &mut (AboutDialog, bool)| {
                if !state.1 {
                    theme::install(ui.ctx());
                    theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                    state.1 = true;
                    return;
                }
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                state.0.ui(&mut window);
            },
            (AboutDialog::new(), false),
        );
        for _ in 0..3 {
            harness.run();
        }
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join("about.png")).unwrap();
    }

    /// The Open With dialog with the apps that open a text file.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn open_with_dialog() {
        let file = std::env::temp_dir().join("macsplorer-open-with-dialog.txt");
        fs::write(&file, "hi").unwrap();
        let dialog = OpenWithDialog::new(file.clone(), open_with::apps_for(&file));
        let mut harness = Harness::builder().with_size(open_with_dialog::SIZE).with_pixels_per_point(2.0).wgpu().build_ui_state(
            |ui, state: &mut (OpenWithDialog, Option<FileIcons>)| {
                let Some(icons) = &mut state.1 else {
                    theme::install(ui.ctx());
                    theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                    state.1 = Some(FileIcons::new());
                    return;
                };
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                state.0.ui(&mut window, icons);
            },
            (dialog, None),
        );
        for _ in 0..3 {
            harness.run();
        }
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join("open-with-dialog.png")).unwrap();
        fs::remove_file(&file).ok();
    }

    /// The Copying... dialog part way through a paste.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn copy_progress() {
        use macsplorer_core::fileops::ProgressView;
        let view = ProgressView {
            done: 41,
            total: 100,
            item: Some((PathBuf::from("/Users/jan/Documents/Quarterly report final version 3.pdf"), PathBuf::from("/Volumes/Backup/Reports"))),
        };
        let mut harness = Harness::builder().with_size(crate::progress_dialog::ProgressDialog::size()).with_pixels_per_point(2.0).wgpu().build_ui_state(
            |ui, ready: &mut bool| {
                if !*ready {
                    theme::install(ui.ctx());
                    theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                    *ready = true;
                    return;
                }
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                crate::progress_dialog::draw(&mut window, "Copying...", &view, "12 Seconds Remaining", 0.55, true);
            },
            false,
        );
        for _ in 0..3 {
            harness.run();
        }
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join("copy-progress.png")).unwrap();
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
        shoot_properties_of(shot, &[name]);
    }

    /// The dialog for one fixture item, or for several of them selected together.
    fn shoot_properties_of(shot: &str, names: &[&str]) {
        let listing = list_dir(&fixture(), false).unwrap();
        let entries: Vec<Entry> = names.iter().map(|n| listing.iter().find(|e| e.path == fixture().join(n)).unwrap().clone()).collect();
        shoot_properties_dialog(shot, entries, |_| {});
    }

    /// The dialog for `entries`, with `setup` run on it once it exists.
    fn shoot_properties_dialog(shot: &str, entries: Vec<Entry>, setup: impl Fn(&mut Properties)) {
        let entry = entries[0].clone();
        let size = if entries.len() > 1 { crate::properties::size_for_multi() } else { crate::properties::size_for(&entry) };
        let mut harness = Harness::builder()
            .with_size(size)
            .with_pixels_per_point(2.0)
            .wgpu()
            .build_ui_state(
                |ui, (dialog, icons): &mut (Option<Properties>, FileIcons)| {
                    // Fonts installed now apply from the next frame.
                    let Some(dialog) = dialog else {
                        theme::install(ui.ctx());
                        theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                        let mut new = if entries.len() > 1 {
                            Properties::new_multi(ui.ctx(), entries.clone(), true)
                        } else {
                            Properties::new(ui.ctx(), entry.clone(), true, true)
                        };
                        setup(&mut new);
                        *dialog = Some(new);
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
    fn properties_selection() {
        shoot_properties_of("properties-selection", &["Projects", "Quarterly report for the board of directors.txt", "budget.xlsx"]);
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn properties_folder() {
        shoot_properties("properties-folder", "Projects");
    }

    /// The Win2000 folder in the folder colors, as Linux draws them: Finder's
    /// seven and two of our own, at List and Large Icons size.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn colored_folders() {
        use macsplorer_core::folder_style::TagColor;
        let colors: Vec<[u8; 3]> = TagColor::ALL.iter().map(|c| c.rgb()).chain([[0, 128, 128], [128, 64, 0]]).collect();
        let mut harness = Harness::builder().with_size(egui::vec2(40.0 * colors.len() as f32 + 8.0, 64.0)).with_pixels_per_point(2.0).wgpu().build_ui(|ui| {
            ui.painter().rect_filled(ui.ctx().content_rect(), 0.0, theme::color::window());
            for (n, [r, g, b]) in colors.iter().copied().enumerate() {
                let x = 8.0 + 40.0 * n as f32;
                let fill = egui::Color32::from_rgb(r, g, b);
                theme::paint_colored_folder(ui.painter(), egui::Rect::from_min_size(egui::pos2(x + 8.0, 6.0), egui::vec2(16.0, 16.0)), fill);
                theme::paint_colored_folder(ui.painter(), egui::Rect::from_min_size(egui::pos2(x, 26.0), egui::vec2(32.0, 32.0)), fill);
            }
        });
        harness.run();
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join("colored-folders.png")).unwrap();
    }

    /// The Color dialog with Define Custom Colors open, on a teal.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn color_dialog() {
        let mut harness = Harness::builder().with_size(egui::vec2(470.0, 334.0)).with_pixels_per_point(2.0).wgpu().build_ui_state(
            |ui, dialog: &mut Option<crate::color_dialog::ColorDialog>| {
                let Some(dialog) = dialog else {
                    theme::install(ui.ctx());
                    theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                    *dialog = Some(crate::color_dialog::ColorDialog::new([0, 128, 160]));
                    return;
                };
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                dialog.ui(&mut window);
            },
            None,
        );
        for _ in 0..5 {
            harness.run();
        }
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join("color-dialog.png")).unwrap();
    }

    /// The Customize tab of a teal folder with a rocket (Finder: green). The
    /// folder lives outside the fixture so the window shots don't change.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn properties_customize() {
        use macsplorer_core::folder_style::{Emblem, FolderColor, FolderStyle};
        let parent = std::env::temp_dir().join("macsplorer-screenshot").join("Customize");
        let folder = parent.join("Launch Plans");
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir_all(&folder).unwrap();
        let style = FolderStyle { color: Some(FolderColor::Custom([0, 128, 128])), emblem: Some(Emblem::Emoji("🚀".into())) };
        macsplorer_core::folder_style::write(&folder, &style).unwrap();
        let entry = list_dir(&parent, false).unwrap().remove(0);
        shoot_properties_dialog("properties-customize", vec![entry], Properties::show_customize);
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn properties_file() {
        shoot_properties("properties-file", "Quarterly report for the board of directors.txt");
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn properties_link() {
        shoot_properties("properties-link", "Link to notes.txt");
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn properties_folder_link() {
        shoot_properties("properties-folder-link", "Calculator.app");
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

    /// The View menu, which is mostly checkboxes.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn view_menu() {
        let dir = std::env::temp_dir().join("macsplorer-screenshot-menu").join("Plain");
        fs::create_dir_all(&dir).unwrap();
        let mut harness = Harness::builder().with_size(egui::vec2(500.0, 360.0)).with_pixels_per_point(2.0).wgpu().build_eframe(|cc| {
            theme::install(&cc.egui_ctx);
            let mut app = Explorer::new(cc.egui_ctx.clone(), dir.clone());
            app.finish_listing();
            app
        });
        for _ in 0..10 {
            harness.run();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let pos = egui::pos2(65.0, 33.0);
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
        image.save(out.join("view-menu.png")).unwrap();
        // The pointer on a radio item and on a checked one.
        for (name, y) in [("view-menu-radio-hover", 121.5), ("view-menu-check-hover", 257.5)] {
            harness.hover_at(egui::pos2(100.0, y));
            harness.run();
            harness.run();
            harness.render().expect("render").save(out.join(format!("{name}.png"))).unwrap();
        }
        // And its Text Size submenu.
        harness.hover_at(egui::pos2(92.0, 150.0));
        for _ in 0..10 {
            harness.run();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        harness.render().expect("render").save(out.join("view-menu-text-size.png")).unwrap();
    }

    /// An item's context menu (for the user manual).
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn item_menu() {
        shoot_driven("item-menu", egui::vec2(900.0, 500.0), |_| {}, |h| {
            let pos = egui::pos2(275.0, 378.0);
            h.hover_at(pos);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
                h.run();
            }
            h.run();
        });
    }

    /// The Edit menu with its keyboard shortcuts.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn edit_menu() {
        shoot_driven("edit-menu", egui::vec2(900.0, 500.0), |app| select(app, &["data.yaml"]), |h| {
            let pos = egui::pos2(40.0, 33.0);
            h.hover_at(pos);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run();
            }
            h.run();
        });
    }

    /// Rename in place, with the name selected up to the extension.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn rename() {
        shoot("rename", egui::vec2(900.0, 400.0), |app| {
            select(app, &["data.yaml"]);
            app.begin_rename();
        });
    }

    /// The search box filtering the folder.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn search() {
        shoot("search", egui::vec2(900.0, 400.0), |app| app.search = "re".into());
    }

    /// View > Text Size at 150%.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn text_size_larger() {
        shoot("text-size-larger", egui::vec2(1000.0, 680.0), |app| app.ctx.set_zoom_factor(1.5));
    }

    /// A window small enough that both panes need classic scroll bars.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn scroll_bars() {
        shoot("scroll-bars", egui::vec2(560.0, 300.0), |_| {});
    }
}
