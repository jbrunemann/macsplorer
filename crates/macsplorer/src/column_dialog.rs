//! The header menu's "More..." dialog, after shell32's `CColumnDlg`
//! (`_InitListView`, `DlgProc`, `_UpdateDlgButtons`, `_MoveItem`,
//! `_SaveState`), in its own borderless window. Win2000 called it "Column
//! Settings". The wording is from memory of Windows XP's "Choose Details":
//! the strings live in shell32's MUI file, which isn't analyzed.

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, TextEdit, Ui, pos2, vec2};

use macsplorer_core::columns::{Column, ColumnSet};

use crate::theme::{self, Bevel};

pub const SIZE: egui::Vec2 = vec2(352.0, 372.0);
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
const ROW_HEIGHT: f32 = 17.0;
const TITLE: &str = "Column Settings";

/// What the dialog asks the window to do after this frame.
pub enum Outcome {
    Keep,
    Cancel,
    /// OK: apply with [`ColumnDialog::apply`], then close.
    Apply,
}

pub struct ColumnDialog {
    viewport: egui::ViewportId,
    /// The list: each column and its check box, top to bottom.
    rows: Vec<(Column, bool)>,
    /// Every column's width, and whether the width box changed it.
    widths: Vec<(Column, f32, bool)>,
    selected: usize,
    /// The width box's text, for the selected column.
    width_text: String,
}

impl ColumnDialog {
    pub fn new(columns: &ColumnSet) -> Self {
        let rows = columns.dialog_rows();
        let widths = Column::ALL.iter().map(|&c| (c, columns.width(c), false)).collect();
        let mut dialog = Self { viewport: egui::ViewportId::from_hash_of("column-settings"), rows, widths, selected: 0, width_text: String::new() };
        dialog.select(0);
        dialog
    }

    /// Carries out OK (`_SaveState`): the checked columns in list order,
    /// and the widths typed in the width box.
    pub fn apply(&self, columns: &mut ColumnSet) {
        let shown: Vec<Column> = self.rows.iter().filter(|r| r.1).map(|r| r.0).collect();
        columns.set_visible(&shown);
        for &(column, width, edited) in &self.widths {
            if edited {
                columns.set_width(column, width);
            }
        }
    }

    fn width_of(&self, column: Column) -> f32 {
        self.widths.iter().find(|w| w.0 == column).map_or(0.0, |w| w.1)
    }

    fn select(&mut self, index: usize) {
        self.selected = index.min(self.rows.len().saturating_sub(1));
        let width = self.width_of(self.rows[self.selected].0);
        self.width_text = format!("{}", width.round() as u32);
    }

    /// Checks or unchecks a row. Name can't be unchecked: the dialog
    /// refuses the change (and beeps, which macsplorer doesn't).
    fn set_checked(&mut self, index: usize, checked: bool) {
        let (column, on) = &mut self.rows[index];
        if !checked && column.always_visible() {
            return;
        }
        *on = checked;
    }

    /// Move Up (-1) and Move Down (+1) swap the selected row with its
    /// neighbor; the selection moves along (`_MoveItem`).
    fn move_selected(&mut self, by: isize) {
        let Some(to) = self.selected.checked_add_signed(by).filter(|&to| to < self.rows.len()) else { return };
        self.rows.swap(self.selected, to);
        self.selected = to;
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
        ctx.show_viewport_immediate(self.viewport, builder, |ui, _| {
            if ui.input(|i| i.viewport().close_requested()) {
                return Outcome::Cancel;
            }
            self.ui(ui)
        })
    }

    /// Draws the whole window: frame, caption, list, buttons.
    pub fn ui(&mut self, ui: &mut Ui) -> Outcome {
        let (outer, _) = ui.allocate_exact_size(SIZE, Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::FACE);
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, TITLE);

        let font = egui::TextStyle::Body.resolve(ui.style());
        let left = inner.left() + 11.0;
        let right = inner.right() - 11.0;
        let mut y = inner.top() + 18.0 + 11.0;
        let intro = ui.fonts_mut(|f| {
            f.layout(
                "Select the details you want to display for the files in this folder.".to_owned(),
                font.clone(),
                theme::color::TEXT,
                right - left,
            )
        });
        let intro_height = intro.size().y;
        ui.painter().galley(pos2(left, y), intro, theme::color::TEXT);
        y += intro_height + 10.0;
        ui.painter().text(pos2(left, y), egui::Align2::LEFT_TOP, "Details:", font.clone(), theme::color::TEXT);
        y += 17.0;

        let buttons_x = right - BUTTON.x;
        let ok_top = inner.bottom() - 11.0 - BUTTON.y;
        let width_row = ok_top - 14.0 - 21.0;
        let list = Rect::from_min_max(pos2(left, y), pos2(buttons_x - 9.0, width_row - 10.0));
        let (escape, enter) = ui.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
        self.list(ui, list);

        let (column, checked) = self.rows[self.selected];
        let fixed = column.always_visible();
        let mut button_y = y;
        let mut button = |ui: &mut Ui, label: &str, enabled: bool| {
            let rect = Rect::from_min_size(pos2(buttons_x, button_y), BUTTON);
            button_y += BUTTON.y + 6.0;
            theme::push_button(ui, rect, label, enabled, false).clicked()
        };
        // `_UpdateDlgButtons`.
        if button(ui, "Move Up", self.selected > 0) {
            self.move_selected(-1);
        }
        if button(ui, "Move Down", self.selected + 1 < self.rows.len()) {
            self.move_selected(1);
        }
        if button(ui, "Show", !checked && !fixed) {
            self.set_checked(self.selected, true);
        }
        if button(ui, "Hide", checked && !fixed) {
            self.set_checked(self.selected, false);
        }

        self.width_box(ui, left, width_row, &font);

        let cancel = Rect::from_min_size(pos2(right - BUTTON.x, ok_top), BUTTON);
        let ok = cancel.translate(vec2(-BUTTON.x - 6.0, 0.0));
        if theme::push_button(ui, ok, "OK", true, true).clicked() || enter {
            return Outcome::Apply;
        }
        if theme::push_button(ui, cancel, "Cancel", true, false).clicked() || escape {
            return Outcome::Cancel;
        }
        Outcome::Keep
    }

    /// The check box list. A click selects a row, a click on its box or a
    /// double-click (`NM_DBLCLK`) toggles it, as does Space; Up and Down
    /// move the selection.
    fn list(&mut self, ui: &mut Ui, rect: Rect) {
        let inner = theme::sunken_field(ui.painter(), rect);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let painter = ui.painter_at(inner);
        let (up, down, space) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Space)));
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if !typing {
            if up && self.selected > 0 {
                self.select(self.selected - 1);
            }
            if down {
                self.select(self.selected + 1);
            }
            if space {
                let on = self.rows[self.selected].1;
                self.set_checked(self.selected, !on);
            }
        }
        for i in 0..self.rows.len() {
            let row = Rect::from_min_size(pos2(inner.left(), inner.top() + 1.0 + i as f32 * ROW_HEIGHT), vec2(inner.width(), ROW_HEIGHT));
            if row.top() >= inner.bottom() {
                break;
            }
            let (column, checked) = self.rows[i];
            let check = Rect::from_min_size(pos2(row.left() + 3.0, row.center().y - 6.5), vec2(13.0, 13.0));
            theme::check_box(&painter, check, checked, true);
            let galley = ui.fonts_mut(|f| f.layout_no_wrap(column.caption().to_owned(), font.clone(), theme::color::TEXT));
            let label = Rect::from_min_size(pos2(check.right() + 4.0, row.top() + 1.0), vec2(galley.size().x + 4.0, ROW_HEIGHT - 2.0));
            let selected = i == self.selected;
            if selected {
                painter.rect_filled(label, 0.0, theme::color::SELECTION);
            }
            let text = if selected { theme::color::SELECTION_TEXT } else { theme::color::TEXT };
            painter.galley_with_override_text_color(pos2(label.left() + 2.0, label.center().y - galley.size().y / 2.0), galley, text);
            if selected {
                theme::focus_rect(&painter, label);
            }

            let resp = ui.interact(row.intersect(inner), ui.id().with(("column-row", i)), Sense::click());
            if resp.clicked() || resp.double_clicked() {
                ui.ctx().memory_mut(|m| m.stop_text_input());
                if i != self.selected {
                    self.select(i);
                }
                let on_box = resp.interact_pointer_pos().is_some_and(|p| check.expand(1.0).contains(p));
                if on_box || resp.double_clicked() {
                    self.set_checked(i, !checked);
                }
            }
        }
    }

    /// "Width of selected column (in pixels):" and its three-digit box. A
    /// change sets the selected column's width on OK.
    fn width_box(&mut self, ui: &mut Ui, left: f32, top: f32, font: &egui::FontId) {
        let label = "Width of selected column (in pixels):";
        let text_w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), theme::color::TEXT).size().x);
        let field = Rect::from_min_size(pos2(left + text_w + 8.0, top), vec2(40.0, 21.0));
        ui.painter().text(pos2(left, field.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), theme::color::TEXT);
        let text_rect = theme::sunken_field(ui.painter(), field);
        let mut edit = ui.new_child(egui::UiBuilder::new().max_rect(text_rect.shrink2(vec2(2.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        let resp = edit.add(TextEdit::singleline(&mut self.width_text).frame(egui::Frame::NONE).desired_width(f32::INFINITY).char_limit(3));
        if resp.changed() {
            // ES_NUMBER: digits only.
            self.width_text.retain(|c| c.is_ascii_digit());
            if let Ok(width) = self.width_text.parse::<u32>() {
                let column = self.rows[self.selected].0;
                if let Some(w) = self.widths.iter_mut().find(|w| w.0 == column) {
                    w.1 = width as f32;
                    w.2 = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_shows_checked_rows_in_list_order() {
        let mut columns = ColumnSet::default();
        let mut dialog = ColumnDialog::new(&columns);
        // Rows: Name, Size, Modified, Attributes, then hidden by caption.
        let created = dialog.rows.iter().position(|r| r.0 == Column::Created).unwrap();
        dialog.select(created);
        dialog.set_checked(created, true);
        while dialog.selected > 1 {
            dialog.move_selected(-1);
        }
        dialog.set_checked(dialog.rows.iter().position(|r| r.0 == Column::Attributes).unwrap(), false);
        dialog.apply(&mut columns);
        assert_eq!(columns.visible(), [Column::Name, Column::Created, Column::Size, Column::Modified]);
    }

    #[test]
    fn name_stays_checked() {
        let columns = ColumnSet::default();
        let mut dialog = ColumnDialog::new(&columns);
        dialog.set_checked(0, false);
        assert_eq!(dialog.rows[0], (Column::Name, true));
    }

    #[test]
    fn moves_stop_at_the_ends() {
        let mut dialog = ColumnDialog::new(&ColumnSet::default());
        dialog.move_selected(-1);
        assert_eq!(dialog.selected, 0);
        let last = dialog.rows.len() - 1;
        dialog.select(last);
        dialog.move_selected(1);
        assert_eq!(dialog.selected, last);
    }
}
