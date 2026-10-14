//! The Properties dialog: the General page of shell32's `CFilePropSheetPage`
//! (`_InitSingleFilePrsht`, `_UpdateSizeField`, `_UpdateSizeCount`), in its
//! own borderless window. Left out on purpose: Sharing and Security tabs,
//! Size on disk, Opens with, Attributes and Advanced.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, TextEdit, Ui, pos2, vec2};

use macsplorer_core::listing::{Entry, is_package_name};
use macsplorer_core::properties::{FolderSize, FolderTotals, ItemInfo, format_contains, format_size_line, item_info};
use macsplorer_core::rename::RenameKind;

use crate::icons::{FileIcons, IconState};
use crate::locale;
use crate::theme::{self, Bevel};

/// The window, fixed size like a property sheet. Tall enough for a file's
/// six rows; a folder's page has room to spare, as in Windows.
pub const SIZE: egui::Vec2 = vec2(380.0, 332.0);
const FRAME: f32 = 4.0;
const TAB_HEIGHT: f32 = 20.0;
const ROW_HEIGHT: f32 = 22.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
/// Where values start, right of the labels and lined up with the name box.
const VALUE_X: f32 = 86.0;
/// How often a running folder walk's totals are shown, like the 250 ms
/// timer `CFolderContentsInfo::SizeThreadProc` sets.
const SIZE_REFRESH: Duration = Duration::from_millis(250);

/// What the dialog asks the window to do after this frame.
pub enum Outcome {
    Keep,
    Close,
    /// Rename to [`Properties::new_name`]; OK also closes on success.
    Rename { close: bool },
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
    size: Arc<FolderSize>,
    /// The folder totals on screen and when they were taken.
    shown: FolderTotals,
    shown_at: Instant,
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
            size,
            shown: FolderTotals::default(),
            shown_at: Instant::now(),
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
        }
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

    fn changed(&self) -> bool {
        self.new_name() != self.entry.name
    }

    /// The rename went through: show the new name from now on.
    pub fn renamed(&mut self, to: PathBuf) {
        self.entry.name = to.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.entry.path = to;
    }

    fn title(&self) -> String {
        let name = self.name.trim();
        format!("{} Properties", if name.is_empty() { &self.entry.name } else { name })
    }

    fn poll(&mut self) {
        if let Ok((info, kind)) = self.info_rx.try_recv() {
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
        if self.is_dir() && !self.size.is_done() {
            ctx.request_repaint_after(SIZE_REFRESH);
        }
        let title = self.title();
        let builder = egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size(SIZE)
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
        let (outer, _) = ui.allocate_exact_size(SIZE, Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::FACE);
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
            return if changed { Outcome::Rename { close: true } } else { Outcome::Close };
        }
        if theme::push_button(ui, cancel, "Cancel", true, false).clicked() || escape {
            return Outcome::Close;
        }
        if theme::push_button(ui, apply, "Apply", changed, false).clicked() {
            return Outcome::Rename { close: false };
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
        let groups: Vec<Vec<(&str, String)>> = if self.is_dir() {
            let ready = info.is_some();
            let (size, contains) = if ready { (format_size_line(self.shown.bytes), format_contains(self.shown)) } else { Default::default() };
            // Plain folders keep Windows' name; packages get Finder's Kind.
            let type_name = match (&self.kind, self.entry.kind) {
                (_, RenameKind::Folder) => "File Folder".to_owned(),
                (Some(kind), _) => kind.clone(),
                (None, _) => self.entry.kind_label(),
            };
            vec![
                vec![("Type:", type_name), ("Location:", location), ("Size:", size), ("Contains:", contains)],
                vec![("Created:", info.map_or_else(String::new, |i| locale::long_date_time(i.created, true)))],
            ]
        } else {
            let date = |f: fn(&ItemInfo) -> Option<std::time::SystemTime>, with_time: bool| info.map_or_else(String::new, |i| locale::long_date_time(f(i), with_time));
            vec![
                vec![("Type of file:", self.kind.clone().unwrap_or_else(|| self.entry.kind_label()))],
                vec![("Location:", location), ("Size:", info.map_or_else(String::new, |i| format_size_line(i.size)))],
                vec![
                    ("Created:", date(|i| i.created, true)),
                    ("Modified:", date(|i| i.modified, true)),
                    // FAT keeps access dates without a time.
                    ("Accessed:", date(|i| i.accessed, !info.is_some_and(|i| i.on_fat))),
                ],
            ]
        };

        let mut y = icon.bottom() + 11.0;
        for group in groups {
            etched_line(ui, page, y);
            y += 9.0;
            for (label, value) in group {
                let row = Rect::from_min_max(pos2(left, y), pos2(page.right() - 12.0, y + ROW_HEIGHT));
                let font = egui::TextStyle::Body.resolve(ui.style());
                ui.painter().text(pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, label, font, theme::color::TEXT);
                let value_rect = Rect::from_min_max(pos2(page.left() + VALUE_X, row.top()), row.max);
                // Long values (a deep Location) end in "..." with the full
                // text as a tooltip, like SetDlgItemTextWithToolTip.
                ui.scope_builder(egui::UiBuilder::new().max_rect(value_rect).layout(Layout::left_to_right(Align::Center)), |ui| {
                    ui.add(egui::Label::new(value).truncate());
                });
                y += ROW_HEIGHT;
            }
            y += 6.0;
        }
    }
}

impl Drop for Properties {
    /// Closing the dialog stops its folder walk.
    fn drop(&mut self) {
        self.size.cancel();
    }
}

/// A raised tab page with one selected tab joined to its top edge.
fn tab_page(ui: &Ui, page: Rect, label: &str) {
    let p = ui.painter();
    theme::bevel(p, page, Bevel::Raised);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), theme::color::TEXT).size().x);
    let tab = Rect::from_min_max(pos2(page.left(), page.top() - TAB_HEIGHT), pos2(page.left() + text_w + 16.0, page.top() + 1.0));
    let (l, t, r) = (tab.left() + 0.5, tab.top() + 0.5, tab.right() - 0.5);
    let line = |a: egui::Pos2, b: egui::Pos2, c| p.line_segment([a, b], egui::Stroke::new(1.0, c));
    // Open the page's top edge under the tab, then draw the tab's sides.
    p.rect_filled(Rect::from_min_max(pos2(tab.left() + 1.0, page.top()), pos2(tab.right() - 2.0, page.top() + 2.0)), 0.0, theme::color::FACE);
    p.rect_filled(Rect::from_min_max(pos2(tab.left() + 1.0, tab.top() + 1.0), pos2(tab.right() - 2.0, page.top())), 0.0, theme::color::FACE);
    line(pos2(l, page.top() + 1.0), pos2(l, t + 2.0), theme::color::HIGHLIGHT);
    line(pos2(l + 1.0, t + 1.0), pos2(l + 1.0, t + 1.0) + vec2(0.5, 0.0), theme::color::HIGHLIGHT);
    line(pos2(l + 2.0, t), pos2(r - 2.0, t), theme::color::HIGHLIGHT);
    line(pos2(r - 1.0, t + 2.0), pos2(r - 1.0, page.top() + 1.0), theme::color::SHADOW);
    line(pos2(r, t + 2.0), pos2(r, page.top() + 1.0), theme::color::DARK);
    line(pos2(r - 1.0, t + 1.0), pos2(r - 1.0, t + 1.0) + vec2(0.5, 0.0), theme::color::DARK);
    p.text(pos2(tab.center().x, tab.center().y - 1.0), egui::Align2::CENTER_CENTER, label, font, theme::color::TEXT);
}

/// The etched groove between groups of rows.
fn etched_line(ui: &Ui, page: Rect, y: f32) {
    let x = (page.left() + 14.0)..=(page.right() - 12.0);
    let p = ui.painter();
    p.hline(x.clone(), y + 0.5, egui::Stroke::new(1.0, theme::color::SHADOW));
    p.hline(x, y + 1.5, egui::Stroke::new(1.0, theme::color::HIGHLIGHT));
}
