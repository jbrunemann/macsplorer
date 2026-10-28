//! A device's Properties: what Network Neighborhood knows of a LocalSend or
//! AirLynx device (address, id, when it was last seen), in its own borderless
//! window laid out like the General page of the file Properties dialog
//! (`properties.rs`). Not from Windows, which had no such page for a network
//! neighbor; read only, so there is one OK button.

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, Ui, pos2, vec2};

use crate::properties::{BASE_HEIGHT, BUTTON, FRAME, GROUP_BOTTOM, GROUP_TOP, ROW_HEIGHT, TAB_HEIGHT, VALUE_X, WIDTH, etched_line, group_height, tab_page};
use crate::theme::{self, Bevel};

/// What the dialog shows of a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub name: String,
    /// Label and value rows, in groups set apart by etched lines.
    pub groups: Vec<Vec<(&'static str, String)>>,
}

impl View {
    pub fn title(&self) -> String {
        format!("{} Properties", self.name)
    }

    pub fn size(&self) -> egui::Vec2 {
        vec2(WIDTH, BASE_HEIGHT + self.groups.iter().map(|g| group_height(g.len())).sum::<f32>())
    }
}

/// Shows the dialog in its own window for this frame; `true` once it should
/// close. `logo` is the device kind's, drawn where a file has its icon.
pub fn show(ctx: &egui::Context, id: &str, view: &View, logo: egui::TextureId) -> bool {
    let builder = egui::ViewportBuilder::default()
        .with_title(view.title())
        .with_inner_size(view.size())
        .with_resizable(false)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_decorations(false);
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("device-properties", id)), builder, |ui, _| {
        ui.input(|i| i.viewport().close_requested()) || draw(ui, view, logo)
    })
}

/// Draws the whole window: frame, caption, the General tab, OK. `true` when
/// OK, Enter or Escape closed it.
pub fn draw(ui: &mut Ui, view: &View, logo: egui::TextureId) -> bool {
    let (outer, _) = ui.allocate_exact_size(view.size(), Sense::hover());
    let p = ui.painter();
    p.rect_filled(outer, 0.0, theme::color::face());
    theme::bevel(p, outer, Bevel::Raised);
    let inner = outer.shrink(FRAME);
    let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    theme::dialog_title_bar(&mut caption, &view.title());

    let buttons_top = inner.bottom() - 7.0 - BUTTON.y;
    let tab_top = inner.top() + 18.0 + 6.0;
    let page = Rect::from_min_max(pos2(inner.left() + 6.0, tab_top + TAB_HEIGHT), pos2(inner.right() - 6.0, buttons_top - 7.0));
    tab_page(ui, page, &["General"], 0);

    let left = page.left() + 14.0;
    let icon = Rect::from_min_size(pos2(left, page.top() + 12.0), vec2(32.0, 32.0));
    ui.painter().image(logo, icon, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let name = pos2(page.left() + VALUE_X, icon.center().y);
    ui.painter().text(name, egui::Align2::LEFT_CENTER, &view.name, font.clone(), theme::color::text());

    let mut y = icon.bottom() + 11.0;
    for group in &view.groups {
        etched_line(ui, page, y);
        y += GROUP_TOP;
        for (label, value) in group {
            let row = Rect::from_min_max(pos2(left, y), pos2(page.right() - 12.0, y + ROW_HEIGHT));
            ui.painter().text(pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), theme::color::text());
            let value_rect = Rect::from_min_max(pos2(page.left() + VALUE_X, row.top()), row.max);
            // A long value (a fingerprint) ends in "..." with the whole text as a tooltip.
            ui.scope_builder(egui::UiBuilder::new().max_rect(value_rect).layout(Layout::left_to_right(Align::Center)), |ui| {
                ui.add(egui::Label::new(value).truncate());
            });
            y += ROW_HEIGHT;
        }
        y += GROUP_BOTTOM;
    }

    let ok = Rect::from_min_size(pos2(inner.right() - 6.0 - BUTTON.x, buttons_top), BUTTON);
    let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    theme::push_button(ui, ok, "OK", true, true).clicked() || enter || escape
}
