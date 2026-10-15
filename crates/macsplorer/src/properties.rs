//! The Properties dialog: the General page of shell32's `CFilePropSheetPage`
//! (`_InitSingleFilePrsht`, `_UpdateSizeField`, `_UpdateSizeCount`), in its
//! own borderless window, with the Unix permissions where Windows has its
//! Attributes check boxes and a symlink's source and target.
//! Left out on purpose: Sharing and Security tabs, Size on disk, Opens with
//! and Advanced.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, TextEdit, Ui, pos2, vec2};

use macsplorer_core::listing::{Entry, Link, is_package_name};
use macsplorer_core::properties::{
    FolderSize, FolderTotals, ItemInfo, SelectedItem, format_common_location, format_contains, format_size_line, format_target, item_info, permissions,
};

type Permissions = [[bool; 3]; 3];
use macsplorer_core::rename::RenameKind;

use crate::icons::{FileIcons, IconState};
use crate::locale;
use crate::theme::{self, Bevel};

/// The window's width. Its height is fixed for an item, like a property
/// sheet, but follows the rows the item has (see [`size_for`]).
const WIDTH: f32 = 380.0;
/// The window's height without any group of rows: frame, caption, tab, the
/// icon and name row, and the buttons.
const BASE_HEIGHT: f32 = 155.0;
/// The etched line above a group, and the gap below it.
const GROUP_TOP: f32 = 9.0;
const GROUP_BOTTOM: f32 = 6.0;
/// Where each permission's check box starts, from the first one.
const BOX_STEP: f32 = 76.0;
const FRAME: f32 = 4.0;
const TAB_HEIGHT: f32 = 20.0;
const ROW_HEIGHT: f32 = 22.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
/// Where values start, right of the labels and lined up with the name box.
const VALUE_X: f32 = 86.0;
/// How often a running folder walk's totals are shown, like the 250 ms
/// timer `CFolderContentsInfo::SizeThreadProc` sets.
const SIZE_REFRESH: Duration = Duration::from_millis(250);

/// The window size for `entry`'s page, known before its metadata is read.
pub fn size_for(entry: &Entry) -> egui::Vec2 {
    let is_dir = entry.kind != RenameKind::File;
    // The Source and Target group follows Location, then the dates (files)
    // or Created (folders), then the permissions.
    let mut rows = if is_dir { vec![4, 1] } else { vec![1, 2, 3] };
    if is_linked(entry) {
        rows.insert(if is_dir { 1 } else { 2 }, 2);
    }
    rows.push(PERMISSION_ROWS);
    vec2(WIDTH, BASE_HEIGHT + rows.into_iter().map(group_height).sum::<f32>())
}

/// The window size for a selection of several items: Type, Location, Size
/// and Contains in one group.
pub fn size_for_multi() -> egui::Vec2 {
    vec2(WIDTH, BASE_HEIGHT + group_height(4))
}

fn group_height(rows: usize) -> f32 {
    GROUP_TOP + rows as f32 * ROW_HEIGHT + GROUP_BOTTOM
}

fn is_linked(entry: &Entry) -> bool {
    matches!(entry.link, Some(Link::Symlink | Link::Broken))
}

/// Owner, Group and Others.
const PERMISSION_ROWS: usize = 3;

/// A block of the page between etched lines.
enum Group {
    Rows(Vec<(&'static str, String)>),
    /// The nine permission bits, as [`permissions`] returns them.
    Permissions(Permissions),
}

#[cfg(test)]
impl Group {
    fn rows(&self) -> usize {
        match self {
            Group::Rows(rows) => rows.len(),
            Group::Permissions(_) => PERMISSION_ROWS,
        }
    }
}

/// What the dialog asks the window to do after this frame.
pub enum Outcome {
    Keep,
    Close,
    /// Rename to [`Properties::new_name`] if the name changed, then set
    /// [`Properties::new_permissions`] if they did; OK also closes on success.
    Apply { close: bool },
}

/// What several selected items add to the page: no rename, dates or
/// permissions, only what they have in common and their combined size.
struct Multi {
    entries: Vec<Entry>,
    /// "All of type X" or "Multiple types", once the worker has looked.
    types: Option<String>,
    types_rx: Receiver<String>,
}

pub struct Properties {
    viewport: egui::ViewportId,
    entry: Entry,
    /// The name box. With extensions hidden it holds the stem only.
    name: String,
    hidden_ext: Option<String>,
    editable: bool,
    info: Option<Result<ItemInfo, String>>,
    /// Finder's Kind, used as the type name when there is one.
    kind: Option<String>,
    info_rx: Receiver<(io::Result<ItemInfo>, Option<String>)>,
    /// The permission boxes as shown, once the item has been read.
    permissions: Option<Permissions>,
    /// The permissions on disk, to tell whether the boxes were changed.
    saved_permissions: Option<Permissions>,
    size: Arc<FolderSize>,
    /// The folder totals on screen and when they were taken.
    shown: FolderTotals,
    shown_at: Instant,
    /// Set when the dialog is for a selection of several items.
    multi: Option<Multi>,
}

impl Properties {
    /// Opens the dialog for `entry`. Reading its metadata and, for a folder,
    /// walking it run on a worker thread.
    pub fn new(ctx: &egui::Context, entry: Entry, show_extensions: bool, editable: bool) -> Self {
        let (tx, info_rx) = channel();
        let size = Arc::new(FolderSize::default());
        let (path, walker, ctx2) = (entry.path.clone(), size.clone(), ctx.clone());
        thread::spawn(move || {
            let info = item_info(&path);
            let is_dir = info.as_ref().is_ok_and(|i| i.is_dir);
            if tx.send((info, locale::kind_name(&path))).is_err() {
                return;
            }
            ctx2.request_repaint();
            if is_dir {
                walker.measure(&path);
                ctx2.request_repaint();
            }
        });
        Self {
            viewport: egui::ViewportId::from_hash_of(("properties", &entry.path, Instant::now())),
            name: entry.display_name(show_extensions).to_owned(),
            hidden_ext: entry.hidden_extension(show_extensions).map(str::to_owned),
            entry,
            editable,
            info: None,
            kind: None,
            info_rx,
            permissions: None,
            saved_permissions: None,
            size,
            shown: FolderTotals::default(),
            shown_at: Instant::now(),
            multi: None,
        }
    }

    /// Opens the dialog for several selected items (two or more). The name
    /// box lists them and can't be edited; Size and Contains add them up,
    /// counting the selected folders and walking them.
    pub fn new_multi(ctx: &egui::Context, entries: Vec<Entry>, show_extensions: bool) -> Self {
        let first = entries[0].clone();
        let mut dialog = Self::new_blank(first, show_extensions);
        dialog.name = entries.iter().map(|e| e.display_name(show_extensions)).collect::<Vec<_>>().join(", ");
        dialog.hidden_ext = None;
        let items: Vec<SelectedItem> = entries
            .iter()
            .map(|e| if e.kind == RenameKind::File { SelectedItem::File(e.size.unwrap_or(0)) } else { SelectedItem::Folder(e.path.clone()) })
            .collect();
        let (tx, types_rx) = channel();
        let (walker, ctx2) = (dialog.size.clone(), ctx.clone());
        let for_types = entries.clone();
        thread::spawn(move || {
            if tx.send(common_type(&for_types)).is_err() {
                return;
            }
            ctx2.request_repaint();
            walker.measure_selection(&items);
            ctx2.request_repaint();
        });
        dialog.multi = Some(Multi { entries, types: None, types_rx });
        dialog
    }

    /// A dialog with nothing read yet and no worker thread.
    fn new_blank(entry: Entry, show_extensions: bool) -> Self {
        let (_, info_rx) = channel();
        Self {
            viewport: egui::ViewportId::from_hash_of(("properties", &entry.path, Instant::now())),
            name: entry.display_name(show_extensions).to_owned(),
            hidden_ext: None,
            entry,
            editable: false,
            info: None,
            kind: None,
            info_rx,
            permissions: None,
            saved_permissions: None,
            size: Arc::new(FolderSize::default()),
            shown: FolderTotals::default(),
            shown_at: Instant::now(),
            multi: None,
        }
    }

    /// A folder that isn't in the listing (the open folder, a tree node).
    pub fn folder_entry(path: &Path) -> Entry {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let kind = if is_package_name(&name) { RenameKind::Package } else { RenameKind::Folder };
        Entry {
            name,
            path: path.to_path_buf(),
            kind,
            size: None,
            modified: None,
            created: None,
            accessed: None,
            mode: 0o040755,
            owner: String::new(),
            group: String::new(),
            comment: None,
            hidden: false,
            custom_icon: false,
            git_repo: false,
            // One lstat for a dialog the user just asked for.
            link: fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink()).then_some(Link::Symlink),
        }
    }

    fn size(&self) -> egui::Vec2 {
        if self.multi.is_some() { size_for_multi() } else { size_for(&self.entry) }
    }

    pub fn path(&self) -> &Path {
        &self.entry.path
    }

    /// The full new name, the hidden extension put back like a list rename.
    pub fn new_name(&self) -> String {
        let name = self.name.trim();
        match &self.hidden_ext {
            Some(ext) => format!("{name}.{ext}"),
            None => name.to_owned(),
        }
    }

    fn name_changed(&self) -> bool {
        self.multi.is_none() && self.new_name() != self.entry.name
    }

    fn permissions_changed(&self) -> bool {
        self.permissions != self.saved_permissions
    }

    fn changed(&self) -> bool {
        self.name_changed() || self.permissions_changed()
    }

    /// The boxes' state when they differ from what is on disk.
    pub fn new_permissions(&self) -> Option<Permissions> {
        self.permissions.filter(|_| self.permissions_changed())
    }

    /// The new name when it differs from the current one.
    pub fn name_to_apply(&self) -> Option<String> {
        self.name_changed().then(|| self.new_name())
    }

    /// The permissions went through: the boxes are the saved state now.
    pub fn permissions_applied(&mut self) {
        self.saved_permissions = self.permissions;
    }

    /// The rename went through: show the new name from now on.
    pub fn renamed(&mut self, to: PathBuf) {
        self.entry.name = to.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.entry.path = to;
    }

    fn title(&self) -> String {
        if self.multi.is_some() {
            return "Properties".into();
        }
        let name = self.name.trim();
        format!("{} Properties", if name.is_empty() { &self.entry.name } else { name })
    }

    fn poll(&mut self) {
        if let Some(multi) = &mut self.multi
            && let Ok(types) = multi.types_rx.try_recv()
        {
            multi.types = Some(types);
        }
        if let Ok((info, kind)) = self.info_rx.try_recv() {
            self.saved_permissions = info.as_ref().ok().map(|i| permissions(i.mode));
            self.permissions = self.saved_permissions;
            self.info = Some(info.map_err(|e| e.to_string()));
            self.kind = kind;
        }
        let done = self.size.is_done();
        if done || self.shown_at.elapsed() >= SIZE_REFRESH {
            self.shown = self.size.totals();
            self.shown_at = Instant::now();
        }
    }

    fn is_dir(&self) -> bool {
        match &self.info {
            Some(Ok(info)) => info.is_dir,
            _ => self.entry.kind != RenameKind::File,
        }
    }

    /// Shows the dialog in its own window for this frame.
    pub fn show(&mut self, ctx: &egui::Context, icons: &mut FileIcons) -> Outcome {
        self.poll();
        if (self.is_dir() || self.multi.is_some()) && !self.size.is_done() {
            ctx.request_repaint_after(SIZE_REFRESH);
        }
        let title = self.title();
        let builder = egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size(self.size())
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_decorations(false);
        ctx.show_viewport_immediate(self.viewport, builder, |ui, _| {
            if ui.input(|i| i.viewport().close_requested()) {
                return Outcome::Close;
            }
            self.ui(ui, icons)
        })
    }

    /// Draws the whole window: frame, caption, the General tab, buttons.
    pub fn ui(&mut self, ui: &mut Ui, icons: &mut FileIcons) -> Outcome {
        self.poll();
        let title = self.title();
        let (outer, _) = ui.allocate_exact_size(self.size(), Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, &title);

        let buttons_top = inner.bottom() - 7.0 - BUTTON.y;
        let tab_top = inner.top() + 18.0 + 6.0;
        let page = Rect::from_min_max(pos2(inner.left() + 6.0, tab_top + TAB_HEIGHT), pos2(inner.right() - 6.0, buttons_top - 7.0));
        tab_page(ui, page, "General");
        self.general(ui, page, icons);

        let changed = self.editable && self.changed();
        let apply = Rect::from_min_size(pos2(inner.right() - 6.0 - BUTTON.x, buttons_top), BUTTON);
        let cancel = apply.translate(vec2(-BUTTON.x - 6.0, 0.0));
        let ok = cancel.translate(vec2(-BUTTON.x - 6.0, 0.0));
        let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if theme::push_button(ui, ok, "OK", true, true).clicked() || enter {
            return if changed { Outcome::Apply { close: true } } else { Outcome::Close };
        }
        if theme::push_button(ui, cancel, "Cancel", true, false).clicked() || escape {
            return Outcome::Close;
        }
        if theme::push_button(ui, apply, "Apply", changed, false).clicked() {
            return Outcome::Apply { close: false };
        }
        Outcome::Keep
    }

    /// The General page's contents inside `page`.
    fn general(&mut self, ui: &mut Ui, page: Rect, icons: &mut FileIcons) {
        let left = page.left() + 14.0;
        let icon = Rect::from_min_size(pos2(left, page.top() + 12.0), vec2(32.0, 32.0));
        icons.paint(ui, icon, &self.entry, IconState::Normal);
        let field = Rect::from_min_max(pos2(page.left() + VALUE_X, icon.center().y - 10.5), pos2(page.right() - 12.0, icon.center().y + 10.5));
        let text_rect = theme::sunken_field(ui.painter(), field);
        let mut edit = ui.new_child(egui::UiBuilder::new().max_rect(text_rect.shrink2(vec2(2.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        edit.add(TextEdit::singleline(&mut self.name).frame(egui::Frame::NONE).desired_width(f32::INFINITY).interactive(self.editable));

        let info = self.info.as_ref().and_then(|i| i.as_ref().ok());
        let location = self.entry.path.parent().map_or_else(String::new, |p| p.display().to_string());
        let editable = self.editable && self.permissions.is_some();
        let mut groups = self.groups(info, location);

        let mut y = icon.bottom() + 11.0;
        let font = egui::TextStyle::Body.resolve(ui.style());
        for group in groups.drain(..) {
            etched_line(ui, page, y);
            y += GROUP_TOP;
            match group {
                Group::Rows(rows) => {
                    for (label, value) in rows {
                        let row = Rect::from_min_max(pos2(left, y), pos2(page.right() - 12.0, y + ROW_HEIGHT));
                        ui.painter().text(pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), theme::color::text());
                        let value_rect = Rect::from_min_max(pos2(page.left() + VALUE_X, row.top()), row.max);
                        // Long values (a deep Location) end in "..." with the full
                        // text as a tooltip, like SetDlgItemTextWithToolTip.
                        ui.scope_builder(egui::UiBuilder::new().max_rect(value_rect).layout(Layout::left_to_right(Align::Center)), |ui| {
                            ui.add(egui::Label::new(value).truncate());
                        });
                        y += ROW_HEIGHT;
                    }
                }
                Group::Permissions(bits) => {
                    for (who, row_bits) in ["Owner:", "Group:", "Others:"].into_iter().zip(bits) {
                        let center = y + ROW_HEIGHT / 2.0;
                        ui.painter().text(pos2(left, center), egui::Align2::LEFT_CENTER, who, font.clone(), theme::color::text());
                        for (n, (on, name)) in row_bits.into_iter().zip(["Read", "Write", "Execute"]).enumerate() {
                            let check = Rect::from_min_size(pos2(page.left() + VALUE_X + n as f32 * BOX_STEP, center - 6.5), vec2(13.0, 13.0));
                            theme::check_box(ui.painter(), check, on, editable);
                            let at = pos2(check.right() + 4.0, center);
                            let galley = ui.painter().layout_no_wrap(name.to_owned(), font.clone(), theme::color::text());
                            let hit = Rect::from_min_max(check.min, pos2(at.x + galley.size().x + 2.0, check.max.y)).expand2(vec2(0.0, 4.0));
                            if editable {
                                ui.painter().galley(pos2(at.x, center - galley.size().y / 2.0), galley, theme::color::text());
                                let click = ui.interact(hit, egui::Id::new(("permission", who, n)), Sense::click());
                                if click.clicked() {
                                    // Row and column of the box in the grid.
                                    let row = ["Owner:", "Group:", "Others:"].iter().position(|w| *w == who).unwrap_or_default();
                                    if let Some(boxes) = &mut self.permissions {
                                        boxes[row][n] = !on;
                                    }
                                }
                            } else {
                                // Disabled text is embossed: a white copy one pixel down-right.
                                ui.painter().text(at + vec2(1.0, 1.0), egui::Align2::LEFT_CENTER, name, font.clone(), theme::color::highlight());
                                ui.painter().text(at, egui::Align2::LEFT_CENTER, name, font.clone(), theme::color::gray_text());
                            }
                        }
                        y += ROW_HEIGHT;
                    }
                }
            }
            y += GROUP_BOTTOM;
        }
    }

    /// The page's groups below the name, top to bottom. Their row counts
    /// must match what [`size_for`] reserves.
    fn groups(&self, info: Option<&ItemInfo>, location: String) -> Vec<Group> {
        if let Some(multi) = &self.multi {
            let ready = multi.types.is_some();
            let location = format_common_location(multi.entries.iter().filter_map(|e| e.path.parent()));
            let (size, contains) = if ready { (format_size_line(self.shown.bytes), format_contains(self.shown)) } else { Default::default() };
            return vec![Group::Rows(vec![("Type:", multi.types.clone().unwrap_or_default()), ("Location:", location), ("Size:", size), ("Contains:", contains)])];
        }
        let mut groups = if self.is_dir() {
            let ready = info.is_some();
            let (size, contains) = if ready { (format_size_line(self.shown.bytes), format_contains(self.shown)) } else { Default::default() };
            // Plain folders keep Windows' name; packages get Finder's Kind.
            let type_name = match (&self.kind, self.entry.kind) {
                (_, RenameKind::Folder) => "File Folder".to_owned(),
                (Some(kind), _) => kind.clone(),
                (None, _) => self.entry.kind_label(),
            };
            vec![
                Group::Rows(vec![("Type:", type_name), ("Location:", location), ("Size:", size), ("Contains:", contains)]),
                Group::Rows(vec![("Created:", info.map_or_else(String::new, |i| locale::long_date_time(i.created, true)))]),
            ]
        } else {
            let date = |f: fn(&ItemInfo) -> Option<std::time::SystemTime>, with_time: bool| info.map_or_else(String::new, |i| locale::long_date_time(f(i), with_time));
            vec![
                Group::Rows(vec![("Type of file:", self.kind.clone().unwrap_or_else(|| self.entry.kind_label()))]),
                Group::Rows(vec![("Location:", location), ("Size:", info.map_or_else(String::new, |i| format_size_line(i.size)))]),
                Group::Rows(vec![
                    ("Created:", date(|i| i.created, true)),
                    ("Modified:", date(|i| i.modified, true)),
                    // FAT keeps access dates without a time.
                    ("Accessed:", date(|i| i.accessed, !info.is_some_and(|i| i.on_fat))),
                ]),
            ]
        };
        if is_linked(&self.entry) {
            // The link's own path, and what it holds (a relative target as written).
            let target = info.and_then(|i| i.link.as_ref()).map_or_else(String::new, format_target);
            let at = if self.is_dir() { 1 } else { 2 };
            groups.insert(at, Group::Rows(vec![("Source:", self.entry.path.display().to_string()), ("Target:", target)]));
        }
        groups.push(Group::Permissions(self.permissions.unwrap_or_default()));
        groups
    }
}

impl Drop for Properties {
    /// Closing the dialog stops its folder walk.
    fn drop(&mut self) {
        self.size.cancel();
    }
}

/// "All of type X" when the items share a type name, else "Multiple types".
/// Finder's Kind where there is one, like the single item's page; plain
/// folders are "File Folder".
fn common_type(entries: &[Entry]) -> String {
    let mut common: Option<String> = None;
    for entry in entries {
        let name = match entry.kind {
            RenameKind::Folder => "File Folder".to_owned(),
            _ => locale::kind_name(&entry.path).unwrap_or_else(|| entry.kind_label()),
        };
        match &common {
            Some(c) if *c != name => return "Multiple types".into(),
            _ => common = Some(name),
        }
    }
    format!("All of type {}", common.unwrap_or_default())
}

/// A raised tab page with one selected tab joined to its top edge.
fn tab_page(ui: &Ui, page: Rect, label: &str) {
    let p = ui.painter();
    theme::bevel(p, page, Bevel::Raised);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), theme::color::text()).size().x);
    let tab = Rect::from_min_max(pos2(page.left(), page.top() - TAB_HEIGHT), pos2(page.left() + text_w + 16.0, page.top() + 1.0));
    let (l, t, r) = (tab.left() + 0.5, tab.top() + 0.5, tab.right() - 0.5);
    let line = |a: egui::Pos2, b: egui::Pos2, c| p.line_segment([a, b], egui::Stroke::new(1.0, c));
    // Open the page's top edge under the tab, then draw the tab's sides.
    p.rect_filled(Rect::from_min_max(pos2(tab.left() + 1.0, page.top()), pos2(tab.right() - 2.0, page.top() + 2.0)), 0.0, theme::color::face());
    p.rect_filled(Rect::from_min_max(pos2(tab.left() + 1.0, tab.top() + 1.0), pos2(tab.right() - 2.0, page.top())), 0.0, theme::color::face());
    line(pos2(l, page.top() + 1.0), pos2(l, t + 2.0), theme::color::highlight());
    line(pos2(l + 1.0, t + 1.0), pos2(l + 1.0, t + 1.0) + vec2(0.5, 0.0), theme::color::highlight());
    line(pos2(l + 2.0, t), pos2(r - 2.0, t), theme::color::highlight());
    line(pos2(r - 1.0, t + 2.0), pos2(r - 1.0, page.top() + 1.0), theme::color::shadow());
    line(pos2(r, t + 2.0), pos2(r, page.top() + 1.0), theme::color::dark());
    line(pos2(r - 1.0, t + 1.0), pos2(r - 1.0, t + 1.0) + vec2(0.5, 0.0), theme::color::dark());
    p.text(pos2(tab.center().x, tab.center().y - 1.0), egui::Align2::CENTER_CENTER, label, font, theme::color::text());
}

/// The etched groove between groups of rows.
fn etched_line(ui: &Ui, page: Rect, y: f32) {
    let x = (page.left() + 14.0)..=(page.right() - 12.0);
    let p = ui.painter();
    p.hline(x.clone(), y + 0.5, egui::Stroke::new(1.0, theme::color::shadow()));
    p.hline(x, y + 1.5, egui::Stroke::new(1.0, theme::color::highlight()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window is as tall as the groups the page draws.
    #[test]
    fn size_matches_groups() {
        let dir = std::env::temp_dir().join(format!("macsplorer-props-size-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("folder")).unwrap();
        fs::write(dir.join("file"), b"x").unwrap();
        std::os::unix::fs::symlink("file", dir.join("file link")).unwrap();
        std::os::unix::fs::symlink("folder", dir.join("folder link")).unwrap();

        let ctx = egui::Context::default();
        for name in ["file", "folder", "file link", "folder link"] {
            let path = dir.join(name);
            let entry = macsplorer_core::listing::list_dir(&dir, false).unwrap().into_iter().find(|e| e.path == path).unwrap();
            let dialog = Properties::new(&ctx, entry.clone(), true, true);
            let rows = dialog.groups(None, String::new()).iter().map(Group::rows).collect::<Vec<_>>();
            let height = BASE_HEIGHT + rows.iter().copied().map(group_height).sum::<f32>();
            assert_eq!(size_for(&entry).y, height, "{name}: {rows:?}");
            assert_eq!(rows.len(), if name.starts_with("folder") { 3 } else { 4 } + usize::from(name.contains("link")), "{name}");
        }
        // A folder that isn't in the listing finds out it's a link.
        let tree_entry = Properties::folder_entry(&dir.join("folder link"));
        assert_eq!(tree_entry.link, Some(Link::Symlink));
        assert_eq!(Properties::folder_entry(&dir.join("folder")).link, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A click on a permission box toggles it, and the dialog notices a
    /// change against what is on disk.
    #[test]
    fn clicking_a_box_toggles_it() {
        use egui_kittest::Harness;
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("macsplorer-props-click-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f");
        fs::write(&file, b"x").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let entry = macsplorer_core::listing::list_dir(&dir, false).unwrap().remove(0);

        let size = size_for(&entry);
        let mut harness = Harness::builder().with_size(size).build_ui_state(
            |ui, (dialog, icons): &mut (Option<Properties>, FileIcons)| {
                // Fonts installed now apply from the next frame.
                let Some(dialog) = dialog else {
                    crate::theme::install(ui.ctx());
                    *dialog = Some(Properties::new(ui.ctx(), entry.clone(), true, true));
                    return;
                };
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                dialog.ui(&mut window, icons);
            },
            (None, FileIcons::new()),
        );
        type Dialog = (Option<Properties>, FileIcons);
        let wait_for = |harness: &mut Harness<'_, Dialog>, done: &dyn Fn(&Properties) -> bool| {
            for _ in 0..100 {
                harness.run();
                if harness.state().0.as_ref().is_some_and(done) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!("the dialog never got there");
        };
        wait_for(&mut harness, &|d| d.permissions.is_some());
        let state = |h: &Harness<'_, Dialog>| h.state().0.as_ref().map(|d| (d.permissions.unwrap(), d.new_permissions()));
        assert_eq!(state(&harness).unwrap().0, [[true, true, false], [true, false, false], [true, false, false]]);
        assert_eq!(state(&harness).unwrap().1, None);

        // The page's top, the icon row and the groups above the permissions,
        // then the Owner row's Execute box.
        let groups_above = [1, 2, 3].map(group_height).iter().sum::<f32>();
        let y = FRAME + 24.0 + TAB_HEIGHT + 55.0 + groups_above + GROUP_TOP + ROW_HEIGHT / 2.0;
        let at = pos2(FRAME + 6.0 + VALUE_X + 2.0 * BOX_STEP + 6.0, y);
        let click = |harness: &mut Harness<'_, Dialog>| {
            // One frame per event, so the press is quick enough to count as a click.
            harness.event(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
            }
            harness.step();
            harness.run();
        };
        click(&mut harness);
        let (boxes, changed) = state(&harness).unwrap();
        assert!(boxes[0][2]);
        assert_eq!(changed, Some(boxes));
        click(&mut harness);
        assert_eq!(state(&harness).unwrap().1, None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
