//! Extract All (and Extract, for the items selected in an archive): where to put them, after `zipfldr.dll`'s
//! `CZipWiz` (`docs/archive-findings.md` §2.8), as the one page Windows 7
//! made of XP's wizard. The folder is typed (a leading `~` is the home
//! folder); "Show extracted files when complete" goes there afterwards.
//! The wording is Windows 7's, from memory. No Browse button yet.

use std::path::{Path, PathBuf};

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, TextEdit, Ui, pos2, vec2};

use macsplorer_core::home::{self, Home};

use crate::theme::{self, Bevel};

pub const SIZE: egui::Vec2 = vec2(460.0, 214.0);
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);

pub enum Outcome {
    Keep,
    Cancel,
    /// Extract into the folder typed, which the caller checks.
    Extract { folder: String, show: bool },
}

pub struct ExtractDialog {
    viewport: egui::ViewportId,
    pub archive: PathBuf,
    /// The paths inside the archive to extract; empty for all of it.
    pub items: Vec<String>,
    folder: String,
    show: bool,
    /// Why the folder typed can't be used, shown above the buttons.
    pub error: Option<String>,
}

impl ExtractDialog {
    pub fn new(archive: PathBuf, items: Vec<String>, folder: &Path, home: Option<&Home>, show: bool) -> Self {
        Self { viewport: egui::ViewportId::from_hash_of("extract"), archive, items, folder: home::abbreviate(folder, home), show, error: None }
    }

    fn title(&self) -> &'static str {
        if self.items.is_empty() { "Extract All" } else { "Extract" }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let builder = egui::ViewportBuilder::default()
            .with_title(self.title())
            .with_inner_size(SIZE)
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_decorations(false);
        ctx.show_viewport_immediate(self.viewport, builder, |ui, _| if ui.input(|i| i.viewport().close_requested()) { Outcome::Cancel } else { self.ui(ui) })
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Outcome {
        let (escape, enter) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::Escape), i.consume_key(egui::Modifiers::NONE, Key::Enter)));
        let (outer, _) = ui.allocate_exact_size(SIZE, Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, self.title());

        let font = egui::TextStyle::Body.resolve(ui.style());
        let left = inner.left() + 11.0;
        let right = inner.right() - 11.0;
        let mut y = inner.top() + 18.0 + 11.0;
        let name = self.archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let heading = ui.fonts_mut(|f| f.layout(format!("Select a Destination and Extract Files from \"{name}\""), theme::bold(theme::FONT_SIZE), theme::color::text(), right - left));
        let height = heading.size().y;
        ui.painter().galley(pos2(left, y), heading, theme::color::text());
        y += height + 12.0;
        let label = ui.fonts_mut(|f| f.layout_no_wrap("Files will be extracted to this folder:".to_owned(), font.clone(), theme::color::text()));
        let height = label.size().y;
        ui.painter().galley(pos2(left, y), label, theme::color::text());
        y += height + 4.0;

        let field = Rect::from_min_size(pos2(left, y), vec2(right - left, 22.0));
        let field_inner = theme::sunken_field(ui.painter(), field);
        let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(field_inner.shrink2(vec2(2.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        let response = field_ui.add(TextEdit::singleline(&mut self.folder).id(ui.id().with("extract-folder")).frame(egui::Frame::NONE).desired_width(f32::INFINITY));
        if !response.has_focus() && ui.memory(|m| m.focused().is_none()) {
            response.request_focus();
        }
        if response.changed() {
            self.error = None;
        }
        y = field.bottom() + 12.0;

        let check_label = ui.fonts_mut(|f| f.layout_no_wrap("Show extracted files when complete".to_owned(), font.clone(), theme::color::text()));
        let row = Rect::from_min_size(pos2(left, y), vec2(13.0 + 6.0 + check_label.size().x, 15.0));
        if ui.interact(row, ui.id().with("extract-show"), Sense::click()).clicked() {
            self.show = !self.show;
        }
        theme::check_box(ui.painter(), Rect::from_min_size(pos2(left, y + 1.0), vec2(13.0, 13.0)), self.show, true);
        ui.painter().galley(pos2(left + 19.0, y + 7.5 - check_label.size().y / 2.0), check_label, theme::color::text());

        let top = inner.bottom() - 11.0 - BUTTON.y;
        if let Some(error) = &self.error {
            let galley = ui.fonts_mut(|f| f.layout(error.clone(), font.clone(), theme::color::text(), right - left - 2.0 * BUTTON.x - 12.0));
            ui.painter().galley(pos2(left, top), galley, theme::color::text());
        }
        let cancel = Rect::from_min_size(pos2(right - BUTTON.x, top), BUTTON);
        let extract = cancel.translate(vec2(-BUTTON.x - 6.0, 0.0));
        let typed = !self.folder.trim().is_empty();
        if (theme::push_button(ui, extract, "Extract", typed, true).clicked() || enter) && typed {
            return Outcome::Extract { folder: self.folder.trim().to_owned(), show: self.show };
        }
        if theme::push_button(ui, cancel, "Cancel", true, false).clicked() || escape {
            return Outcome::Cancel;
        }
        Outcome::Keep
    }
}
