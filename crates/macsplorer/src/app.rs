//! The prototype window: toolbar, breadcrumb, navigation pane, details view, status bar.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use chrono::{DateTime, Local};
use eframe::egui;
use egui::text::CCursor;
use egui::text_selection::CCursorRange;
use egui::{Align, Key, Layout, RichText, Sense, TextEdit, Ui};
use egui_extras::{Column, TableBuilder};

use macsplorer_core::breadcrumb::crumbs;
use macsplorer_core::history::History;
use macsplorer_core::listing::{Entry, SortColumn, SortSpec, format_byte_size, format_size, list_dir, sort_entries};
use macsplorer_core::natural::natural_cmp;
use macsplorer_core::places::{favorites, volumes};
use macsplorer_core::rename::{RenameKind, initial_selection, is_invalid_char, rename_no_replace, validate_name};
use macsplorer_core::tree::{FolderTree, Node, NodeKey, Root, Row, Section, load_children};

use crate::icons::FileIcons;
use crate::open_with;
use crate::theme::{self, Bevel, Icon};

const LIST_ROW_HEIGHT: f32 = 17.0;
const TREE_ROW_HEIGHT: f32 = 18.0;
const TREE_INDENT: f32 = 19.0;

type LoadResult = (PathBuf, bool, Result<Vec<Node>, String>);
/// A details-view listing, tagged with the request it answers.
type ListResult = (u64, Result<Vec<Entry>, String>);
/// Search-filtered rows and the (entries version, extensions shown, needle) they came from.
type VisibleCache = ((u64, bool, String), Rc<[usize]>);

struct Settings {
    show_extensions: bool,
    show_hidden: bool,
    natural_sort: bool,
    /// Explorer's "Expand to open folder" navigation pane option.
    expand_to_open: bool,
    /// The Folders explorer bar (navigation pane) is shown.
    show_folders: bool,
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
    Click { index: usize, toggle: bool, extend: bool },
    BeginRename,
    CommitRename,
    CancelRename,
    Sort(SortColumn),
    Back,
    Forward,
    HistoryJump(usize),
    Up,
    Refresh,
    RevealInFinder(PathBuf),
    TreeSelect(NodeKey),
    TreeToggle(NodeKey),
    TreeExpand(NodeKey),
    TreeCollapse(NodeKey),
    Focus(Pane),
    ToggleFolders,
    SelectAll,
    InvertSelection,
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
    anchor: Option<usize>,
    rename: Option<RenameState>,
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
    focus: Pane,
    /// Set while navigating from a tree click, so the clicked row stays
    /// selected instead of jumping to another root that has the same folder.
    navigating_from_tree: bool,
    icons: FileIcons,
    /// The "Open With" apps for the file whose context menu is open.
    open_with: Option<(PathBuf, Vec<PathBuf>)>,
    ctx: egui::Context,
    home: Option<PathBuf>,
}

impl Explorer {
    pub fn new(ctx: egui::Context, start: PathBuf) -> Self {
        let (load_tx, load_rx) = channel();
        let (list_tx, list_rx) = channel();
        let mut app = Self {
            history: History::new(start),
            entries: Vec::new(),
            entries_version: 0,
            visible_cache: None,
            load_error: None,
            list_tx,
            list_rx,
            list_request: 0,
            listing: false,
            sort: SortSpec::default(),
            settings: Settings { show_extensions: true, show_hidden: false, natural_sort: true, expand_to_open: true, show_folders: true },
            selection: BTreeSet::new(),
            anchor: None,
            rename: None,
            address_edit: None,
            address_needs_focus: false,
            search: String::new(),
            status: None,
            tree: FolderTree::new(tree_roots()),
            load_tx,
            load_rx,
            focus: Pane::List,
            navigating_from_tree: false,
            icons: FileIcons::new(),
            open_with: None,
            ctx,
            home: dirs::home_dir(),
        };
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
            if tx.send((request, result)).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    fn poll_lists(&mut self) {
        while let Ok((request, result)) = self.list_rx.try_recv() {
            self.listed(request, result);
        }
    }

    fn listed(&mut self, request: u64, result: Result<Vec<Entry>, String>) {
        if request != self.list_request {
            return;
        }
        self.listing = false;
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
    }

    /// Blocks until the latest listing has arrived, for tests.
    #[cfg(test)]
    fn finish_listing(&mut self) {
        while self.listing {
            let (request, result) = self.list_rx.recv().expect("listing thread");
            self.listed(request, result);
        }
    }

    fn resort(&mut self) {
        sort_entries(&mut self.entries, self.sort);
        self.entries_version += 1;
        self.anchor = None;
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
        self.rename = None;
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
        } else if let Err(e) = Command::new("open").arg(&path).spawn() {
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
        if let Err(e) = validate_name(&name) {
            self.status = Some(e.to_string());
            return;
        }
        let target = r.path.with_file_name(&name);
        match rename_no_replace(&r.path, &target) {
            Ok(()) => {
                self.status = None;
                self.reload();
                let loads = self.tree.refresh_path(self.history.current());
                self.load(loads);
                self.selection = BTreeSet::from([target]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                self.status = Some(format!("There is already an item named \"{name}\" here."));
            }
            Err(e) => self.status = Some(format!("Couldn't rename: {e}")),
        }
    }

    fn apply(&mut self, actions: Vec<Action>, visible: &[usize]) {
        for action in actions {
            match action {
                Action::Navigate(p) => self.navigate(p),
                Action::Open(p) => self.open(p),
                Action::OpenWith(p, app) => self.open_with(p, app),
                Action::Click { index, toggle, extend } => {
                    self.focus = Pane::List;
                    self.click(index, toggle, extend, visible);
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
                    let _ = Command::new("open").arg("-R").arg(p).spawn();
                }
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
                Action::Focus(pane) => self.focus = pane,
                Action::ToggleFolders => {
                    self.settings.show_folders = !self.settings.show_folders;
                    if !self.settings.show_folders {
                        self.focus = Pane::List;
                    }
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
            if in_list && i.key_pressed(Key::F2) {
                actions.push(Action::BeginRename);
            }
            if in_list && i.key_pressed(Key::Enter) && let Some(p) = self.selection.first() {
                actions.push(Action::Open(p.clone()));
            }
            if i.key_pressed(Key::Backspace) || (cmd && i.key_pressed(Key::OpenBracket)) || (alt && i.key_pressed(Key::ArrowLeft)) {
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
            let step = match () {
                _ if cmd || alt => 0,
                _ if i.key_pressed(Key::ArrowDown) => 1,
                _ if i.key_pressed(Key::ArrowUp) => -1,
                _ => 0,
            };
            if step != 0 && !visible.is_empty() {
                let cur = self.anchor.and_then(|a| visible.iter().position(|&v| v == a));
                let next = match cur {
                    Some(p) => (p as isize + step).clamp(0, visible.len() as isize - 1) as usize,
                    None => 0,
                };
                actions.push(Action::Click { index: visible[next], toggle: false, extend: i.modifiers.shift });
            }
        });
    }

    // ---- menu bar, toolbar and address bar --------------------------------

    fn menu_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Close").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
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
                let mut changed = false;
                changed |= ui.checkbox(&mut self.settings.show_extensions, "File name extensions").changed();
                changed |= ui.checkbox(&mut self.settings.show_hidden, "Hidden files and folders").changed();
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
            let back = theme::tool_button(ui, Icon::Back, Some("Back"), self.history.can_go_back()).on_hover_text("Back (⌘[)");
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
            let fwd = theme::tool_button(ui, Icon::Forward, None, self.history.can_go_forward()).on_hover_text("Forward (⌘])");
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
            if theme::tool_button(ui, Icon::Up, None, has_parent).on_hover_text("Up (⌘↑)").clicked() {
                actions.push(Action::Up);
            }
            theme::etched_vline(ui, 22.0);
            let folders = theme::tool_toggle(ui, Icon::Folders, "Folders", self.settings.show_folders);
            if folders.clicked() {
                actions.push(Action::ToggleFolders);
            }
            if theme::tool_button(ui, Icon::Refresh, Some("Refresh"), true).on_hover_text("Refresh (⌘R)").clicked() {
                actions.push(Action::Refresh);
            }
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
        for crumb in crumbs(self.current(), self.home.as_deref()) {
            if crumb_button(ui, &crumb.label).clicked() {
                actions.push(Action::Navigate(crumb.path.clone()));
            }
            // Explorer's chevron: a menu of the crumb's subfolders.
            let chevron = crumb_button(ui, "›");
            egui::Popup::menu(&chevron).show(|ui| {
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for (name, path) in subfolders(&crumb.path, self.settings.show_hidden) {
                        if ui.button(name).clicked() {
                            actions.push(Action::Navigate(path));
                        }
                    }
                });
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
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
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
                tree_row(ui, row, guide, scroll, focused, actions);
            }
        });
    }

    // ---- details view --------------------------------------------------

    fn details(&mut self, ui: &mut Ui, visible: &[usize], actions: &mut Vec<Action>) {
        let rect = ui.available_rect_before_wrap();
        let inner = theme::sunken_field(ui.painter(), rect);
        ui.allocate_rect(rect, Sense::hover());
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        let ui = &mut ui;

        if let Some(err) = &self.load_error {
            ui.centered_and_justified(|ui| ui.label(err));
            return;
        }
        let focused = self.focus == Pane::List && window_focused(ui);
        let Self { entries, selection, rename, sort, settings, search, icons, open_with, listing, .. } = self;
        // Classic list views don't highlight rows on hover.
        ui.visuals_mut().widgets.hovered.bg_fill = theme::color::WINDOW;
        ui.spacing_mut().item_spacing.x = 0.0;

        let header = |ui: &mut Ui, label: &str, column: SortColumn, right: bool, actions: &mut Vec<Action>| {
            let rect = ui.max_rect();
            let resp = ui.interact(rect, ui.id().with(label), Sense::click());
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
        };

        let table = TableBuilder::new(ui)
            .striped(false)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::initial(300.0).at_least(120.0).clip(true))
            .column(Column::initial(80.0).at_least(50.0))
            .column(Column::initial(140.0).at_least(60.0).clip(true))
            .column(Column::remainder().at_least(120.0).clip(true))
            .header(18.0, |mut row| {
                row.col(|ui| header(ui, "Name", SortColumn::Name, false, actions));
                row.col(|ui| header(ui, "Size", SortColumn::Size, true, actions));
                row.col(|ui| header(ui, "Type", SortColumn::Kind, false, actions));
                row.col(|ui| header(ui, "Modified", SortColumn::Modified, false, actions));
            });

        if visible.is_empty() {
            table.body(|_| {});
            if *listing {
                return;
            }
            let msg = if search.is_empty() { "This folder is empty." } else { "No items match your search." };
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(msg).color(theme::color::GRAY_TEXT));
            });
            return;
        }

        table.body(|body| {
            body.rows(LIST_ROW_HEIGHT, visible.len(), |mut row| {
                let index = visible[row.index()];
                let entry = &entries[index];
                let selected = selection.contains(&entry.path);

                row.col(|ui| {
                    ui.add_space(4.0);
                    let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
                    icons.paint(ui, icon_rect, entry);
                    ui.add_space(3.0);
                    match rename.as_mut().filter(|r| r.path == entry.path) {
                        Some(r) => rename_editor(ui, r, actions),
                        None => {
                            let name = entry.display_name(settings.show_extensions);
                            selectable_name(ui, name, selected, focused);
                        }
                    }
                });
                row.col(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(6.0);
                        if let Some(size) = entry.size {
                            ui.label(format_size(size));
                        }
                    });
                });
                row.col(|ui| {
                    ui.add_space(6.0);
                    ui.label(entry.kind_label());
                });
                row.col(|ui| {
                    ui.add_space(6.0);
                    if let Some(t) = entry.modified {
                        ui.label(DateTime::<Local>::from(t).format("%d/%m/%Y %H:%M").to_string());
                    }
                });

                let resp = row.response();
                if resp.clicked() || resp.secondary_clicked() && !selection.contains(&entry.path) {
                    let m = resp.ctx.input(|i| i.modifiers);
                    actions.push(Action::Click { index, toggle: m.command, extend: m.shift });
                }
                if resp.double_clicked() {
                    actions.push(Action::Open(entry.path.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button(RichText::new("Open").font(theme::bold(theme::FONT_SIZE))).clicked() {
                        actions.push(Action::Open(entry.path.clone()));
                    }
                    if entry.kind == RenameKind::File {
                        open_with_menu(ui, &entry.path, open_with, actions);
                    }
                    if ui.button("Show in Finder").clicked() {
                        actions.push(Action::RevealInFinder(entry.path.clone()));
                    }
                    ui.separator();
                    if ui.button("Rename").clicked() {
                        actions.push(Action::BeginRename);
                    }
                });
            });
        });
    }

    fn status_bar(&self, ui: &mut Ui, visible: &[usize]) {
        let selected: Vec<&Entry> = self.entries.iter().filter(|e| self.selection.contains(&e.path)).collect();
        let (objects, bytes) = if selected.is_empty() {
            let n = visible.len();
            let bytes: u64 = visible.iter().filter_map(|&i| self.entries[i].size).sum();
            (format!("{n} object(s)"), bytes)
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
        p.text(egui::pos2(icon.right() + 4.0, right.center().y), egui::Align2::LEFT_CENTER, "My Computer", font, theme::color::TEXT);
    }
}

impl eframe::App for Explorer {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.poll_loads();
        self.poll_lists();
        self.icons.begin_frame();
        let visible = self.visible();
        let mut actions = Vec::new();
        self.keyboard(ui, &visible, &mut actions);

        // The window frame: a raised 3D border around everything.
        let outer = ui.max_rect();
        ui.painter().rect_filled(outer, 0.0, theme::color::FACE);
        theme::bevel(ui.painter(), outer, Bevel::Raised);
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
            .show(ui, |ui| self.details(ui, &visible, &mut actions));

        self.apply(actions, &visible);
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

fn rename_editor(ui: &mut Ui, r: &mut RenameState, actions: &mut Vec<Action>) {
    let id = ui.make_persistent_id(("rename", &r.path));
    let frame = egui::Frame::NONE
        .fill(theme::color::WINDOW)
        .stroke(egui::Stroke::new(1.0, theme::color::TEXT))
        .inner_margin(egui::Margin::symmetric(2, 0));
    let resp = frame
        .show(ui, |ui| ui.add(TextEdit::singleline(&mut r.text).id(id).frame(egui::Frame::NONE).desired_width(f32::INFINITY)))
        .inner;
    if resp.changed() {
        r.text.retain(|c| !is_invalid_char(c));
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
fn selectable_name(ui: &mut Ui, text: &str, selected: bool, focused: bool) {
    let bg = ui.painter().add(egui::Shape::Noop);
    let color = if selected && focused { theme::color::SELECTION_TEXT } else { theme::color::TEXT };
    let resp = ui.add(egui::Label::new(RichText::new(text).color(color)).truncate().selectable(false).sense(Sense::hover()));
    if selected {
        let fill = if focused { theme::color::SELECTION } else { theme::color::FACE };
        ui.painter().set(bg, egui::Shape::rect_filled(resp.rect.expand2(egui::vec2(2.0, 1.0)), 0.0, fill));
    }
}

/// A flat crumb in the address field that pops out when hovered.
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
fn tree_row(ui: &mut Ui, row: &Row, guide: &Guide, scroll: bool, focused: bool, actions: &mut Vec<Action>) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_w = ui.painter().layout_no_wrap(row.label.clone(), font.clone(), theme::color::TEXT).size().x;
    let x0 = ui.max_rect().left() + 2.0;
    let label_x = x0 + (row.depth + 1) as f32 * TREE_INDENT + 16.0 + 4.0;
    let width = (label_x - ui.max_rect().left() + text_w + 6.0).max(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, TREE_ROW_HEIGHT), Sense::click());
    let p = ui.painter();
    let line = egui::Stroke::new(1.0, theme::color::TREE_LINE);
    let dotted = |a: egui::Pos2, b: egui::Pos2| {
        p.extend(egui::Shape::dashed_line(&[a, b], line, 1.0, 1.0));
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
    let icon = if row.depth == 0 && row.section == Section::Locations { Icon::Drive } else { Icon::Folder };
    theme::paint_icon(p, icon_rect, icon, true);
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
        if ui.button("Show in Finder").clicked() {
            actions.push(Action::RevealInFinder(row.key.path.clone()));
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

fn place_label(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
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
mod screenshot {
    use super::*;
    use egui_kittest::Harness;
    use std::fs;

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join("macsplorer-screenshot").join("My Documents");
        let _ = fs::remove_dir_all(&dir);
        for d in ["Projects/2026", "Pictures", "Setup.app"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        for name in ["main.rs", "app.py", "index.ts", "README.md", "Makefile", "Cargo.toml", "styles.css", "package.json", "photo.png", "archive.zip", "Dockerfile", "notes.txt", "report.pdf", "query.sql", "script.sh", "budget.xlsx", "data.yaml"] {
            fs::write(dir.join(name), vec![b'x'; 1_500]).unwrap();
        }
        // A real app (via symlink) to show a native icon.
        let _ = std::os::unix::fs::symlink("/System/Applications/Calculator.app", dir.join("Calculator.app"));
        dir
    }

    fn shoot(name: &str, setup: impl FnOnce(&mut Explorer)) {
        let start = fixture();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 560.0))
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
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        fs::create_dir_all(&out).unwrap();
        image.save(out.join(format!("{name}.png"))).unwrap();
    }

    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn window() {
        shoot("window", |app| {
            let pick = app.entries.iter().find(|e| e.name == "main.rs").map(|e| e.path.clone());
            app.selection.extend(pick);
        });
    }
}
