//! The About box Help > About Macsplorer opens, in the manner of Windows
//! 2000's "About Windows": the app icon, the name and version, a few lines
//! of text, an etched line and OK. The layout is from memory, not traced.

use std::path::PathBuf;

use eframe::egui;
use egui::{Align, Align2, Key, Layout, Rect, Sense, TextureHandle, TextureOptions, Ui, pos2, vec2};

use crate::theme::{self, Bevel};

pub const SIZE: egui::Vec2 = vec2(380.0, 272.0);
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
const TITLE: &str = "About Macsplorer";
/// The icon's side on screen.
const ICON: f32 = 48.0;
/// The icon is shrunk by this much on load; the GPU's own scaling of the 1024 px original alone looks jagged.
const SHRINK: usize = 8;

/// What the dialog wants after a frame.
pub enum Outcome {
    Open,
    Close,
    /// The font file's link was clicked: close and show the file in the explorer.
    Reveal(PathBuf),
}

pub struct AboutDialog {
    viewport: egui::ViewportId,
    icon: Option<TextureHandle>,
}

impl AboutDialog {
    pub fn new() -> Self {
        Self { viewport: egui::ViewportId::from_hash_of("about"), icon: None }
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

    /// Draws the whole window: frame, caption, the text, OK.
    pub fn ui(&mut self, ui: &mut Ui) -> Outcome {
        let icon = self.icon.get_or_insert_with(|| load_icon(ui.ctx())).clone();
        let (outer, _) = ui.allocate_exact_size(SIZE, Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, TITLE);

        let left = inner.left() + 16.0;
        let right = inner.right() - 16.0;
        let top = inner.top() + 18.0 + 16.0;
        let image = Rect::from_min_size(pos2(left, top), vec2(ICON, ICON));
        ui.painter().image(icon.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);

        let text_left = image.right() + 16.0;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let color = theme::color::text();
        let p = ui.painter();
        p.text(pos2(text_left, top - 1.0), Align2::LEFT_TOP, "Macsplorer", theme::bold(theme::FONT_SIZE + 4.0), color);
        p.text(pos2(text_left, top + 22.0), Align2::LEFT_TOP, format!("Version {}", env!("CARGO_PKG_VERSION")), font.clone(), color);
        let blurb = "A Windows Explorer style file browser for macOS and Linux, in the Windows Classic look.";
        let galley = ui.fonts_mut(|f| f.layout(blurb.to_owned(), font.clone(), color, right - text_left));
        let blurb_height = galley.size().y;
        let blurb_y = image.bottom() + 12.0;
        ui.painter().galley(pos2(left, blurb_y), galley, color);
        let galley = ui.fonts_mut(|f| f.layout("Licensed under the MIT License.".to_owned(), font.clone(), color, right - left));
        ui.painter().galley(pos2(left, blurb_y + blurb_height + 6.0), galley, color);
        let font_y = blurb_y + blurb_height + 6.0 + 20.0;
        let font_info = theme::ui_font();
        let galley = ui.fonts_mut(|f| f.layout(format!("Font: {}", font_info.label), font.clone(), color, right - left));
        let label_height = galley.size().y;
        ui.painter().galley(pos2(left, font_y), galley, color);
        let mut reveal = None;
        if let Some(path) = &font_info.path {
            // The font file as a link: it closes the box and opens the file's folder.
            let link = ui.visuals().hyperlink_color;
            let format = egui::TextFormat { font_id: font, color: link, underline: egui::Stroke::new(1.0, link), ..Default::default() };
            let mut job = egui::text::LayoutJob::single_section(path.display().to_string(), format);
            job.wrap.max_width = right - left;
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let at = Rect::from_min_size(pos2(left, font_y + label_height), galley.size());
            let response = ui.interact(at, ui.id().with("font-link"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().galley(at.min, galley, link);
            if response.clicked() {
                reveal = Some(path.clone());
            }
        }

        let ok_top = inner.bottom() - 11.0 - BUTTON.y;
        etched_line(ui, left - 5.0, right + 5.0, ok_top - 12.0);
        let ok = Rect::from_min_size(pos2(inner.center().x - BUTTON.x / 2.0, ok_top), BUTTON);
        let escape_or_enter = ui.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter));
        let ok = theme::push_button(ui, ok, "OK", true, true).clicked() || escape_or_enter;
        match reveal {
            Some(path) => Outcome::Reveal(path),
            None if ok => Outcome::Close,
            None => Outcome::Open,
        }
    }
}

/// A horizontal groove: a shadow line over a highlight line.
fn etched_line(ui: &Ui, left: f32, right: f32, y: f32) {
    let p = ui.painter();
    p.hline(left..=right, y, egui::Stroke::new(1.0, theme::color::shadow()));
    p.hline(left..=right, y + 1.0, egui::Stroke::new(1.0, theme::color::highlight()));
}

/// The app icon, shrunk with a box filter to a size that draws smoothly at `ICON` points.
fn load_icon(ctx: &egui::Context) -> TextureHandle {
    let source = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png")).expect("embedded icon is a valid PNG");
    let (width, height) = (source.width as usize / SHRINK, source.height as usize / SHRINK);
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            // Premultiplied sums, so the transparent corners don't tint the edge.
            let mut sum = [0u32; 4];
            for (dy, dx) in (0..SHRINK).flat_map(|dy| (0..SHRINK).map(move |dx| (dy, dx))) {
                let at = ((y * SHRINK + dy) * source.width as usize + x * SHRINK + dx) * 4;
                let alpha = source.rgba[at + 3] as u32;
                for (total, channel) in sum.iter_mut().zip(&source.rgba[at..at + 3]) {
                    *total += *channel as u32 * alpha;
                }
                sum[3] += alpha;
            }
            let count = (SHRINK * SHRINK) as u32;
            let average = |total: u32| total.checked_div(sum[3]).unwrap_or(0) as u8;
            pixels.extend([average(sum[0]), average(sum[1]), average(sum[2]), (sum[3] / count) as u8]);
        }
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels);
    ctx.load_texture("about-icon", image, TextureOptions::LINEAR)
}
