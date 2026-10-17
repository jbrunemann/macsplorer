//! The keyboard shortcut list that `?` or F1 (Help > Keyboard Shortcuts)
//! opens, in its own borderless window dressed as a Windows 2000 dialog. Not
//! from Explorer: it follows the `?` overlays of terminal file managers and
//! of web mail. The list repeats the manual's chapter 13; keep them in step.

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, Ui, pos2, vec2};

use crate::scrollbar::{self, Bars};
use crate::theme::{self, Bevel};

pub const SIZE: egui::Vec2 = vec2(460.0, 440.0);
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
const ROW_HEIGHT: f32 = 18.0;
const HEADING_HEIGHT: f32 = 26.0;
const KEYS_WIDTH: f32 = 150.0;
const TITLE: &str = "Keyboard Shortcuts";

/// Keys as macOS prints them (`⌘`, `⌥`, `⇧`); other systems get Ctrl, Alt and Shift spelled out.
const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Moving around",
        &[
            ("⌘[  ⌥←  ⌫", "Back"),
            ("⌘]  ⌥→", "Forward"),
            ("⌘↑  ⌥↑", "Up to the containing folder"),
            ("Tab  F6", "Next part of the window"),
            ("⌘L  F4  ⌥D", "Type a location"),
            ("⇧⌘G", "Go To Folder (jump by name)"),
            ("/  ⌘F  F3", "Search (filter) this folder"),
            ("⌘R  F5", "Refresh"),
            ("Arrow keys", "Move the selection"),
            ("Home  End", "First or last item"),
            ("Page Up  Page Down", "A pageful up or down"),
            ("Space", "Select or deselect the focused item"),
            ("Letters", "Jump to the item starting with them"),
        ],
    ),
    (
        "Working with files",
        &[
            ("Enter", "Open"),
            ("F2", "Rename"),
            ("⌘A", "Select all"),
            ("⌘C  ⌘X  ⌘V", "Copy, cut, paste"),
            ("⌥⌘V", "Move Here"),
            ("⌥⌘C", "Copy as Path"),
            ("⌦  ⌘⌫", "Move to Trash"),
            ("⌘Z", "Undo"),
            ("⌘I", "Properties"),
        ],
    ),
    (
        "Window",
        &[
            ("⌘E  ⌘N", "New window"),
            ("⌘+  ⌘−  ⌘0", "Bigger, smaller, normal text"),
            ("?  F1", "This list"),
            ("⌘Q", "Close"),
        ],
    ),
];

/// A key label for this system: macOS keeps the symbols, the others spell them out.
fn keys_text(keys: &str) -> String {
    if cfg!(target_os = "macos") {
        return keys.to_owned();
    }
    keys.replace("⌥⌘", "Ctrl+Alt+").replace("⇧⌘", "Ctrl+Shift+").replace('⌘', "Ctrl+").replace('⌥', "Alt+").replace('⇧', "Shift+").replace('⌫', "Backspace").replace('⌦', "Delete")
}

/// What the dialog wants after a frame.
pub enum Outcome {
    Open,
    Close,
}

pub struct ShortcutsDialog {
    viewport: egui::ViewportId,
}

impl ShortcutsDialog {
    pub fn new() -> Self {
        Self { viewport: egui::ViewportId::from_hash_of("shortcuts") }
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
        ctx.show_viewport_immediate(self.viewport, builder, |ui, _| if ui.input(|i| i.viewport().close_requested()) { Outcome::Close } else { self.ui(ui) })
    }

    /// Draws the whole window: frame, caption, the list, OK.
    pub fn ui(&mut self, ui: &mut Ui) -> Outcome {
        let (outer, _) = ui.allocate_exact_size(SIZE, Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, TITLE);

        let left = inner.left() + 11.0;
        let right = inner.right() - 11.0;
        let top = inner.top() + 18.0 + 11.0;
        let ok_top = inner.bottom() - 11.0 - BUTTON.y;
        self.list(ui, Rect::from_min_max(pos2(left, top), pos2(right, ok_top - 12.0)));

        let ok = Rect::from_min_size(pos2(inner.center().x - BUTTON.x / 2.0, ok_top), BUTTON);
        let escape_or_enter = ui.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter));
        if theme::push_button(ui, ok, "OK", true, true).clicked() || escape_or_enter { Outcome::Close } else { Outcome::Open }
    }

    fn list(&self, ui: &mut Ui, rect: Rect) {
        let inner = theme::sunken_field(ui.painter(), rect);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let id = ui.id().with("shortcuts-bars");
        let content: f32 = SECTIONS.iter().map(|(_, rows)| HEADING_HEIGHT + ROW_HEIGHT * rows.len() as f32).sum();
        ui.data_mut(|d| d.insert_temp(id, egui::Vec2b::new(false, content > inner.height())));
        let (bars, viewport) = Bars::begin(ui, id, inner, [false, true], ROW_HEIGHT);
        let mut body = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down(Align::Min)));
        let out = egui::ScrollArea::vertical().auto_shrink(false).scroll_bar_visibility(scrollbar::hidden(&mut body)).show(&mut body, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (heading, rows) in SECTIONS {
                let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADING_HEIGHT), Sense::hover());
                let galley = ui.fonts_mut(|f| f.layout_no_wrap((*heading).to_owned(), theme::bold(theme::FONT_SIZE), theme::color::text()));
                ui.painter().galley(pos2(row.left() + 6.0, row.bottom() - galley.size().y - 3.0), galley, theme::color::text());
                for (keys, action) in *rows {
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
                    let keys = ui.fonts_mut(|f| f.layout_no_wrap(keys_text(keys), font.clone(), theme::color::text()));
                    ui.painter().galley(pos2(row.left() + 14.0, row.center().y - keys.size().y / 2.0), keys, theme::color::text());
                    let action = ui.fonts_mut(|f| f.layout_no_wrap((*action).to_owned(), font.clone(), theme::color::text()));
                    ui.painter().with_clip_rect(row).galley(pos2(row.left() + 14.0 + KEYS_WIDTH, row.center().y - action.size().y / 2.0), action, theme::color::text());
                }
            }
        });
        bars.end(&mut body, &out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_keys_and_an_action() {
        for (heading, rows) in SECTIONS {
            assert!(!heading.is_empty() && !rows.is_empty());
            assert!(rows.iter().all(|(k, a)| !k.is_empty() && !a.is_empty()));
        }
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn other_systems_spell_the_keys_out() {
        assert_eq!(keys_text("⌥⌘C"), "Ctrl+Alt+C");
        assert_eq!(keys_text("⇧⌘G"), "Ctrl+Shift+G");
    }
}
