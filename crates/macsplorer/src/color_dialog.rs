//! The Color dialog (`ChooseColor`), for the folder color on the Properties
//! dialog's Customize tab, in its own borderless window. From memory of
//! Windows 2000: comdlg32 isn't among the analyzed binaries, so the layout,
//! the 48 basic colors and the behavior are unverified. Basic colors, 16
//! custom color boxes (kept in `settings.json`), and the button
//! "Define Custom Colors >>", which widens the window to the hue/sat
//! spectrum, the lum bar and the Hue/Sat/Lum and Red/Green/Blue boxes.
//! Help is left out.

use eframe::egui;
use egui::{Align, Color32, ColorImage, Key, Layout, Rect, Sense, TextEdit, TextureHandle, Ui, pos2, vec2};

use macsplorer_core::hls;
use macsplorer_core::preferences::Preferences;

use crate::theme::{self, Bevel};

const SMALL: egui::Vec2 = vec2(236.0, 334.0);
const FULL: egui::Vec2 = vec2(470.0, 334.0);
const FRAME: f32 = 4.0;
const TITLE: &str = "Color";
/// A color box and the step from one to the next.
const BOX: egui::Vec2 = vec2(21.0, 17.0);
const PITCH: egui::Vec2 = vec2(27.0, 23.0);
/// The spectrum (inside its border) and the lum bar.
const SPECTRUM: egui::Vec2 = vec2(172.0, 184.0);
const LUM_BAR_WIDTH: f32 = 10.0;
const BUTTON_HEIGHT: f32 = 23.0;

/// The basic colors, row by row (from memory).
const BASIC: [[u8; 3]; 48] = [
    [255, 128, 128], [255, 255, 128], [128, 255, 128], [0, 255, 128], [128, 255, 255], [0, 128, 255], [255, 128, 192], [255, 128, 255],
    [255, 0, 0], [255, 255, 0], [128, 255, 0], [0, 255, 64], [0, 255, 255], [0, 128, 192], [128, 128, 192], [255, 0, 255],
    [128, 64, 64], [255, 128, 64], [0, 255, 0], [0, 128, 128], [0, 64, 128], [128, 128, 255], [128, 0, 64], [255, 0, 128],
    [128, 0, 0], [255, 128, 0], [0, 128, 0], [0, 128, 64], [0, 0, 255], [0, 0, 160], [128, 0, 128], [128, 0, 255],
    [64, 0, 0], [128, 64, 0], [0, 64, 0], [0, 64, 64], [0, 0, 128], [0, 0, 64], [64, 0, 64], [64, 0, 128],
    [0, 0, 0], [128, 128, 0], [128, 128, 64], [128, 128, 128], [64, 128, 128], [192, 192, 192], [64, 0, 64], [255, 255, 255],
];

/// What the dialog asks its owner to do after this frame.
pub enum Outcome {
    Keep,
    Cancel,
    Ok([u8; 3]),
}

/// Which box has the selection frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selected {
    Basic(usize),
    Custom(usize),
    None,
}

pub struct ColorDialog {
    viewport: egui::ViewportId,
    rgb: [u8; 3],
    /// Hue, sat and lum: kept apart from `rgb` so a gray keeps its hue.
    hls: [i32; 3],
    selected: Selected,
    custom: [Option<[u8; 3]>; 16],
    /// The custom box "Add to Custom Colors" fills next.
    next_custom: usize,
    expanded: bool,
    /// Hue, Sat, Lum, Red, Green, Blue as typed.
    fields: [String; 6],
    spectrum: Option<TextureHandle>,
}

impl ColorDialog {
    /// Opens with `rgb` chosen; its basic box is selected if it has one.
    pub fn new(rgb: [u8; 3]) -> Self {
        let custom = Preferences::load().custom_colors;
        let selected = match BASIC.iter().position(|&c| c == rgb) {
            Some(n) => Selected::Basic(n),
            None => custom.iter().position(|&c| c == Some(rgb)).map_or(Selected::None, Selected::Custom),
        };
        let mut dialog = Self {
            viewport: egui::ViewportId::from_hash_of(("color-dialog", std::time::Instant::now())),
            rgb,
            hls: hls::from_rgb(rgb),
            selected,
            custom,
            next_custom: 0,
            // A color that isn't a basic one opens with the editor showing.
            expanded: selected == Selected::None,
            fields: Default::default(),
            spectrum: None,
        };
        dialog.set_rgb(rgb, None);
        dialog
    }

    fn size(&self) -> egui::Vec2 {
        if self.expanded { FULL } else { SMALL }
    }

    /// Chooses `rgb`, and writes it into every box but `editing`.
    fn set_rgb(&mut self, rgb: [u8; 3], editing: Option<usize>) {
        self.rgb = rgb;
        let [h, s, l] = hls::from_rgb(rgb);
        // A gray has no hue or sat of its own: keep the ones shown.
        self.hls = if s == 0 { [self.hls[0], 0, l] } else { [h, s, l] };
        self.write_fields(editing);
    }

    fn set_hls(&mut self, hls: [i32; 3], editing: Option<usize>) {
        self.hls = [hls[0].clamp(0, 239), hls[1].clamp(0, 240), hls[2].clamp(0, 240)];
        self.rgb = hls::to_rgb(self.hls);
        self.selected = Selected::None;
        self.write_fields(editing);
    }

    fn write_fields(&mut self, editing: Option<usize>) {
        let values = [self.hls[0], self.hls[1], self.hls[2], self.rgb[0] as i32, self.rgb[1] as i32, self.rgb[2] as i32];
        for (n, v) in values.into_iter().enumerate() {
            if editing != Some(n) {
                self.fields[n] = v.to_string();
            }
        }
    }

    /// Shows the dialog in its own window for this frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let builder = egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size(self.size())
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

    /// Draws the whole window.
    pub fn ui(&mut self, ui: &mut Ui) -> Outcome {
        let (outer, _) = ui.allocate_exact_size(self.size(), Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, TITLE);

        let font = egui::TextStyle::Body.resolve(ui.style());
        let painter = ui.painter().clone();
        let label = |at: egui::Pos2, text: &str| {
            painter.text(at, egui::Align2::LEFT_TOP, text, font.clone(), theme::color::text());
        };
        let left = inner.left() + 8.0;
        let mut y = inner.top() + 18.0 + 8.0;
        label(pos2(left, y), "Basic colors:");
        y += 16.0;
        let mut outcome = Outcome::Keep;
        for (n, rgb) in BASIC.into_iter().enumerate() {
            let at = pos2(left + (n % 8) as f32 * PITCH.x, y + (n / 8) as f32 * PITCH.y);
            let response = self.color_box(ui, at, Some(rgb), self.selected == Selected::Basic(n), ("basic", n));
            if response.double_clicked() {
                outcome = Outcome::Ok(rgb);
            } else if response.clicked() {
                self.selected = Selected::Basic(n);
                self.set_rgb(rgb, None);
            }
        }
        y += 6.0 * PITCH.y + 8.0;
        label(pos2(left, y), "Custom colors:");
        y += 16.0;
        for n in 0..16 {
            let at = pos2(left + (n % 8) as f32 * PITCH.x, y + (n / 8) as f32 * PITCH.y);
            let response = self.color_box(ui, at, self.custom[n], self.selected == Selected::Custom(n), ("custom", n));
            if response.clicked() {
                // An empty box is white, and is where "Add" goes.
                self.selected = Selected::Custom(n);
                self.next_custom = n;
                self.set_rgb(self.custom[n].unwrap_or([255, 255, 255]), None);
            }
            if response.double_clicked() {
                outcome = Outcome::Ok(self.rgb);
            }
        }
        y += 2.0 * PITCH.y + 8.0;
        let define = Rect::from_min_size(pos2(left, y), vec2(8.0 * PITCH.x - (PITCH.x - BOX.x), BUTTON_HEIGHT));
        if theme::push_button(ui, define, "Define Custom Colors >>", !self.expanded, false).clicked() {
            self.expanded = true;
        }
        y += BUTTON_HEIGHT + 10.0;
        let ok = Rect::from_min_size(pos2(left, y), vec2(68.0, BUTTON_HEIGHT));
        let cancel = ok.translate(vec2(74.0, 0.0));
        let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if theme::push_button(ui, ok, "OK", true, true).clicked() || enter {
            outcome = Outcome::Ok(self.rgb);
        }
        if theme::push_button(ui, cancel, "Cancel", true, false).clicked() || escape {
            outcome = Outcome::Cancel;
        }
        if self.expanded {
            self.editor(ui, pos2(inner.left() + SMALL.x - FRAME + 2.0, inner.top() + 18.0 + 12.0));
        }
        outcome
    }

    /// One color box: sunken, the color inside, framed when selected.
    fn color_box(&self, ui: &Ui, at: egui::Pos2, rgb: Option<[u8; 3]>, selected: bool, id: (&str, usize)) -> egui::Response {
        let rect = Rect::from_min_size(at, BOX);
        let response = ui.interact(rect.expand(2.0), egui::Id::new(id), Sense::click());
        let p = ui.painter();
        let [r, g, b] = rgb.unwrap_or([255, 255, 255]);
        p.rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
        theme::bevel(p, rect, Bevel::Sunken);
        if selected {
            p.rect_stroke(rect.expand(2.0), 0.0, egui::Stroke::new(1.0, theme::color::text()), egui::StrokeKind::Inside);
        }
        response
    }

    /// The right half: spectrum, lum bar, the Color|Solid sample, the six
    /// number boxes and "Add to Custom Colors".
    fn editor(&mut self, ui: &mut Ui, at: egui::Pos2) {
        let p = ui.painter().clone();
        let font = egui::TextStyle::Body.resolve(ui.style());
        let frame = Rect::from_min_size(at, SPECTRUM + vec2(4.0, 4.0));
        let spectrum = frame.shrink(2.0);
        let texture = self.spectrum.get_or_insert_with(|| ui.ctx().load_texture("color-spectrum", spectrum_image(), egui::TextureOptions::LINEAR));
        p.image(texture.id(), spectrum, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        theme::bevel(&p, frame, Bevel::Sunken);
        let picked = ui.interact(spectrum, egui::Id::new("color spectrum"), Sense::click_and_drag());
        if (picked.clicked() || picked.dragged())
            && let Some(pos) = picked.interact_pointer_pos()
        {
            let hue = ((pos.x - spectrum.left()) / spectrum.width() * 239.0).round() as i32;
            let sat = (240.0 - (pos.y - spectrum.top()) / spectrum.height() * 240.0).round() as i32;
            // Black and white have no hue to show: picking one from the
            // spectrum starts at middle lum (macsplorer's choice).
            let lum = if matches!(self.hls[2], 0 | 240) { 120 } else { self.hls[2] };
            self.set_hls([hue, sat, lum], None);
        }
        // The crosshair: four short arms around the chosen hue and sat.
        let cross = pos2(spectrum.left() + self.hls[0] as f32 / 239.0 * spectrum.width(), spectrum.top() + (240 - self.hls[1]) as f32 / 240.0 * spectrum.height());
        let clip = p.with_clip_rect(spectrum);
        let stroke = egui::Stroke::new(2.0, Color32::BLACK);
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let d = vec2(dx, dy);
            clip.line_segment([cross + d * 3.0, cross + d * 9.0], stroke);
        }

        let bar_frame = Rect::from_min_size(pos2(frame.right() + 8.0, frame.top()), vec2(LUM_BAR_WIDTH + 4.0, frame.height()));
        let bar = bar_frame.shrink(2.0);
        let steps = bar.height().round() as usize;
        for n in 0..steps {
            let lum = 240 - (n as f32 / steps as f32 * 240.0).round() as i32;
            let [r, g, b] = hls::to_rgb([self.hls[0], self.hls[1], lum]);
            p.rect_filled(Rect::from_min_size(pos2(bar.left(), bar.top() + n as f32), vec2(bar.width(), 1.0)), 0.0, Color32::from_rgb(r, g, b));
        }
        theme::bevel(&p, bar_frame, Bevel::Sunken);
        let arrow_zone = Rect::from_min_max(bar_frame.left_top(), pos2(bar_frame.right() + 10.0, bar_frame.bottom()));
        let dragged = ui.interact(arrow_zone, egui::Id::new("color lum"), Sense::click_and_drag());
        if (dragged.clicked() || dragged.dragged())
            && let Some(pos) = dragged.interact_pointer_pos()
        {
            let lum = (240.0 - (pos.y - bar.top()) / bar.height() * 240.0).round() as i32;
            self.set_hls([self.hls[0], self.hls[1], lum], None);
        }
        let arrow_y = bar.top() + (240 - self.hls[2]) as f32 / 240.0 * bar.height();
        let tip = pos2(bar_frame.right() + 2.0, arrow_y);
        p.add(egui::Shape::convex_polygon(vec![tip, tip + vec2(6.0, -5.0), tip + vec2(6.0, 5.0)], theme::color::text(), egui::Stroke::NONE));

        // Color|Solid: the color and the nearest solid one, the same here
        // (no 256-color displays to dither for).
        let y = frame.bottom() + 8.0;
        let sample = Rect::from_min_size(pos2(frame.left(), y), vec2(64.0, 42.0));
        let [r, g, b] = self.rgb;
        p.rect_filled(sample, 0.0, Color32::from_rgb(r, g, b));
        theme::bevel(&p, sample, Bevel::Sunken);
        p.text(pos2(sample.center().x, sample.bottom() + 3.0), egui::Align2::CENTER_TOP, "Color|Solid", font.clone(), theme::color::text());

        let names = ["Hue:", "Sat:", "Lum:", "Red:", "Green:", "Blue:"];
        let maxima = [239, 240, 240, 255, 255, 255];
        for n in 0..6 {
            let (col, row) = (n / 3, n % 3);
            let label_x = sample.right() + 10.0 + col as f32 * 72.0;
            let row_y = y + row as f32 * 24.0;
            p.text(pos2(label_x, row_y + 10.0), egui::Align2::LEFT_CENTER, names[n], font.clone(), theme::color::text());
            // "Green:" is the widest label.
            let field = Rect::from_min_size(pos2(label_x + if col == 0 { 36.0 } else { 46.0 }, row_y), vec2(28.0, 21.0));
            let text_rect = theme::sunken_field(&p, field);
            let mut edit = ui.new_child(egui::UiBuilder::new().max_rect(text_rect.shrink2(vec2(2.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
            let changed = edit.add(TextEdit::singleline(&mut self.fields[n]).id(egui::Id::new(("color field", n))).frame(egui::Frame::NONE).char_limit(3).desired_width(f32::INFINITY)).changed();
            if changed && let Ok(v) = self.fields[n].trim().parse::<i32>() {
                let v = v.clamp(0, maxima[n]);
                if n < 3 {
                    let mut hls = self.hls;
                    hls[n] = v;
                    self.set_hls(hls, Some(n));
                } else {
                    let mut rgb = self.rgb;
                    rgb[n - 3] = v as u8;
                    self.selected = Selected::None;
                    self.set_rgb(rgb, Some(n));
                }
            }
        }

        let add = Rect::from_min_size(pos2(frame.left(), y + 3.0 * 24.0 + 2.0), vec2(bar_frame.right() + 12.0 - frame.left(), BUTTON_HEIGHT));
        if theme::push_button(ui, add, "Add to Custom Colors", true, false).clicked() {
            self.custom[self.next_custom] = Some(self.rgb);
            self.selected = Selected::Custom(self.next_custom);
            self.next_custom = (self.next_custom + 1) % 16;
            let custom = self.custom;
            Preferences::update(|prefs| prefs.custom_colors = custom);
        }
    }
}

/// Hue left to right, sat top (240) to bottom (0), at middle lum.
fn spectrum_image() -> ColorImage {
    let [w, h] = [SPECTRUM.x as usize, SPECTRUM.y as usize];
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let hue = (x as f32 / (w - 1) as f32 * 239.0).round() as i32;
            let sat = 240 - (y as f32 / (h - 1) as f32 * 240.0).round() as i32;
            let [r, g, b] = hls::to_rgb([hue, sat, 120]);
            pixels.push(Color32::from_rgb(r, g, b));
        }
    }
    ColorImage::new([w, h], pixels)
}
