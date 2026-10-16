//! Windows 2000 "Windows Classic" look: system colors, 3D bevels, Tahoma,
//! title bar, toolbar buttons and 16×16 icons, all drawn with egui.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{
    Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect, Response,
    Sense, Shadow, Stroke, TextStyle, Ui, Vec2, ViewportCommand, pos2, vec2,
};

use crate::platform;

static DARK_MODE: AtomicBool = AtomicBool::new(false);

pub fn is_dark() -> bool {
    DARK_MODE.load(Ordering::Relaxed)
}

/// Windows Classic system colors (`GetSysColor` defaults on Windows 2000),
/// or the same roles in a dark palette. The dark one is not from Windows 2000
/// (it had none, and neither do the build 10240 binaries; Explorer's dark mode
/// came with Windows 10 1809): it keeps the Classic bevels on dark grays,
/// like Windows 10's dark Explorer keeps its flat layout on dark grays.
pub mod color {
    use super::{Color32, is_dark};

    fn pick(light: Color32, dark: Color32) -> Color32 {
        if is_dark() { dark } else { light }
    }

    const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
        Color32::from_rgb(r, g, b)
    }

    pub fn face() -> Color32 {
        pick(rgb(0xD4, 0xD0, 0xC8), rgb(0x28, 0x28, 0x2C)) // COLOR_3DFACE
    }
    /// Menus and dialogs sit above the panels: in the dark palette they are
    /// lighter, as there are no shadows to lift them.
    pub fn raised_face() -> Color32 {
        pick(rgb(0xD4, 0xD0, 0xC8), rgb(0x33, 0x36, 0x3C))
    }
    pub fn highlight() -> Color32 {
        pick(Color32::WHITE, rgb(0x58, 0x5C, 0x65)) // COLOR_3DHILIGHT
    }
    pub fn light() -> Color32 {
        pick(rgb(0xD4, 0xD0, 0xC8), rgb(0x3C, 0x3F, 0x46)) // COLOR_3DLIGHT
    }
    pub fn shadow() -> Color32 {
        pick(rgb(0x80, 0x80, 0x80), rgb(0x15, 0x17, 0x1B)) // COLOR_3DSHADOW
    }
    pub fn dark() -> Color32 {
        pick(rgb(0x40, 0x40, 0x40), rgb(0x08, 0x09, 0x0B)) // COLOR_3DDKSHADOW
    }
    pub fn window() -> Color32 {
        pick(Color32::WHITE, rgb(0x1D, 0x1D, 0x20))
    }
    pub fn text() -> Color32 {
        pick(Color32::BLACK, Color32::WHITE)
    }
    pub fn gray_text() -> Color32 {
        pick(rgb(0x80, 0x80, 0x80), rgb(0x84, 0x88, 0x91))
    }
    pub fn selection() -> Color32 {
        pick(rgb(0x0A, 0x24, 0x6A), rgb(0x2B, 0x56, 0x9A)) // COLOR_HIGHLIGHT
    }
    pub fn selection_text() -> Color32 {
        pick(Color32::WHITE, rgb(0xF2, 0xF4, 0xF8))
    }
    pub fn active_caption() -> Color32 {
        pick(rgb(0x0A, 0x24, 0x6A), rgb(0x10, 0x1C, 0x38))
    }
    pub fn caption_gradient() -> Color32 {
        pick(rgb(0xA6, 0xCA, 0xF0), rgb(0x3A, 0x62, 0x9C))
    }
    pub fn caption_inactive() -> Color32 {
        pick(rgb(0x80, 0x80, 0x80), rgb(0x24, 0x26, 0x2B))
    }
    pub fn caption_inactive_gradient() -> Color32 {
        pick(rgb(0xC0, 0xC0, 0xC0), rgb(0x3A, 0x3D, 0x44))
    }
    pub fn caption_inactive_text() -> Color32 {
        pick(rgb(0xD4, 0xD0, 0xC8), rgb(0x90, 0x90, 0x90))
    }
    /// A checked toolbar button: Windows draws it with a dither of face and
    /// white, approximated by a flat in-between color.
    pub fn checked_face() -> Color32 {
        pick(rgb(0xEA, 0xE8, 0xE3), rgb(0x3F, 0x42, 0x49))
    }
    pub fn tree_line() -> Color32 {
        pick(rgb(0x80, 0x80, 0x80), rgb(0x6A, 0x6E, 0x77))
    }

    // Special file names, after Ubuntu's `ls` colors in the dark shades of
    // the 16-color palette, which read on a white list; the bright shades
    // on a dark one.
    pub fn executable() -> Color32 {
        pick(rgb(0x00, 0x80, 0x00), rgb(0x4C, 0xD9, 0x4C))
    }
    pub fn link() -> Color32 {
        pick(rgb(0x00, 0x80, 0x80), rgb(0x4C, 0xD6, 0xD6))
    }
    pub fn archive() -> Color32 {
        pick(rgb(0x80, 0x00, 0x00), rgb(0xFF, 0x6B, 0x6B))
    }
    /// The Git summary's colors. posh-git uses console colors (cyan, green,
    /// red, yellow, dark green, dark red); these are darker, to read on the
    /// button face, and lighter again in the dark palette.
    pub fn git(tone: macsplorer_core::git::Tone) -> Color32 {
        use macsplorer_core::git::Tone;
        match tone {
            Tone::Delimiter | Tone::Diverged => pick(rgb(0x80, 0x80, 0x00), rgb(0xE0, 0xD0, 0x50)),
            Tone::Branch => link(),
            Tone::Ahead => pick(rgb(0x00, 0x90, 0x00), rgb(0x4C, 0xD9, 0x4C)),
            Tone::Behind => pick(rgb(0xC0, 0x00, 0x00), rgb(0xFF, 0x6B, 0x6B)),
            Tone::Index => pick(rgb(0x00, 0x60, 0x00), rgb(0x40, 0xB0, 0x40)),
            Tone::Working => pick(rgb(0x80, 0x00, 0x00), rgb(0xE0, 0x60, 0x60)),
        }
    }
    /// The status bar message for a failed operation.
    pub fn error_text() -> Color32 {
        pick(rgb(0x80, 0x00, 0x00), rgb(0xFF, 0x6B, 0x6B))
    }

    // The same in both palettes: icon art.
    pub const FOLDER: Color32 = Color32::from_rgb(0xF5, 0xD4, 0x6B);
    pub const FOLDER_DARK: Color32 = Color32::from_rgb(0x9C, 0x7A, 0x1E);
    pub const FOLDER_LIGHT: Color32 = Color32::from_rgb(0xFF, 0xF1, 0xB8);
    pub const ARROW: Color32 = Color32::from_rgb(0x2E, 0x8B, 0x2E);
    pub const ARROW_DARK: Color32 = Color32::from_rgb(0x14, 0x50, 0x14);
    /// The sheet of a document icon and its outline stay a white page.
    pub const PAPER: Color32 = Color32::WHITE;
    pub const PAPER_EDGE: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
}
use color::*;

pub const FONT_SIZE: f32 = 12.0;

/// VSCodium's Seti file icon font (see `assets/seti`).
pub fn seti_family() -> FontFamily {
    FontFamily::Name("seti".into())
}

pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Menu shortcut text: the key symbols (⌘ ⌥ ⌫) come from the symbol font so they
/// share one design, the letters from the usual stack.
pub fn shortcut_family() -> FontFamily {
    FontFamily::Name("shortcut".into())
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}

/// Installs fonts and visuals. Tahoma is read from the system fonts at
/// runtime (not bundled, see `platform::TAHOMA`); egui's default font is the
/// fallback.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut bold_fonts = Vec::new();
    #[cfg(target_os = "linux")]
    let system = crate::system_fonts::SystemFonts::scan();
    #[cfg(unix)]
    for (key, paths, family) in [("tahoma", platform::TAHOMA, FontFamily::Proportional), ("tahoma-bold", platform::TAHOMA_BOLD, bold_family())] {
        let data = paths.iter().find_map(|path| std::fs::read(path).ok()).map(FontData::from_owned);
        // No Tahoma at the known paths: one elsewhere on the system, else the bundled DejaVu.
        #[cfg(target_os = "linux")]
        let data = data
            .or_else(|| {
                let found = system.ui_font(family != FontFamily::Proportional)?;
                Some(FontData { index: found.index, ..FontData::from_owned(found.data) })
            })
            .or_else(|| Some(FontData::from_static(platform::bundled_ui_font(family != FontFamily::Proportional))));
        if let Some(data) = data {
            fonts.font_data.insert(key.into(), Arc::new(data));
            if family == FontFamily::Proportional {
                fonts.families.entry(family).or_default().insert(0, key.into());
            } else {
                bold_fonts.push(key.to_string());
            }
        }
    }
    // Scripts and symbols the UI font lacks (file names hold anything), after egui's own fonts.
    #[cfg(target_os = "linux")]
    for (n, found) in system.fallbacks(&std::env::var("LANG").unwrap_or_default()).into_iter().enumerate() {
        let key = format!("system-{n}-{}", found.family);
        fonts.font_data.insert(key.clone(), Arc::new(FontData { index: found.index, ..FontData::from_owned(found.data) }));
        fonts.families.entry(FontFamily::Proportional).or_default().push(key);
    }
    if let Some(bytes) = platform::SYMBOL_FONT.iter().find_map(|path| std::fs::read(path).ok()) {
        fonts.font_data.insert("symbols".into(), Arc::new(FontData::from_owned(bytes)));
        fonts.families.entry(FontFamily::Proportional).or_default().push("symbols".into());
    }
    // Apple Symbols draws its glyphs small next to Tahoma: a scaled copy for the shortcuts.
    let mut shortcut = Vec::new();
    if let Some(bytes) = platform::SYMBOL_FONT.iter().find_map(|path| std::fs::read(path).ok()) {
        let tweak = egui::FontTweak { scale: 1.3, y_offset_factor: 0.12, ..Default::default() };
        fonts.font_data.insert("shortcut-symbols".into(), Arc::new(FontData::from_owned(bytes).tweak(tweak)));
        shortcut.extend(fonts.families.get(&FontFamily::Proportional).and_then(|f| f.first()).cloned().filter(|k| k == "tahoma"));
        shortcut.push("shortcut-symbols".to_string());
    }
    shortcut.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(shortcut_family(), shortcut);
    // Bold falls back to the regular stack (Tahoma, then egui defaults, then symbols).
    let regular = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    bold_fonts.extend(regular);
    fonts.families.insert(bold_family(), bold_fonts);
    fonts.font_data.insert("seti".into(), Arc::new(FontData::from_static(include_bytes!("../../../assets/seti/seti.ttf"))));
    fonts.families.insert(seti_family(), vec!["seti".into()]);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        use FontFamily::Proportional;
        style.text_styles = [
            (TextStyle::Small, FontId::new(9.0, Proportional)),
            (TextStyle::Body, FontId::new(FONT_SIZE, Proportional)),
            (TextStyle::Button, FontId::new(FONT_SIZE, Proportional)),
            (TextStyle::Heading, FontId::new(14.0, Proportional)),
            (TextStyle::Monospace, FontId::new(FONT_SIZE, FontFamily::Monospace)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = vec2(4.0, 2.0);
        s.button_padding = vec2(6.0, 2.0);
        s.interact_size.y = 18.0;
        s.menu_margin = egui::Margin::same(2);
        s.window_margin = egui::Margin::same(3);
        // egui scroll bars, for areas not using `scrollbar::Bars`: opaque,
        // no edge fade.
        s.scroll = egui::style::ScrollStyle {
            bar_width: 16.0,
            bar_inner_margin: 0.0,
            bar_outer_margin: 0.0,
            dormant_background_opacity: 1.0,
            active_background_opacity: 1.0,
            interact_background_opacity: 1.0,
            dormant_handle_opacity: 1.0,
            active_handle_opacity: 1.0,
            interact_handle_opacity: 1.0,
            fade: egui::style::ScrollFadeStyle { strength: 0.0, ..Default::default() },
            ..egui::style::ScrollStyle::solid()
        };

    });
    apply_visuals(ctx);
}

/// Switches between the light and dark palette and restyles egui. The app's
/// own painting reads the palette every frame, so it follows by repainting.
pub fn set_dark(ctx: &egui::Context, dark: bool) {
    DARK_MODE.store(dark, Ordering::Relaxed);
    apply_visuals(ctx);
    ctx.request_repaint();
}

fn apply_visuals(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = if is_dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = face();
        v.window_fill = raised_face();
        v.window_stroke = Stroke::new(1.0, shadow());
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow { offset: [2, 2], blur: 0, spread: 0, color: Color32::from_black_alpha(if is_dark() { 160 } else { 90 }) };
        v.window_corner_radius = CornerRadius::ZERO;
        v.menu_corner_radius = CornerRadius::ZERO;
        v.extreme_bg_color = window();
        v.text_edit_bg_color = Some(window());
        v.faint_bg_color = window();
        v.striped = false;
        v.hyperlink_color = selection();
        v.override_text_color = None;
        v.selection.bg_fill = selection();
        v.selection.stroke = Stroke::new(1.0, selection_text());
        v.interact_cursor = None;
        v.text_cursor.stroke = Stroke::new(1.0, text());

        let w = &mut v.widgets;
        for wv in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            wv.corner_radius = CornerRadius::ZERO;
            wv.expansion = 0.0;
            wv.bg_fill = face();
            wv.weak_bg_fill = face();
            wv.fg_stroke = Stroke::new(1.0, text());
            wv.bg_stroke = Stroke::new(1.0, shadow());
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, shadow());
        // Menu items and other egui-drawn hovers use the classic navy highlight.
        for wv in [&mut w.hovered, &mut w.active, &mut w.open] {
            wv.bg_fill = selection();
            wv.weak_bg_fill = selection();
            wv.fg_stroke = Stroke::new(1.0, selection_text());
            wv.bg_stroke = Stroke::NONE;
        }
    });
}

// ---- bevels -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Bevel {
    /// Push buttons, column headers, window frame.
    Raised,
    /// Edit boxes, list views, tree views.
    Sunken,
    /// Hot toolbar buttons, status bar outline.
    RaisedThin,
    /// Pressed toolbar buttons, status bar panels.
    SunkenThin,
}

/// Draws a Windows Classic 3D edge just inside `r`.
pub fn bevel(p: &Painter, r: Rect, kind: Bevel) {
    let (outer_tl, outer_br, inner) = match kind {
        Bevel::Raised => (highlight(), dark(), Some((light(), shadow()))),
        Bevel::Sunken => (shadow(), highlight(), Some((dark(), light()))),
        Bevel::RaisedThin => (highlight(), shadow(), None),
        Bevel::SunkenThin => (shadow(), highlight(), None),
    };
    edge(p, r, outer_tl, outer_br);
    if let Some((tl, br)) = inner {
        edge(p, r.shrink(1.0), tl, br);
    }
}

fn edge(p: &Painter, r: Rect, tl: Color32, br: Color32) {
    // Pixel-aligned 1px lines along the inside of the rect.
    let (l, t, rr, b) = (r.left() + 0.5, r.top() + 0.5, r.right() - 0.5, r.bottom() - 0.5);
    p.line_segment([pos2(l, b), pos2(l, t)], Stroke::new(1.0, tl));
    p.line_segment([pos2(l, t), pos2(rr, t)], Stroke::new(1.0, tl));
    p.line_segment([pos2(rr, t), pos2(rr, b)], Stroke::new(1.0, br));
    p.line_segment([pos2(l - 0.5, b), pos2(rr + 0.5, b)], Stroke::new(1.0, br));
}

/// A 1 pt square at `pos` in a solid color. egui turns small filled rects into
/// anti-aliased line segments, which fade away at 1x scale (the Linux port), so
/// this adds an unfeathered mesh quad snapped to the pixel grid instead.
pub fn dot(p: &Painter, pos: Pos2, color: Color32) {
    let ppp = p.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let min = pos2(snap(pos.x), snap(pos.y));
    let max = pos2(snap(pos.x + 1.0).max(min.x + 1.0 / ppp), snap(pos.y + 1.0).max(min.y + 1.0 / ppp));
    let mut mesh = egui::Mesh::default();
    mesh.add_colored_rect(Rect::from_min_max(min, max), color);
    p.add(egui::Shape::mesh(mesh));
}

/// A dotted outline just inside `r`, like `DrawFocusRect`: 1 pt dots on
/// alternate points of a checkerboard, so the pattern holds still while the
/// rectangle moves. Windows XORs the dots; black is the closest egui can do.
pub fn focus_rect(p: &Painter, r: Rect) {
    focus_rect_colored(p, r, text());
}

/// [`focus_rect`] over a known solid background `bg`, in the color the XOR
/// gives there: light dots on the selection's blue, dark ones on white.
pub fn focus_rect_over(p: &Painter, r: Rect, bg: Color32) {
    focus_rect_colored(p, r, Color32::from_rgb(!bg.r(), !bg.g(), !bg.b()));
}

fn focus_rect_colored(p: &Painter, r: Rect, color: Color32) {
    let (x0, y0) = (r.left().round(), r.top().round());
    let (x1, y1) = (r.right().round() - 1.0, r.bottom().round() - 1.0);
    if x1 < x0 || y1 < y0 {
        return;
    }
    let dot = |x: f32, y: f32| {
        if (x + y).rem_euclid(2.0) == 0.0 {
            dot(p, pos2(x, y), color);
        }
    };
    let mut x = x0;
    while x <= x1 {
        dot(x, y0);
        if y1 > y0 {
            dot(x, y1);
        }
        x += 1.0;
    }
    let mut y = y0 + 1.0;
    while y < y1 {
        dot(x0, y);
        if x1 > x0 {
            dot(x1, y);
        }
        y += 1.0;
    }
}

/// egui's menu style with roomier items: its own horizontal padding is 2 px,
/// which leaves the last letter of a label touching the edge of the menu.
pub fn menu_style(style: &mut egui::Style) {
    egui::containers::menu::menu_style(style);
    style.spacing.button_padding.x = 8.0;
}

/// A checkbox row for a menu. egui's checkbox has no padding of its own, so
/// unlike the other menu items its label would touch the right edge.
pub fn menu_checkbox(ui: &mut Ui, checked: &mut bool, text: &str) -> Response {
    let spacing = ui.spacing();
    let pad = (spacing.button_padding.x - spacing.item_spacing.x).max(0.0);
    let mut atoms = egui::Atoms::new(text.to_owned());
    atoms.push_right(egui::Atom::custom(egui::Id::new("menu_checkbox_pad"), vec2(pad, 0.0)));
    ui.add(egui::Checkbox::new(checked, atoms))
}

/// The etched groove between rebar bands and menu sections.
pub fn etched_hline(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
    let p = ui.painter();
    p.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, shadow()));
    p.hline(r.x_range(), r.top() + 1.5, Stroke::new(1.0, highlight()));
}

pub fn etched_vline(ui: &mut Ui, height: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(6.0, height), Sense::hover());
    let x = r.center().x.floor();
    let p = ui.painter();
    p.vline(x + 0.5, r.y_range(), Stroke::new(1.0, shadow()));
    p.vline(x + 1.5, r.y_range(), Stroke::new(1.0, highlight()));
}

/// A classic 13 × 13 check box at `rect`'s top left: a sunken white well,
/// gray when disabled, with the 7 × 7 check mark when `checked`.
pub fn check_box(p: &Painter, rect: Rect, checked: bool, enabled: bool) {
    let r = Rect::from_min_size(rect.min, vec2(13.0, 13.0));
    p.rect_filled(r, 0.0, if enabled { window() } else { face() });
    bevel(p, r, Bevel::Sunken);
    if !checked {
        return;
    }
    // Each column of the mark is three pixels tall.
    const TOPS: [f32; 7] = [2.0, 3.0, 4.0, 3.0, 2.0, 1.0, 0.0];
    let color = if enabled { text() } else { shadow() };
    for (x, top) in TOPS.iter().enumerate() {
        for y in 0..3 {
            dot(p, r.min + vec2(3.0 + x as f32, 3.0 + top + y as f32), color);
        }
    }
}

/// Paints a white sunken field into `rect` and returns the area inside the bevel.
pub fn sunken_field(p: &Painter, rect: Rect) -> Rect {
    p.rect_filled(rect, 0.0, window());
    bevel(p, rect, Bevel::Sunken);
    rect.shrink(2.0)
}

// ---- buttons -----------------------------------------------------------------

/// A flat toolbar button: raised when hot, sunken when pressed (IE5 coolbar).
pub fn tool_button(ui: &mut Ui, icon: Icon, label: Option<&str>, enabled: bool) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let galley = label.map(|l| ui.painter().layout_no_wrap(l.into(), font.clone(), if enabled { text() } else { gray_text() }));
    // Disabled text is embossed: a white copy one pixel down-right.
    let emboss = label.filter(|_| !enabled).map(|l| ui.painter().layout_no_wrap(l.into(), font, highlight()));
    let text_w = galley.as_ref().map_or(0.0, |g| g.size().x + 4.0);
    let size = vec2(6.0 + 16.0 + text_w + 6.0, 24.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let p = ui.painter();
    let down = enabled && resp.is_pointer_button_down_on();
    if down {
        bevel(p, rect, Bevel::SunkenThin);
    } else if enabled && resp.hovered() {
        bevel(p, rect, Bevel::RaisedThin);
    }
    let offset = if down { vec2(1.0, 1.0) } else { Vec2::ZERO };
    let icon_rect = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 8.0) + offset, vec2(16.0, 16.0));
    paint_icon(p, icon_rect, icon, enabled);
    if let Some(g) = galley {
        let pos = pos2(icon_rect.right() + 4.0, rect.center().y - g.size().y / 2.0) + offset;
        if let Some(e) = emboss {
            p.galley(pos + vec2(1.0, 1.0), e, highlight());
        }
        p.galley(pos, g, text());
    }
    resp
}

/// A toolbar button that stays pushed in while `checked` (e.g. "Folders").
pub fn tool_toggle(ui: &mut Ui, icon: Icon, label: &str, checked: bool) -> Response {
    if !checked {
        return tool_button(ui, icon, Some(label), true);
    }
    // Checked buttons get the classic light dither behind a sunken edge.
    let bg = ui.painter().add(egui::Shape::Noop);
    let resp = tool_button(ui, icon, Some(label), true);
    ui.painter().set(bg, egui::Shape::rect_filled(resp.rect, 0.0, checked_face()));
    bevel(ui.painter(), resp.rect, Bevel::SunkenThin);
    resp
}

/// The small ▾ split part next to Back/Forward.
pub fn drop_arrow_button(ui: &mut Ui, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(12.0, 24.0), sense);
    let p = ui.painter();
    if enabled && resp.is_pointer_button_down_on() {
        bevel(p, rect, Bevel::SunkenThin);
    } else if enabled && resp.hovered() {
        bevel(p, rect, Bevel::RaisedThin);
    }
    let c = rect.center();
    let color = if enabled { text() } else { gray_text() };
    p.add(egui::Shape::convex_polygon(
        vec![pos2(c.x - 3.0, c.y - 1.0), pos2(c.x + 3.0, c.y - 1.0), pos2(c.x, c.y + 2.0)],
        color,
        Stroke::NONE,
    ));
    resp
}

/// A button that only opens a menu, icon and ▾ in one bevel (the toolbar's
/// Views button, `BTNS_WHOLEDROPDOWN`). `name` is for accessibility.
pub fn tool_dropdown(ui: &mut Ui, icon: Icon, name: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(6.0 + 16.0 + 4.0 + 6.0 + 4.0, 24.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    let p = ui.painter();
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let down = open || resp.is_pointer_button_down_on();
    if down {
        bevel(p, rect, Bevel::SunkenThin);
    } else if resp.hovered() {
        bevel(p, rect, Bevel::RaisedThin);
    }
    let offset = if down { vec2(1.0, 1.0) } else { Vec2::ZERO };
    let icon_rect = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 8.0) + offset, vec2(16.0, 16.0));
    paint_icon(p, icon_rect, icon, true);
    let c = pos2(icon_rect.right() + 6.0, rect.center().y) + offset;
    p.add(egui::Shape::convex_polygon(
        vec![pos2(c.x - 3.0, c.y - 1.0), pos2(c.x + 3.0, c.y - 1.0), pos2(c.x, c.y + 2.0)],
        text(),
        Stroke::NONE,
    ));
    resp
}

/// A dialog push button in `rect`. The default button (the one Return
/// presses) has an extra black frame; pressed is a black frame with a
/// shadow line inside; disabled text is embossed gray.
pub fn push_button(ui: &mut Ui, rect: Rect, label: &str, enabled: bool, default: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, ui.id().with(("push_button", label)), sense);
    let p = ui.painter();
    p.rect_filled(rect, 0.0, face());
    let down = enabled && resp.is_pointer_button_down_on() && resp.hovered();
    let inner = if default {
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, dark()), egui::StrokeKind::Inside);
        rect.shrink(1.0)
    } else {
        rect
    };
    if down {
        p.rect_stroke(inner, 0.0, Stroke::new(1.0, dark()), egui::StrokeKind::Inside);
        edge(p, inner.shrink(1.0), shadow(), face());
    } else {
        bevel(p, inner, Bevel::Raised);
    }
    let center = rect.center() + if down { vec2(1.0, 1.0) } else { Vec2::ZERO };
    let font = FontId::new(FONT_SIZE, FontFamily::Proportional);
    if enabled {
        p.text(center, Align2::CENTER_CENTER, label, font, text());
    } else {
        p.text(center + vec2(1.0, 1.0), Align2::CENTER_CENTER, label, font.clone(), highlight());
        p.text(center, Align2::CENTER_CENTER, label, font, shadow());
    }
    resp
}

/// A tiny close box used on pane headers.
pub fn close_box(ui: &mut Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
    let p = ui.painter();
    if resp.is_pointer_button_down_on() {
        bevel(p, rect, Bevel::SunkenThin);
    } else if resp.hovered() {
        bevel(p, rect, Bevel::RaisedThin);
    }
    let r = rect.shrink(4.0);
    let s = Stroke::new(1.0, text());
    p.line_segment([r.left_top(), r.right_bottom()], s);
    p.line_segment([r.right_top(), r.left_bottom()], s);
    resp
}

// ---- title bar ------------------------------------------------------------------

/// The gradient caption with minimize / maximize / close, replacing the
/// hidden macOS title bar. Dragging it moves the window; double-click zooms.
pub fn title_bar(ui: &mut Ui, title: &str) {
    caption(ui, title, false);
}

/// A dialog's caption: no icon and only a close button. Win2000 property
/// sheets also have "?" (context help), left out as there is no help.
pub fn dialog_title_bar(ui: &mut Ui, title: &str) {
    caption(ui, title, true);
}

fn caption(ui: &mut Ui, title: &str, dialog: bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
    let (active, maximized) = ui.ctx().input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().maximized.unwrap_or(false)));
    let (left, right, text) = if active {
        (active_caption(), caption_gradient(), Color32::WHITE)
    } else {
        (caption_inactive(), caption_inactive_gradient(), caption_inactive_text())
    };
    let p = ui.painter();
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);

    // On macOS, not Windows' system menu: the red dot closes, like the macOS close button in the
    // top left. The folder icon next to it does nothing, as in Finder's title bar.
    let icon_left = if cfg!(target_os = "macos") { 16.0 } else { 2.0 };
    let icon = Rect::from_min_size(pos2(rect.left() + icon_left, rect.top() + 1.0), vec2(if dialog { 0.0 } else { 16.0 }, 16.0));
    if cfg!(target_os = "macos") {
        let close_area = Rect::from_min_max(pos2(rect.left(), rect.top()), pos2(icon.left(), rect.bottom()));
        let b = ui.interact(close_area, ui.id().with("caption_icon"), Sense::click());
        mac_close_dot(p, pos2(rect.left() + 8.0, rect.center().y), active, b.hovered(), b.is_pointer_button_down_on());
        if b.clicked() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
    }
    if !dialog {
        paint_icon(p, icon, Icon::FolderOpen, true);
    }

    // Caption buttons, right to left: close, (gap), maximize, minimize.
    let btn = vec2(16.0, 14.0);
    let y = rect.center().y - btn.y / 2.0;
    let close = Rect::from_min_size(pos2(rect.right() - 2.0 - btn.x, y), btn);
    let max = close.translate(vec2(-btn.x - 2.0, 0.0));
    let min = max.translate(vec2(-btn.x, 0.0));

    // A title too long for the caption ends in an ellipsis before the buttons.
    let text_left = icon.right() + 3.0;
    let text_right = if dialog { close.left() } else { min.left() } - 4.0;
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), bold(FONT_SIZE), text);
    job.wrap = egui::text::TextWrapping::truncate_at_width((text_right - text_left).max(0.0));
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    p.galley(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, text);
    let ctx = ui.ctx().clone();
    let buttons = [(min, Caption::Minimize), (max, if maximized { Caption::Restore } else { Caption::Maximize }), (close, Caption::Close)];
    for (r, kind) in buttons.into_iter().skip(if dialog { 2 } else { 0 }) {
        let b = ui.interact(r, ui.id().with(("caption", kind as u8)), Sense::click());
        caption_button(ui.painter(), r, kind, b.is_pointer_button_down_on());
        if b.clicked() {
            ctx.send_viewport_cmd(match kind {
                Caption::Minimize => ViewportCommand::Minimized(true),
                Caption::Maximize => ViewportCommand::Maximized(true),
                Caption::Restore => ViewportCommand::Maximized(false),
                Caption::Close => ViewportCommand::Close,
            });
        }
    }

    if resp.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if resp.double_clicked() && !dialog {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

/// The macOS close button: red, gray while the window is inactive (unless the pointer is on it),
/// with an x while hovered, darker while pressed. Not from Windows; colors after AppKit's.
fn mac_close_dot(p: &egui::Painter, c: Pos2, active: bool, hovered: bool, pressed: bool) {
    let (fill, edge) = if pressed {
        (Color32::from_rgb(0xbf, 0x48, 0x42), Color32::from_rgb(0x9a, 0x2a, 0x24))
    } else if active || hovered {
        (Color32::from_rgb(0xff, 0x5f, 0x57), Color32::from_rgb(0xc0, 0x30, 0x28))
    } else {
        (Color32::from_rgb(0xd0, 0xd0, 0xd0), Color32::from_rgb(0xa8, 0xa8, 0xa8))
    };
    p.circle(c, 5.0, fill, Stroke::new(1.0, edge));
    if hovered {
        let x = Stroke::new(1.2, Color32::from_rgb(0x4d, 0x00, 0x00));
        let d = 2.2;
        p.line_segment([c + vec2(-d, -d), c + vec2(d, d)], x);
        p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], x);
    }
}

/// Resize handles on the 4 px window frame. macOS keeps edge resizing on a
/// borderless window, but on Linux the compositor gives it none, so the app
/// asks for a resize drag itself. Does nothing on macOS or when maximized.
pub fn resize_frame(ui: &Ui, outer: Rect) {
    if cfg!(target_os = "macos") || ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    use egui::viewport::ResizeDirection as Dir;
    use egui::CursorIcon as Cursor;
    const EDGE: f32 = 4.0;
    const CORNER: f32 = 12.0;
    let (l, r, t, b) = (outer.left(), outer.right(), outer.top(), outer.bottom());
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    // Corners first: they take priority over the edges they overlap.
    let handles = [
        (rect(l, t, l + CORNER, t + CORNER), Dir::NorthWest, Cursor::ResizeNwSe),
        (rect(r - CORNER, t, r, t + CORNER), Dir::NorthEast, Cursor::ResizeNeSw),
        (rect(l, b - CORNER, l + CORNER, b), Dir::SouthWest, Cursor::ResizeNeSw),
        (rect(r - CORNER, b - CORNER, r, b), Dir::SouthEast, Cursor::ResizeNwSe),
        (rect(l, t, r, t + EDGE), Dir::North, Cursor::ResizeVertical),
        (rect(l, b - EDGE, r, b), Dir::South, Cursor::ResizeVertical),
        (rect(l, t, l + EDGE, b), Dir::West, Cursor::ResizeHorizontal),
        (rect(r - EDGE, t, r, b), Dir::East, Cursor::ResizeHorizontal),
    ];
    for (i, (area, dir, cursor)) in handles.into_iter().enumerate() {
        let resp = ui.interact(area, ui.id().with(("resize", i)), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

/// Width of the size grip at the right end of the status bar.
pub const GRIP_WIDTH: f32 = 16.0;

/// Whether the window is maximized (the grip is hidden then, as in Windows).
pub fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// The status bar's size grip: three diagonal ridges (shadow with a highlight
/// line beside it) in the bottom right corner, and a drag handle that resizes
/// the window. On macOS winit cannot start a resize drag, so the size follows
/// the pointer instead.
pub fn size_grip(ui: &Ui, area: Rect) {
    let p = ui.painter();
    let (right, bottom) = (area.right(), area.bottom());
    let pixel = |dx: f32, dy: f32, c: Color32| {
        let min = pos2(right - dx - 1.0, bottom - dy - 1.0);
        p.rect_filled(Rect::from_min_size(min, vec2(1.0, 1.0)), 0.0, c);
    };
    for n in 0..3 {
        let sum = 3 + 4 * n;
        for dx in 0..=sum + 1 {
            pixel(dx as f32, (sum + 1 - dx) as f32, highlight());
        }
        for dx in 0..=sum {
            pixel(dx as f32, (sum - dx) as f32, shadow());
        }
    }
    let resp = ui.interact(area, ui.id().with("size_grip"), Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }
    if cfg!(target_os = "macos") {
        if resp.dragged() {
            let size = ui.ctx().input(|i| i.viewport().inner_rect.map(|r| r.size()));
            if let Some(size) = size {
                let min = macsplorer_core::preferences::MIN_WINDOW;
                let to = vec2((size.x + resp.drag_delta().x).max(min[0]), (size.y + resp.drag_delta().y).max(min[1]));
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(to));
            }
        }
    } else if resp.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(egui::viewport::ResizeDirection::SouthEast));
    }
}

#[derive(Clone, Copy)]
enum Caption {
    Minimize,
    Maximize,
    Restore,
    Close,
}

fn caption_button(p: &Painter, r: Rect, kind: Caption, down: bool) {
    p.rect_filled(r, 0.0, face());
    if down {
        p.rect_stroke(r, 0.0, Stroke::new(1.0, dark()), egui::StrokeKind::Inside);
        edge(p, r.shrink(1.0), shadow(), face());
    } else {
        bevel(p, r, Bevel::Raised);
    }
    let o = if down { vec2(1.0, 1.0) } else { Vec2::ZERO };
    let g = r.shrink2(vec2(4.0, 3.0)).translate(o);
    let s = Stroke::new(1.0, text());
    match kind {
        Caption::Minimize => {
            p.rect_filled(Rect::from_min_max(pos2(g.left(), g.bottom() - 2.0), pos2(g.left() + 6.0, g.bottom())), 0.0, text());
        }
        Caption::Maximize => {
            p.rect_stroke(g, 0.0, s, egui::StrokeKind::Inside);
            p.hline(g.x_range(), g.top() + 1.5, s);
        }
        Caption::Restore => {
            let back = Rect::from_min_size(g.left_top() + vec2(2.0, 0.0), vec2(6.0, 5.0));
            let front = Rect::from_min_size(g.left_top() + vec2(0.0, 3.0), vec2(6.0, 5.0));
            p.rect_stroke(back, 0.0, s, egui::StrokeKind::Inside);
            p.rect_filled(front, 0.0, face());
            p.rect_stroke(front, 0.0, s, egui::StrokeKind::Inside);
            p.hline(front.x_range(), front.top() + 1.5, s);
        }
        Caption::Close => {
            let c = g.shrink2(vec2(0.5, 0.0));
            let s = Stroke::new(1.6, text());
            p.line_segment([c.left_top(), c.right_bottom()], s);
            p.line_segment([c.right_top(), c.left_bottom()], s);
        }
    }
}

// ---- icons -------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Back,
    Forward,
    Up,
    Refresh,
    Folders,
    Undo,
    Views,
    Go,
    Folder,
    FolderOpen,
    App,
    Drive,
}

/// Draws a 16×16 icon scaled into `r`. Disabled icons are embossed gray,
/// like Windows draws disabled toolbar images.
pub fn paint_icon(p: &Painter, r: Rect, icon: Icon, enabled: bool) {
    let s = r.width() / 16.0;
    let at = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s);
    let poly = |pts: &[(f32, f32)], fill: Color32, stroke: Color32| {
        let points: Vec<Pos2> = pts.iter().map(|&(x, y)| at(x, y)).collect();
        p.add(PathShape::convex_polygon(points, fill, PathStroke::new(1.0, stroke)));
    };
    let gray = |c: Color32| if enabled { c } else { shadow() };
    match icon {
        Icon::Back | Icon::Forward | Icon::Go => {
            let pts: &[(f32, f32)] = if icon == Icon::Go {
                &[(3.0, 6.0), (8.0, 6.0), (8.0, 3.0), (13.0, 8.0), (8.0, 13.0), (8.0, 10.0), (3.0, 10.0)]
            } else {
                &[(1.5, 8.0), (7.5, 2.0), (7.5, 5.0), (14.5, 5.0), (14.5, 11.0), (7.5, 11.0), (7.5, 14.0)]
            };
            let pts: Vec<(f32, f32)> =
                if icon == Icon::Forward { pts.iter().map(|&(x, y)| (16.0 - x, y)).collect() } else { pts.to_vec() };
            if !enabled {
                let emboss: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (x + 1.0, y + 1.0)).collect();
                arrow_polygon(p, &emboss.iter().map(|&(x, y)| at(x, y)).collect::<Vec<_>>(), highlight(), highlight());
            }
            arrow_polygon(p, &pts.iter().map(|&(x, y)| at(x, y)).collect::<Vec<_>>(), gray(ARROW), gray(ARROW_DARK));
        }
        Icon::Up => {
            folder(p, r, enabled);
            let c = gray(text());
            poly(&[(8.0, 4.0), (12.0, 8.0), (4.0, 8.0)], c, c);
            p.rect_filled(Rect::from_min_max(at(7.0, 8.0), at(9.0, 12.0)), 0.0, c);
        }
        Icon::Refresh => {
            let c = gray(if is_dark() { Color32::from_rgb(0x5B, 0x8F, 0xF0) } else { Color32::from_rgb(0x1F, 0x4E, 0xB4) });
            let center = at(8.0, 8.0);
            for (start, end) in [(0.15f32, 0.45f32), (0.65, 0.95)] {
                let arc: Vec<Pos2> = (0..=10)
                    .map(|i| {
                        let t = std::f32::consts::TAU * (start + (end - start) * i as f32 / 10.0);
                        center + vec2(t.cos(), t.sin()) * 5.0 * s
                    })
                    .collect();
                let tip = *arc.last().unwrap();
                let t = std::f32::consts::TAU * end;
                let along = vec2(-t.sin(), t.cos());
                let out = vec2(t.cos(), t.sin());
                p.add(PathShape::line(arc, PathStroke::new(1.6 * s, c)));
                p.add(egui::Shape::convex_polygon(
                    vec![tip + out * 2.5 * s, tip + along * 3.0 * s, tip - out * 2.5 * s],
                    c,
                    Stroke::NONE,
                ));
            }
        }
        Icon::Folders => {
            folder(p, Rect::from_min_size(at(5.0, 5.0), r.size() * 0.7), enabled);
            let st = Stroke::new(1.0, gray(text()));
            p.rect_stroke(Rect::from_min_max(at(1.0, 1.0), at(5.0, 5.0)), 0.0, st, egui::StrokeKind::Inside);
            p.line_segment([at(3.0, 5.0), at(3.0, 10.5)], st);
            p.line_segment([at(3.0, 10.5), at(5.0, 10.5)], st);
        }
        Icon::Undo => {
            // A hooked arrow bending back to the left.
            let c = gray(text());
            if !enabled {
                undo_arrow(p, r, vec2(s, s), highlight());
            }
            undo_arrow(p, r, Vec2::ZERO, c);
        }
        Icon::Views => {
            // A window holding a few large icons, as on the Win2000 button.
            let frame = Rect::from_min_max(at(1.0, 2.0), at(15.0, 14.0));
            p.rect_filled(frame, 0.0, PAPER);
            p.rect_stroke(frame, 0.0, Stroke::new(1.0, gray(dark())), egui::StrokeKind::Inside);
            for (x, y) in [(3.0, 4.0), (9.0, 4.0), (3.0, 9.0), (9.0, 9.0)] {
                p.rect_filled(Rect::from_min_max(at(x, y), at(x + 3.0, y + 3.0)), 0.0, gray(active_caption()));
                p.hline(at(x + 3.5, 0.0).x..=at(x + 5.0, 0.0).x, at(0.0, y + 1.5).y, Stroke::new(1.0, gray(shadow())));
            }
        }
        Icon::Folder | Icon::FolderOpen => folder(p, r, enabled),
        Icon::App => {
            p.rect_filled(Rect::from_min_max(at(1.0, 2.0), at(15.0, 14.0)), 0.0, PAPER);
            p.rect_filled(Rect::from_min_max(at(1.0, 2.0), at(15.0, 5.0)), 0.0, active_caption());
            p.rect_stroke(Rect::from_min_max(at(1.0, 2.0), at(15.0, 14.0)), 0.0, Stroke::new(1.0, dark()), egui::StrokeKind::Inside);
        }
        Icon::Drive => {
            let body = Rect::from_min_max(at(1.0, 5.0), at(15.0, 12.0));
            p.rect_filled(body, 0.0, Color32::from_rgb(0xC0, 0xC0, 0xC0));
            p.rect_filled(Rect::from_min_max(at(1.0, 10.0), at(15.0, 12.0)), 0.0, shadow());
            p.rect_stroke(body, 0.0, Stroke::new(1.0, dark()), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_max(at(11.0, 7.0), at(13.0, 8.5)), 0.0, Color32::from_rgb(0x20, 0xC0, 0x20));
        }
    }
}

/// The Windows 2000 document page: white sheet, gray outline, folded
/// top-right corner. `lines` adds the gray "text" of a plain document;
/// typed documents leave room for an emblem instead.
pub fn paint_page(p: &Painter, r: Rect, lines: bool) {
    let s = r.width() / 16.0;
    let at = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s);
    let sheet: Vec<Pos2> = [(2.5, 0.5), (10.0, 0.5), (13.5, 4.0), (13.5, 15.5), (2.5, 15.5)].iter().map(|&(x, y)| at(x, y)).collect();
    p.add(PathShape::convex_polygon(sheet, PAPER, PathStroke::new(1.0, PAPER_EDGE)));
    // The folded corner: a light gray triangle with an outline.
    p.add(egui::Shape::convex_polygon(
        vec![at(10.0, 0.5), at(13.5, 4.0), at(10.0, 4.0)],
        Color32::from_rgb(0xE4, 0xE4, 0xE4),
        Stroke::new(1.0, PAPER_EDGE),
    ));
    if lines {
        for y in [6.5, 8.5, 10.5, 12.5] {
            p.line_segment([at(4.5, y), at(11.5, y)], Stroke::new(1.0, Color32::from_rgb(0xB0, 0xB0, 0xB0)));
        }
    }
}

/// The emblem area on a [`paint_page`] sheet, in the same coordinates.
pub fn page_emblem_center(r: Rect) -> Pos2 {
    let s = r.width() / 16.0;
    pos2(r.left() + 8.0 * s, r.top() + 9.0 * s)
}

fn arrow_polygon(p: &Painter, pts: &[Pos2], fill: Color32, stroke: Color32) {
    // The arrow is concave, so fill it as a fan from its tip (always convex
    // around the tip) and stroke the outline separately.
    let mut mesh = Mesh::default();
    for &pt in pts {
        mesh.colored_vertex(pt, fill);
    }
    for i in 1..pts.len() as u32 - 1 {
        mesh.add_triangle(0, i, i + 1);
    }
    p.add(mesh);
    p.add(PathShape::closed_line(pts.to_vec(), PathStroke::new(1.0, stroke)));
}

/// The Undo arrow: an arc over the top, its head pointing down-left.
fn undo_arrow(p: &Painter, r: Rect, shift: Vec2, color: Color32) {
    let s = r.width() / 16.0;
    let at = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s) + shift;
    let center = at(8.5, 9.0);
    let arc: Vec<Pos2> = (0..=12)
        .map(|i| {
            // From the right side (0°) up over the top to the left (≈200°).
            let t = -std::f32::consts::PI * 1.1 * i as f32 / 12.0;
            center + vec2(t.cos(), t.sin()) * 5.0 * s
        })
        .collect();
    p.add(PathShape::line(arc, PathStroke::new(1.6 * s, color)));
    p.add(egui::Shape::convex_polygon(vec![at(0.5, 8.0), at(7.0, 8.5), at(3.0, 13.5)], color, Stroke::NONE));
}

fn folder(p: &Painter, r: Rect, enabled: bool) {
    let s = r.width() / 16.0;
    let at = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s);
    let (fill, dark, light) = if enabled { (FOLDER, FOLDER_DARK, FOLDER_LIGHT) } else { (face(), shadow(), highlight()) };
    // Tab, then body.
    p.add(egui::Shape::convex_polygon(
        vec![at(1.5, 3.5), at(6.0, 3.5), at(7.5, 5.0), at(7.5, 6.0), at(1.5, 6.0)],
        fill,
        Stroke::new(1.0, dark),
    ));
    let body = Rect::from_min_max(at(1.5, 5.0), at(14.5, 13.5));
    p.rect_filled(body, 0.0, fill);
    p.rect_stroke(body, 0.0, Stroke::new(1.0, dark), egui::StrokeKind::Middle);
    p.hline(at(2.5, 0.0).x..=at(13.5, 0.0).x, at(0.0, 6.5).y, Stroke::new(1.0, light));
}
