//! The question a paste asks when the destination already has an item of
//! that name: Windows 2000's "Confirm File Replace" and "Confirm Folder
//! Replace", in their own borderless window. Which question comes when is
//! traced from build 10240 (`macsplorer_core::fileops`); the layout and
//! wording are Windows 2000's from memory (the strings are in shell32's MUI
//! file, which isn't analyzed), except that the folder question says what
//! macsplorer does with files of the same name: it asks about each one.

use std::sync::mpsc::Sender;

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, Ui, pos2, vec2};

use macsplorer_core::fileops::{Choice, Conflict, ConflictKind};
use macsplorer_core::listing::{Entry, format_byte_size};
use macsplorer_core::rename::RenameKind;

use crate::icons::{FileIcons, IconState};
use crate::locale;
use crate::theme::{self, Bevel};

const WIDTH: f32 = 430.0;
const FILE_HEIGHT: f32 = 246.0;
const FOLDER_HEIGHT: f32 = 172.0;
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
/// Where the text starts, right of the dialog's icon.
const TEXT_X: f32 = 58.0;

/// A conflict from a paste worker, which waits for the answer on `reply`.
/// The entries are read on the worker, so the dialog does no disk I/O.
pub struct Ask {
    pub conflict: Conflict,
    /// The destination folder, for the dialog's icon.
    pub folder: Option<Entry>,
    pub existing: Option<Entry>,
    pub source: Option<Entry>,
    pub reply: Sender<Choice>,
}

pub struct ConflictDialog {
    viewport: egui::ViewportId,
    ask: Ask,
}

impl ConflictDialog {
    pub fn new(ask: Ask) -> Self {
        Self { viewport: egui::ViewportId::from_hash_of(("confirm-replace", &ask.conflict.existing)), ask }
    }

    pub fn size(&self) -> egui::Vec2 {
        vec2(WIDTH, if self.ask.conflict.kind == ConflictKind::Folder { FOLDER_HEIGHT } else { FILE_HEIGHT })
    }

    fn title(&self) -> &'static str {
        if self.ask.conflict.kind == ConflictKind::Folder { "Confirm Folder Replace" } else { "Confirm File Replace" }
    }

    /// Sends the answer to the waiting worker.
    pub fn answer(self, choice: Choice) {
        let _ = self.ask.reply.send(choice);
    }

    /// Shows the dialog in its own window for this frame; `Some` once it
    /// was answered.
    pub fn show(&mut self, ctx: &egui::Context, icons: &mut FileIcons) -> Option<Choice> {
        let builder = egui::ViewportBuilder::default()
            .with_title(self.title())
            .with_inner_size(self.size())
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_decorations(false);
        ctx.show_viewport_immediate(self.viewport, builder, |ui, _| {
            if ui.input(|i| i.viewport().close_requested()) {
                return Some(Choice::Cancel);
            }
            self.ui(ui, icons)
        })
    }

    /// Draws the whole window: frame, caption, the question, buttons.
    pub fn ui(&mut self, ui: &mut Ui, icons: &mut FileIcons) -> Option<Choice> {
        let title = self.title();
        let (outer, _) = ui.allocate_exact_size(self.size(), Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, title);

        let top = inner.top() + 18.0 + 12.0;
        if let Some(folder) = &self.ask.folder {
            icons.paint(ui, Rect::from_min_size(pos2(inner.left() + 14.0, top), vec2(32.0, 32.0)), folder, IconState::Normal);
        }
        let left = inner.left() + TEXT_X;
        let right = inner.right() - 14.0;
        let mut y = top + 2.0;
        let conflict = &self.ask.conflict;
        let name = conflict.existing.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let existing_noun = noun(self.ask.existing.as_ref());
        y = paragraph(ui, left, right, y, &format!("This folder already contains a {existing_noun} named '{name}'."));
        y += 12.0;
        match conflict.kind {
            ConflictKind::Folder => {
                let verb = if conflict.moving { "move" } else { "copy" };
                let text = format!(
                    "If files in the existing folder have the same names as files in the folder you are {verb}ing, you will be asked whether to replace them. Do you still want to {verb} the folder?"
                );
                paragraph(ui, left, right, y, &text);
            }
            ConflictKind::Item => {
                y = paragraph(ui, left, right, y, &format!("Would you like to replace the existing {existing_noun}"));
                y = item(ui, icons, left + 16.0, y + 8.0, self.ask.existing.as_ref());
                y = paragraph(ui, left, right, y + 8.0, "with this one?");
                item(ui, icons, left + 16.0, y + 8.0, self.ask.source.as_ref());
            }
        }

        // Yes, Yes to All, No and Cancel, centered along the bottom.
        let gap = 6.0;
        let row = 4.0 * BUTTON.x + 3.0 * gap;
        let bottom = inner.bottom() - 11.0 - BUTTON.y;
        let mut x = inner.center().x - row / 2.0;
        let (enter, escape, y_key, a_key, n_key) =
            ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape), i.key_pressed(Key::Y), i.key_pressed(Key::A), i.key_pressed(Key::N)));
        let mut answer = None;
        for (label, choice, default, key) in [
            ("Yes", Choice::Yes, true, enter || y_key),
            ("Yes to All", Choice::YesToAll, false, a_key),
            ("No", Choice::No, false, n_key),
            ("Cancel", Choice::Cancel, false, escape),
        ] {
            let rect = Rect::from_min_size(pos2(x, bottom), BUTTON);
            x += BUTTON.x + gap;
            if (theme::push_button(ui, rect, label, true, default).clicked() || key) && answer.is_none() {
                answer = Some(choice);
            }
        }
        answer
    }
}

/// "file" or "folder", as the question names the item.
fn noun(entry: Option<&Entry>) -> &'static str {
    match entry.map(|e| e.kind) {
        Some(RenameKind::Folder) => "folder",
        _ => "file",
    }
}

/// Text wrapped between `left` and `right` from `y`; returns its bottom.
fn paragraph(ui: &Ui, left: f32, right: f32, y: f32, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.fonts_mut(|f| f.layout(text.to_owned(), font, theme::color::text(), right - left));
    let height = galley.size().y;
    ui.painter().galley(pos2(left, y), galley, theme::color::text());
    y + height
}

/// One of the two items: its icon, its size (files only) and when it was
/// modified. Returns the bottom.
fn item(ui: &Ui, icons: &mut FileIcons, left: f32, y: f32, entry: Option<&Entry>) -> f32 {
    let icon = Rect::from_min_size(pos2(left, y), vec2(32.0, 32.0));
    let Some(entry) = entry else { return icon.bottom() };
    icons.paint(ui, icon, entry, IconState::Normal);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_x = icon.right() + 12.0;
    let mut lines = Vec::new();
    if let Some(size) = entry.size {
        lines.push(format_byte_size(size));
    }
    lines.push(modified_line(entry));
    let line_height = 15.0;
    let mut line_y = icon.center().y - line_height * lines.len() as f32 / 2.0;
    for line in lines {
        ui.painter().text(pos2(text_x, line_y), egui::Align2::LEFT_TOP, line, font.clone(), theme::color::text());
        line_y += line_height;
    }
    icon.bottom()
}

/// "modified on Tuesday, March 14, 2000, 10:31 AM", or "modified today, ...".
fn modified_line(entry: &Entry) -> String {
    let date = locale::long_date_time(entry.modified, true);
    if date.is_empty() {
        return String::new();
    }
    match ["Today", "Yesterday"].iter().find(|w| date.starts_with(**w)) {
        Some(word) => format!("modified {}{}", word.to_lowercase(), &date[word.len()..]),
        None => format!("modified on {date}"),
    }
}
