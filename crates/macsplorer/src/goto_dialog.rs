//! Go To Folder: type a few letters of a folder's name and jump there, in
//! its own borderless window dressed as a Windows 2000 dialog. Not from
//! Explorer, which has nothing like it: the box and its keys follow the
//! quick openers of the terminal file managers (the matching is
//! `macsplorer_core::goto`). Up and Down (or Ctrl+P and Ctrl+N) move through
//! the list, Enter goes, Esc closes.

use std::path::{Path, PathBuf};

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, TextEdit, Ui, pos2, vec2};

use macsplorer_core::goto::{self, Candidate};
use macsplorer_core::home::{self, Home};

use crate::scrollbar::{self, Bars};
use crate::theme::{self, Bevel};

pub const SIZE: egui::Vec2 = vec2(420.0, 372.0);
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
const ROW_HEIGHT: f32 = 18.0;
const TITLE: &str = "Go To Folder";
/// How many rows Page Up and Page Down move.
const PAGE: isize = 8;

/// What the dialog asks the window to do after this frame.
pub enum Outcome {
    Keep,
    Cancel,
    Go(PathBuf),
}

pub struct GotoDialog {
    viewport: egui::ViewportId,
    candidates: Vec<Candidate>,
    home: Option<Home>,
    query: String,
    /// Positions in `candidates` that match `query`, best first.
    matches: Vec<usize>,
    selected: usize,
    /// Scroll the selected row into view on the next frame.
    reveal: bool,
}

impl GotoDialog {
    pub fn new(candidates: Vec<Candidate>, home: Option<Home>) -> Self {
        let candidates = goto::dedupe(candidates);
        let matches = goto::rank("", &candidates);
        Self { viewport: egui::ViewportId::from_hash_of("goto"), candidates, home, query: String::new(), matches, selected: 0, reveal: false }
    }

    fn move_selection(&mut self, delta: isize) {
        let before = self.selected;
        self.selected = goto::step(self.selected, delta, self.matches.len());
        self.reveal |= self.selected != before;
    }

    #[cfg(test)]
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_owned();
        self.matches = goto::rank(query, &self.candidates);
    }

    #[cfg(test)]
    pub fn has_folder(&self, path: &Path) -> bool {
        self.candidates.iter().any(|c| c.path == path)
    }

    fn chosen(&self) -> Option<&Candidate> {
        self.matches.get(self.selected).map(|&i| &self.candidates[i])
    }

    /// Where a path is shown: the home folder as `~`.
    fn shown(&self, path: &Path) -> String {
        home::abbreviate(path, self.home.as_ref())
    }

    /// Shows the dialog in its own window for this frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let builder = egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size(SIZE)
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_decorations(false);
        ctx.show_viewport_immediate(self.viewport, builder, |ui, _| if ui.input(|i| i.viewport().close_requested()) { Outcome::Cancel } else { self.ui(ui) })
    }

    /// Draws the whole window: frame, caption, the text box, the list, buttons.
    pub fn ui(&mut self, ui: &mut Ui) -> Outcome {
        // The keys that move through the list, taken before the text box sees them.
        let (up, down, page_up, page_down, escape, enter) = ui.input_mut(|i| {
            let none = egui::Modifiers::NONE;
            let ctrl = egui::Modifiers::CTRL;
            (
                i.consume_key(none, Key::ArrowUp) | i.consume_key(ctrl, Key::P),
                i.consume_key(none, Key::ArrowDown) | i.consume_key(ctrl, Key::N),
                i.consume_key(none, Key::PageUp),
                i.consume_key(none, Key::PageDown),
                i.consume_key(none, Key::Escape),
                i.consume_key(none, Key::Enter),
            )
        });
        for (pressed, delta) in [(up, -1), (down, 1), (page_up, -PAGE), (page_down, PAGE)] {
            if pressed {
                self.move_selection(delta);
            }
        }

        let (outer, _) = ui.allocate_exact_size(SIZE, Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, TITLE);

        let font = egui::TextStyle::Body.resolve(ui.style());
        let left = inner.left() + 11.0;
        let right = inner.right() - 11.0;
        let mut y = inner.top() + 18.0 + 11.0;
        let intro = ui.fonts_mut(|f| f.layout("Type part of a folder's name:".to_owned(), font.clone(), theme::color::text(), right - left));
        let intro_height = intro.size().y;
        ui.painter().galley(pos2(left, y), intro, theme::color::text());
        y += intro_height + 6.0;

        let field = Rect::from_min_size(pos2(left, y), vec2(right - left, 22.0));
        let field_inner = theme::sunken_field(ui.painter(), field);
        let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(field_inner.shrink2(vec2(2.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        let id = ui.id().with("goto-text");
        let before = self.query.clone();
        let response = field_ui.add(TextEdit::singleline(&mut self.query).id(id).frame(egui::Frame::NONE).desired_width(f32::INFINITY));
        response.request_focus();
        if self.query != before {
            self.matches = goto::rank(&self.query, &self.candidates);
            self.selected = 0;
            self.reveal = true;
        }
        y = field.bottom() + 10.0;

        let ok_top = inner.bottom() - 11.0 - BUTTON.y;
        let list = Rect::from_min_max(pos2(left, y), pos2(right, ok_top - 12.0));
        let double_clicked = self.list(ui, list);

        let cancel = Rect::from_min_size(pos2(right - BUTTON.x, ok_top), BUTTON);
        let go = cancel.translate(vec2(-BUTTON.x - 6.0, 0.0));
        let target = self.chosen().map(|c| c.path.clone());
        let confirmed = theme::push_button(ui, go, "Go", target.is_some(), true).clicked() || enter || double_clicked;
        if let (true, Some(path)) = (confirmed, target) {
            return Outcome::Go(path);
        }
        if theme::push_button(ui, cancel, "Cancel", true, false).clicked() || escape {
            return Outcome::Cancel;
        }
        Outcome::Keep
    }

    /// The matches: the folder's name, and where it is in gray. Returns true
    /// on a double-click, which is Go.
    fn list(&mut self, ui: &mut Ui, rect: Rect) -> bool {
        let inner = theme::sunken_field(ui.painter(), rect);
        let reveal = std::mem::take(&mut self.reveal);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let id = ui.id().with("goto-bars");
        let content = ROW_HEIGHT * self.matches.len() as f32;
        ui.data_mut(|d| d.insert_temp(id, egui::Vec2b::new(false, content > inner.height())));
        let (bars, viewport) = Bars::begin(ui, id, inner, [false, true], ROW_HEIGHT);
        let mut body = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down(Align::Min)));
        if self.matches.is_empty() {
            let galley = ui.fonts_mut(|f| f.layout_no_wrap("No folder matches.".to_owned(), font.clone(), theme::color::gray_text()));
            ui.painter().galley(pos2(viewport.left() + 6.0, viewport.top() + 4.0), galley, theme::color::gray_text());
        }
        let mut double_clicked = false;
        let mut clicked = None;
        let selected = self.selected;
        let out = egui::ScrollArea::vertical().auto_shrink(false).scroll_bar_visibility(scrollbar::hidden(&mut body)).show(&mut body, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (row_index, &i) in self.matches.iter().enumerate() {
                let candidate = &self.candidates[i];
                let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
                let on = row_index == selected;
                if on {
                    ui.painter().rect_filled(row, 0.0, theme::color::selection());
                    theme::focus_rect(ui.painter(), row);
                }
                let (name_color, path_color) = if on { (theme::color::selection_text(), theme::color::selection_text()) } else { (theme::color::text(), theme::color::gray_text()) };
                let name = ui.fonts_mut(|f| f.layout_no_wrap(candidate.label.clone(), font.clone(), name_color));
                let name_width = name.size().x;
                ui.painter().galley_with_override_text_color(pos2(row.left() + 4.0, row.center().y - name.size().y / 2.0), name, name_color);
                let parent = candidate.path.parent().map(|p| self.shown(p)).unwrap_or_default();
                let path = ui.fonts_mut(|f| f.layout(parent, font.clone(), path_color, (row.width() - name_width - 24.0).max(0.0)));
                ui.painter().with_clip_rect(row).galley_with_override_text_color(pos2(row.left() + 4.0 + name_width + 10.0, row.center().y - path.size().y / 2.0), path, path_color);
                if on && reveal {
                    ui.scroll_to_rect(row, None);
                }
                if resp.clicked() || resp.double_clicked() {
                    clicked = Some(row_index);
                }
                double_clicked |= resp.double_clicked();
            }
        });
        bars.end(&mut body, &out);
        if let Some(row) = clicked {
            self.selected = row;
        }
        double_clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use macsplorer_core::goto::Source;

    fn dialog() -> GotoDialog {
        GotoDialog::new(
            vec![
                Candidate::named(Path::new("/Users/me/Documents"), Source::Recent),
                Candidate::named(Path::new("/Users/me/Downloads"), Source::Place),
                Candidate::named(Path::new("/Users/me/Documents"), Source::Open),
            ],
            Some(Home::new(PathBuf::from("/Users/me"))),
        )
    }

    #[test]
    fn a_folder_is_offered_once() {
        assert_eq!(dialog().matches.len(), 2);
    }

    #[test]
    fn the_first_match_starts_selected_and_the_selection_stops_at_the_ends() {
        let mut d = dialog();
        assert_eq!(d.chosen().map(|c| c.label.as_str()), Some("Documents"));
        d.move_selection(-1);
        assert_eq!(d.selected, 0);
        d.move_selection(5);
        assert_eq!(d.chosen().map(|c| c.label.as_str()), Some("Downloads"));
    }

    #[test]
    fn paths_show_the_home_folder_as_a_tilde() {
        let d = dialog();
        assert_eq!(d.shown(Path::new("/Users/me")), "~");
        assert_eq!(d.shown(Path::new("/Users/me/Work")), "~/Work");
        assert_eq!(d.shown(Path::new("/Volumes/X")), "/Volumes/X");
    }
}
