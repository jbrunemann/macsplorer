//! Windows 2000 "Windows Classic" look: system colors, 3D bevels, Tahoma,
//! title bar, toolbar buttons and 16×16 icons, all drawn with egui.

use std::sync::Arc;

use eframe::egui;
use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{
    Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect, Response,
    Sense, Shadow, Stroke, TextStyle, Ui, Vec2, ViewportCommand, pos2, vec2,
};

use crate::platform;

/// Windows Classic system colors (`GetSysColor` defaults on Windows 2000).
pub mod color {
    use super::Color32;

    pub const FACE: Color32 = Color32::from_rgb(0xD4, 0xD0, 0xC8); // COLOR_3DFACE
    pub const HIGHLIGHT: Color32 = Color32::WHITE; // COLOR_3DHILIGHT
    pub const LIGHT: Color32 = Color32::from_rgb(0xD4, 0xD0, 0xC8); // COLOR_3DLIGHT
    pub const SHADOW: Color32 = Color32::from_rgb(0x80, 0x80, 0x80); // COLOR_3DSHADOW
    pub const DARK: Color32 = Color32::from_rgb(0x40, 0x40, 0x40); // COLOR_3DDKSHADOW
    pub const WINDOW: Color32 = Color32::WHITE;
    pub const TEXT: Color32 = Color32::BLACK;
    pub const GRAY_TEXT: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
    pub const SELECTION: Color32 = Color32::from_rgb(0x0A, 0x24, 0x6A); // COLOR_HIGHLIGHT
    pub const SELECTION_TEXT: Color32 = Color32::WHITE;
    pub const CAPTION: Color32 = Color32::from_rgb(0x0A, 0x24, 0x6A);
    pub const CAPTION_GRADIENT: Color32 = Color32::from_rgb(0xA6, 0xCA, 0xF0);
    pub const CAPTION_INACTIVE: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
    pub const CAPTION_INACTIVE_GRADIENT: Color32 = Color32::from_rgb(0xC0, 0xC0, 0xC0);
    pub const CAPTION_INACTIVE_TEXT: Color32 = Color32::from_rgb(0xD4, 0xD0, 0xC8);
    pub const TREE_LINE: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);

    pub const FOLDER: Color32 = Color32::from_rgb(0xF5, 0xD4, 0x6B);
    pub const FOLDER_DARK: Color32 = Color32::from_rgb(0x9C, 0x7A, 0x1E);
    pub const FOLDER_LIGHT: Color32 = Color32::from_rgb(0xFF, 0xF1, 0xB8);
    pub const ARROW: Color32 = Color32::from_rgb(0x2E, 0x8B, 0x2E);
    pub const ARROW_DARK: Color32 = Color32::from_rgb(0x14, 0x50, 0x14);
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

pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}

/// Installs fonts and visuals. Tahoma is read from the system fonts at
/// runtime (not bundled, see `platform::TAHOMA`); egui's default font is the
/// fallback.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut bold_fonts = Vec::new();
    for (key, paths, family) in [("tahoma", platform::TAHOMA, FontFamily::Proportional), ("tahoma-bold", platform::TAHOMA_BOLD, bold_family())] {
        if let Some(bytes) = paths.iter().find_map(|path| std::fs::read(path).ok()) {
            fonts.font_data.insert(key.into(), Arc::new(FontData::from_owned(bytes)));
            if family == FontFamily::Proportional {
                fonts.families.entry(family).or_default().insert(0, key.into());
            } else {
                bold_fonts.push(key.to_string());
            }
        }
    }
    // Bold falls back to the regular stack (Tahoma, then egui defaults).
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

        let v = &mut style.visuals;
        *v = egui::Visuals::light();
        v.panel_fill = FACE;
        v.window_fill = FACE;
        v.window_stroke = Stroke::new(1.0, SHADOW);
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow { offset: [2, 2], blur: 0, spread: 0, color: Color32::from_black_alpha(90) };
        v.window_corner_radius = CornerRadius::ZERO;
        v.menu_corner_radius = CornerRadius::ZERO;
        v.extreme_bg_color = WINDOW;
        v.text_edit_bg_color = Some(WINDOW);
        v.faint_bg_color = WINDOW;
        v.striped = false;
        v.hyperlink_color = SELECTION;
        v.override_text_color = None;
        v.selection.bg_fill = SELECTION;
        v.selection.stroke = Stroke::new(1.0, SELECTION_TEXT);
        v.interact_cursor = None;
        v.text_cursor.stroke = Stroke::new(1.0, TEXT);

        let w = &mut v.widgets;
        for wv in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            wv.corner_radius = CornerRadius::ZERO;
            wv.expansion = 0.0;
            wv.bg_fill = FACE;
            wv.weak_bg_fill = FACE;
            wv.fg_stroke = Stroke::new(1.0, TEXT);
            wv.bg_stroke = Stroke::new(1.0, SHADOW);
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, SHADOW);
        // Menu items and other egui-drawn hovers use the classic navy highlight.
        for wv in [&mut w.hovered, &mut w.active, &mut w.open] {
            wv.bg_fill = SELECTION;
            wv.weak_bg_fill = SELECTION;
            wv.fg_stroke = Stroke::new(1.0, SELECTION_TEXT);
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
        Bevel::Raised => (HIGHLIGHT, DARK, Some((LIGHT, SHADOW))),
        Bevel::Sunken => (SHADOW, HIGHLIGHT, Some((DARK, LIGHT))),
        Bevel::RaisedThin => (HIGHLIGHT, SHADOW, None),
        Bevel::SunkenThin => (SHADOW, HIGHLIGHT, None),
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

/// A dotted outline just inside `r`, like `DrawFocusRect`: 1 pt dots on
/// alternate points of a checkerboard, so the pattern holds still while the
/// rectangle moves. Windows XORs the dots; black is the closest egui can do.
pub fn focus_rect(p: &Painter, r: Rect) {
    let (x0, y0) = (r.left().round(), r.top().round());
    let (x1, y1) = (r.right().round() - 1.0, r.bottom().round() - 1.0);
    if x1 < x0 || y1 < y0 {
        return;
    }
    let dot = |x: f32, y: f32| {
        if (x + y).rem_euclid(2.0) == 0.0 {
            p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(1.0, 1.0)), 0.0, TEXT);
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

/// The etched groove between rebar bands and menu sections.
pub fn etched_hline(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
    let p = ui.painter();
    p.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, SHADOW));
    p.hline(r.x_range(), r.top() + 1.5, Stroke::new(1.0, HIGHLIGHT));
}

pub fn etched_vline(ui: &mut Ui, height: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(6.0, height), Sense::hover());
    let x = r.center().x.floor();
    let p = ui.painter();
    p.vline(x + 0.5, r.y_range(), Stroke::new(1.0, SHADOW));
    p.vline(x + 1.5, r.y_range(), Stroke::new(1.0, HIGHLIGHT));
}

/// Paints a white sunken field into `rect` and returns the area inside the bevel.
pub fn sunken_field(p: &Painter, rect: Rect) -> Rect {
    p.rect_filled(rect, 0.0, WINDOW);
    bevel(p, rect, Bevel::Sunken);
    rect.shrink(2.0)
}

// ---- buttons -----------------------------------------------------------------

/// A flat toolbar button: raised when hot, sunken when pressed (IE5 coolbar).
pub fn tool_button(ui: &mut Ui, icon: Icon, label: Option<&str>, enabled: bool) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let galley = label.map(|l| ui.painter().layout_no_wrap(l.into(), font.clone(), if enabled { TEXT } else { GRAY_TEXT }));
    // Disabled text is embossed: a white copy one pixel down-right.
    let emboss = label.filter(|_| !enabled).map(|l| ui.painter().layout_no_wrap(l.into(), font, HIGHLIGHT));
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
            p.galley(pos + vec2(1.0, 1.0), e, HIGHLIGHT);
        }
        p.galley(pos, g, TEXT);
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
    ui.painter().set(bg, egui::Shape::rect_filled(resp.rect, 0.0, Color32::from_rgb(0xEA, 0xE8, 0xE3)));
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
    let color = if enabled { TEXT } else { GRAY_TEXT };
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
        TEXT,
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
    p.rect_filled(rect, 0.0, FACE);
    let down = enabled && resp.is_pointer_button_down_on() && resp.hovered();
    let inner = if default {
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, DARK), egui::StrokeKind::Inside);
        rect.shrink(1.0)
    } else {
        rect
    };
    if down {
        p.rect_stroke(inner, 0.0, Stroke::new(1.0, DARK), egui::StrokeKind::Inside);
        edge(p, inner.shrink(1.0), SHADOW, FACE);
    } else {
        bevel(p, inner, Bevel::Raised);
    }
    let center = rect.center() + if down { vec2(1.0, 1.0) } else { Vec2::ZERO };
    let font = FontId::new(FONT_SIZE, FontFamily::Proportional);
    if enabled {
        p.text(center, Align2::CENTER_CENTER, label, font, TEXT);
    } else {
        p.text(center + vec2(1.0, 1.0), Align2::CENTER_CENTER, label, font.clone(), HIGHLIGHT);
        p.text(center, Align2::CENTER_CENTER, label, font, SHADOW);
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
    let s = Stroke::new(1.0, TEXT);
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
        (CAPTION, CAPTION_GRADIENT, Color32::WHITE)
    } else {
        (CAPTION_INACTIVE, CAPTION_INACTIVE_GRADIENT, CAPTION_INACTIVE_TEXT)
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

    // On macOS, not Windows' system menu: a click on the red dot or the folder closes, matching
    // the macOS close button in the top left.
    let icon_left = if cfg!(target_os = "macos") {
        let dot = pos2(rect.left() + 8.0, rect.center().y);
        p.circle(dot, 5.0, Color32::from_rgb(0xff, 0x5f, 0x57), Stroke::new(1.0, Color32::from_rgb(0xc0, 0x30, 0x28)));
        16.0
    } else {
        2.0
    };
    let icon = Rect::from_min_size(pos2(rect.left() + icon_left, rect.top() + 1.0), vec2(if dialog { 0.0 } else { 16.0 }, 16.0));
    if !dialog {
        paint_icon(p, icon, Icon::FolderOpen, true);
    }
    if cfg!(target_os = "macos") {
        let close_area = Rect::from_min_max(pos2(rect.left(), rect.top()), pos2(icon.right(), rect.bottom()));
        if ui.interact(close_area, ui.id().with("caption_icon"), Sense::click()).clicked() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
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

#[derive(Clone, Copy)]
enum Caption {
    Minimize,
    Maximize,
    Restore,
    Close,
}

fn caption_button(p: &Painter, r: Rect, kind: Caption, down: bool) {
    p.rect_filled(r, 0.0, FACE);
    if down {
        p.rect_stroke(r, 0.0, Stroke::new(1.0, DARK), egui::StrokeKind::Inside);
        edge(p, r.shrink(1.0), SHADOW, FACE);
    } else {
        bevel(p, r, Bevel::Raised);
    }
    let o = if down { vec2(1.0, 1.0) } else { Vec2::ZERO };
    let g = r.shrink2(vec2(4.0, 3.0)).translate(o);
    let s = Stroke::new(1.0, TEXT);
    match kind {
        Caption::Minimize => {
            p.rect_filled(Rect::from_min_max(pos2(g.left(), g.bottom() - 2.0), pos2(g.left() + 6.0, g.bottom())), 0.0, TEXT);
        }
        Caption::Maximize => {
            p.rect_stroke(g, 0.0, s, egui::StrokeKind::Inside);
            p.hline(g.x_range(), g.top() + 1.5, s);
        }
        Caption::Restore => {
            let back = Rect::from_min_size(g.left_top() + vec2(2.0, 0.0), vec2(6.0, 5.0));
            let front = Rect::from_min_size(g.left_top() + vec2(0.0, 3.0), vec2(6.0, 5.0));
            p.rect_stroke(back, 0.0, s, egui::StrokeKind::Inside);
            p.rect_filled(front, 0.0, FACE);
            p.rect_stroke(front, 0.0, s, egui::StrokeKind::Inside);
            p.hline(front.x_range(), front.top() + 1.5, s);
        }
        Caption::Close => {
            let c = g.shrink2(vec2(0.5, 0.0));
            let s = Stroke::new(1.6, TEXT);
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
    let gray = |c: Color32| if enabled { c } else { SHADOW };
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
                arrow_polygon(p, &emboss.iter().map(|&(x, y)| at(x, y)).collect::<Vec<_>>(), HIGHLIGHT, HIGHLIGHT);
            }
            arrow_polygon(p, &pts.iter().map(|&(x, y)| at(x, y)).collect::<Vec<_>>(), gray(ARROW), gray(ARROW_DARK));
        }
        Icon::Up => {
            folder(p, r, enabled);
            let c = gray(TEXT);
            poly(&[(8.0, 4.0), (12.0, 8.0), (4.0, 8.0)], c, c);
            p.rect_filled(Rect::from_min_max(at(7.0, 8.0), at(9.0, 12.0)), 0.0, c);
        }
        Icon::Refresh => {
            let c = gray(Color32::from_rgb(0x1F, 0x4E, 0xB4));
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
            let st = Stroke::new(1.0, gray(TEXT));
            p.rect_stroke(Rect::from_min_max(at(1.0, 1.0), at(5.0, 5.0)), 0.0, st, egui::StrokeKind::Inside);
            p.line_segment([at(3.0, 5.0), at(3.0, 10.5)], st);
            p.line_segment([at(3.0, 10.5), at(5.0, 10.5)], st);
        }
        Icon::Undo => {
            // A hooked arrow bending back to the left.
            let c = gray(TEXT);
            if !enabled {
                undo_arrow(p, r, vec2(s, s), HIGHLIGHT);
            }
            undo_arrow(p, r, Vec2::ZERO, c);
        }
        Icon::Views => {
            // A window holding a few large icons, as on the Win2000 button.
            let frame = Rect::from_min_max(at(1.0, 2.0), at(15.0, 14.0));
            p.rect_filled(frame, 0.0, WINDOW);
            p.rect_stroke(frame, 0.0, Stroke::new(1.0, gray(DARK)), egui::StrokeKind::Inside);
            for (x, y) in [(3.0, 4.0), (9.0, 4.0), (3.0, 9.0), (9.0, 9.0)] {
                p.rect_filled(Rect::from_min_max(at(x, y), at(x + 3.0, y + 3.0)), 0.0, gray(CAPTION));
                p.hline(at(x + 3.5, 0.0).x..=at(x + 5.0, 0.0).x, at(0.0, y + 1.5).y, Stroke::new(1.0, gray(SHADOW)));
            }
        }
        Icon::Folder | Icon::FolderOpen => folder(p, r, enabled),
        Icon::App => {
            p.rect_filled(Rect::from_min_max(at(1.0, 2.0), at(15.0, 14.0)), 0.0, WINDOW);
            p.rect_filled(Rect::from_min_max(at(1.0, 2.0), at(15.0, 5.0)), 0.0, CAPTION);
            p.rect_stroke(Rect::from_min_max(at(1.0, 2.0), at(15.0, 14.0)), 0.0, Stroke::new(1.0, DARK), egui::StrokeKind::Inside);
        }
        Icon::Drive => {
            let body = Rect::from_min_max(at(1.0, 5.0), at(15.0, 12.0));
            p.rect_filled(body, 0.0, Color32::from_rgb(0xC0, 0xC0, 0xC0));
            p.rect_filled(Rect::from_min_max(at(1.0, 10.0), at(15.0, 12.0)), 0.0, SHADOW);
            p.rect_stroke(body, 0.0, Stroke::new(1.0, DARK), egui::StrokeKind::Inside);
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
    p.add(PathShape::convex_polygon(sheet, WINDOW, PathStroke::new(1.0, SHADOW)));
    // The folded corner: a light gray triangle with an outline.
    p.add(egui::Shape::convex_polygon(
        vec![at(10.0, 0.5), at(13.5, 4.0), at(10.0, 4.0)],
        Color32::from_rgb(0xE4, 0xE4, 0xE4),
        Stroke::new(1.0, SHADOW),
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
    let (fill, dark, light) = if enabled { (FOLDER, FOLDER_DARK, FOLDER_LIGHT) } else { (FACE, SHADOW, HIGHLIGHT) };
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
