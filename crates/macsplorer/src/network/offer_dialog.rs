//! "Receive Files": a nearby device asks to send files, in its own
//! borderless window like the app's other dialogs. Not from Windows, which
//! had nothing like it; laid out like Windows 2000's message boxes (icon
//! on the left, the question, a list box, buttons on the bottom right).
//!
//! Decline is the default button, so a Return meant for another window
//! doesn't take files from someone else's phone.

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, Ui, pos2, vec2};

use macsplorer_core::listing::format_byte_size;

use crate::theme::{self, Bevel};

pub const TITLE: &str = "Receive Files";
const WIDTH: f32 = 420.0;
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
/// Where the text starts, right of the dialog's icon.
const TEXT_X: f32 = 58.0;
const ROW: f32 = 18.0;
/// File rows shown; more are summed up in the last one.
const MAX_ROWS: usize = 6;

/// What the dialog shows of an offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub from: String,
    /// What and where the device is: "Samsung phone at 192.168.2.15".
    pub detail: String,
    /// Names as the sender gave them, and sizes.
    pub files: Vec<(String, u64)>,
    /// The folder's name, where accepted files go.
    pub folder: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Decline,
}

impl View {
    fn rows(&self) -> usize {
        self.files.len().min(MAX_ROWS)
    }

    pub fn size(&self) -> egui::Vec2 {
        let list = self.rows() as f32 * ROW + 4.0;
        vec2(WIDTH, FRAME + 18.0 + 12.0 + 34.0 + 6.0 + list + 10.0 + 15.0 + 16.0 + BUTTON.y + 11.0 + FRAME)
    }

    /// The question: "Mystic Blackberry wants to send you 3 files (12.4 MB)."
    fn question(&self) -> String {
        let total = self.files.iter().map(|f| f.1).sum();
        let what = if self.files.len() == 1 { "a file".to_owned() } else { format!("{} files", self.files.len()) };
        format!("{} wants to send you {what} ({}).", self.from, format_byte_size(total))
    }
}

/// Shows the dialog in its own window for this frame; `Some` once it was
/// answered. `logo` is LocalSend's, drawn as the dialog's icon.
pub fn show(ctx: &egui::Context, id: u64, view: &View, logo: egui::TextureId) -> Option<Answer> {
    let builder = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_inner_size(view.size())
        .with_resizable(false)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_decorations(false)
        .with_active(true);
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("receive-offer", id)), builder, |ui, _| {
        if ui.input(|i| i.viewport().close_requested()) {
            return Some(Answer::Decline);
        }
        draw(ui, view, logo)
    })
}

/// Draws the whole window: frame, caption, the question, the files, buttons.
pub fn draw(ui: &mut Ui, view: &View, logo: egui::TextureId) -> Option<Answer> {
    let (outer, _) = ui.allocate_exact_size(view.size(), Sense::hover());
    let p = ui.painter();
    p.rect_filled(outer, 0.0, theme::color::face());
    theme::bevel(p, outer, Bevel::Raised);
    let inner = outer.shrink(FRAME);
    let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    theme::dialog_title_bar(&mut caption, TITLE);

    let top = inner.top() + 18.0 + 12.0;
    let icon = Rect::from_min_size(pos2(inner.left() + 14.0, top), vec2(32.0, 32.0));
    ui.painter().image(logo, icon, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    let (left, right) = (inner.left() + TEXT_X, inner.right() - 14.0);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let p = ui.painter();
    p.text(pos2(left, top + 2.0), egui::Align2::LEFT_TOP, view.question(), font.clone(), theme::color::text());
    p.text(pos2(left, top + 18.0), egui::Align2::LEFT_TOP, &view.detail, font.clone(), theme::color::gray_text());

    // The files, in a list box.
    let list = Rect::from_min_size(pos2(left, top + 34.0 + 6.0), vec2(right - left, view.rows() as f32 * ROW + 4.0));
    p.rect_filled(list, 0.0, theme::color::window());
    theme::bevel(p, list, Bevel::Sunken);
    let rows = view.rows();
    let clip = p.with_clip_rect(list.shrink(2.0));
    for (i, (name, size)) in view.files.iter().take(rows).enumerate() {
        let y = list.top() + 2.0 + i as f32 * ROW;
        let more = view.files.len() - rows;
        let last = i + 1 == rows && more > 0;
        let size_text = if last { String::new() } else { format_byte_size(*size) };
        let size_w = if size_text.is_empty() { 0.0 } else { clip.layout_no_wrap(size_text.clone(), font.clone(), theme::color::text()).size().x };
        let text_x = list.left() + 4.0 + 16.0 + 4.0;
        if last {
            let text = format!("and {} more", more + 1);
            clip.text(pos2(text_x, y + ROW / 2.0), egui::Align2::LEFT_CENTER, text, font.clone(), theme::color::gray_text());
            continue;
        }
        theme::paint_page(&clip, Rect::from_min_size(pos2(list.left() + 4.0, y + 1.0), vec2(16.0, 16.0)), true);
        let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), font.clone(), theme::color::text());
        job.wrap = egui::text::TextWrapping::truncate_at_width((list.right() - 8.0 - size_w - 12.0 - text_x).max(0.0));
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let gy = y + (ROW - galley.size().y) / 2.0;
        clip.galley(pos2(text_x, gy), galley, theme::color::text());
        clip.text(pos2(list.right() - 6.0, y + ROW / 2.0), egui::Align2::RIGHT_CENTER, size_text, font.clone(), theme::color::text());
    }
    let note = format!("Files you accept are saved in {}.", view.folder);
    ui.painter().text(pos2(left, list.bottom() + 10.0), egui::Align2::LEFT_TOP, note, font, theme::color::text());

    let bottom = inner.bottom() - 11.0 - BUTTON.y;
    let decline = Rect::from_min_size(pos2(right - BUTTON.x, bottom), BUTTON);
    let accept = decline.translate(vec2(-(BUTTON.x + 6.0), 0.0));
    let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    if theme::push_button(ui, accept, "Accept", true, false).clicked() {
        return Some(Answer::Accept);
    }
    if theme::push_button(ui, decline, "Decline", true, true).clicked() || enter || escape {
        return Some(Answer::Decline);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(files: &[(&str, u64)]) -> View {
        View { from: "Pixel".into(), detail: String::new(), files: files.iter().map(|(n, s)| (n.to_string(), *s)).collect(), folder: "Downloads".into() }
    }

    #[test]
    fn the_question_counts_the_files() {
        assert!(view(&[("a.jpg", 2048)]).question().starts_with("Pixel wants to send you a file ("));
        assert!(view(&[("a", 1), ("b", 2)]).question().starts_with("Pixel wants to send you 2 files ("));
    }

    #[test]
    fn long_lists_are_cut_short() {
        let many: Vec<(String, u64)> = (0..20).map(|i| (format!("{i}.jpg"), 1)).collect();
        let v = View { files: many, ..view(&[]) };
        assert_eq!(v.rows(), MAX_ROWS);
        assert!(v.size().y < view(&[("a", 1); 7]).size().y + 1.0);
    }
}
