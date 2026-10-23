//! Password Needed: asked for when items in an encrypted zip are opened,
//! copied or extracted, after `zipfldr.dll`'s `CPassword::GetDecryptPassword`
//! (dialog 0x9a, `docs/archive-findings.md` §2.7): the file named, a
//! password field, OK disabled until something is typed. The wording is
//! from memory (the strings are in the MUI file). Windows' third button,
//! Skip File, is left out: Macsplorer asks once per operation.

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, TextEdit, Ui, pos2, vec2};

use crate::theme::{self, Bevel};

pub const SIZE: egui::Vec2 = vec2(380.0, 172.0);
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
const TITLE: &str = "Password Needed";
/// `EM_LIMITTEXT` on the password field.
const MAX_LEN: usize = 64;

pub enum Outcome {
    Keep,
    Cancel,
    Ok(String),
}

pub struct PasswordDialog {
    viewport: egui::ViewportId,
    /// The file the password is for, as the message names it.
    file: String,
    password: String,
    /// The last password was wrong.
    wrong: bool,
}

impl PasswordDialog {
    pub fn new(file: String, wrong: bool) -> Self {
        Self { viewport: egui::ViewportId::from_hash_of("password"), file, password: String::new(), wrong }
    }

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

    pub fn ui(&mut self, ui: &mut Ui) -> Outcome {
        let (escape, enter) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::Escape), i.consume_key(egui::Modifiers::NONE, Key::Enter)));
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
        let message = if self.wrong {
            format!("The password you entered for \"{}\" is wrong. Please enter the password again.", self.file)
        } else {
            format!("The file \"{}\" is password protected. Please enter the password below.", self.file)
        };
        let galley = ui.fonts_mut(|f| f.layout(message, font.clone(), theme::color::text(), right - left));
        let height = galley.size().y;
        ui.painter().galley(pos2(left, y), galley, theme::color::text());
        y += height + 12.0;

        let label = ui.fonts_mut(|f| f.layout_no_wrap("Password:".to_owned(), font.clone(), theme::color::text()));
        let label_width = label.size().x;
        ui.painter().galley(pos2(left, y + 11.0 - label.size().y / 2.0), label, theme::color::text());
        let field = Rect::from_min_max(pos2(left + label_width + 8.0, y), pos2(right, y + 22.0));
        let field_inner = theme::sunken_field(ui.painter(), field);
        let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(field_inner.shrink2(vec2(2.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        let edit = TextEdit::singleline(&mut self.password).id(ui.id().with("password-text")).password(true).char_limit(MAX_LEN).frame(egui::Frame::NONE).desired_width(f32::INFINITY);
        field_ui.add(edit).request_focus();

        let top = inner.bottom() - 11.0 - BUTTON.y;
        let cancel = Rect::from_min_size(pos2(right - BUTTON.x, top), BUTTON);
        let ok = cancel.translate(vec2(-BUTTON.x - 6.0, 0.0));
        let typed = !self.password.is_empty();
        if (theme::push_button(ui, ok, "OK", typed, true).clicked() || enter) && typed {
            return Outcome::Ok(std::mem::take(&mut self.password));
        }
        if theme::push_button(ui, cancel, "Cancel", true, false).clicked() || escape {
            return Outcome::Cancel;
        }
        Outcome::Keep
    }
}
