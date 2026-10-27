//! Windows Classic menu rows for egui: gutters for check marks and submenu
//! arrows, and a highlight that covers the whole row. See the README.

use egui::{Atom, Atoms, Button, Color32, Id, Painter, Pos2, Response, RichText, Ui, pos2, vec2};

/// Width of the gutters on both sides of the labels, in points. It is the
/// width of the check mark and of the submenu arrow.
pub const GUTTER: f32 = 7.0;

/// The distance between the highlighted row and the menu's border, in points.
/// The gap beside a mark is the button padding plus this, so the mark sits
/// centered between the border and the label.
pub const EDGE: f32 = 2.0;

const MARK_GUTTER: &str = "classic_menu_mark_gutter";
const ARROW_GUTTER: &str = "classic_menu_arrow_gutter";

/// egui's menu style with roomier items: its own horizontal padding is 2 pt,
/// which leaves the last letter of a label touching the edge of the menu.
pub fn style(style: &mut egui::Style) {
    egui::containers::menu::menu_style(style);
    style.spacing.button_padding.x = 5.0;
}

/// A 1 pt square at `pos` in a solid color. egui turns small filled rects into
/// anti-aliased line segments, which fade away at 1x scale, so this adds an
/// unfeathered mesh quad snapped to the pixel grid instead.
pub fn dot(p: &Painter, pos: Pos2, color: Color32) {
    let ppp = p.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let min = pos2(snap(pos.x), snap(pos.y));
    let max = pos2(snap(pos.x + 1.0).max(min.x + 1.0 / ppp), snap(pos.y + 1.0).max(min.y + 1.0 / ppp));
    let mut mesh = egui::Mesh::default();
    mesh.add_colored_rect(egui::Rect::from_min_max(min, max), color);
    p.add(egui::Shape::mesh(mesh));
}

/// A label between the two gutters. A `shortcut` goes left of the right
/// gutter, in the weak text color.
pub fn gutter_atoms(label: impl Into<RichText>, shortcut: Option<RichText>) -> Atoms<'static> {
    let mut atoms = Atoms::new(label.into());
    atoms.push_left(Atom::custom(Id::new(MARK_GUTTER), vec2(GUTTER, 13.0)));
    atoms.push_right(Atom::grow());
    if let Some(keys) = shortcut {
        atoms.push_right(keys.weak());
    }
    atoms.push_right(Atom::custom(Id::new(ARROW_GUTTER), vec2(GUTTER, 13.0)));
    atoms
}

/// The gap between the atoms of a row: as wide as the space from the menu's
/// border to the mark.
pub fn gap(ui: &Ui) -> f32 {
    ui.spacing().button_padding.x + EDGE
}

/// A plain menu row with its label behind the left gutter, so it lines up
/// with the marked rows. Add it with `ui.add`.
pub fn item(ui: &Ui, label: impl Into<RichText>, shortcut: Option<RichText>) -> Button<'static> {
    Button::new(gutter_atoms(label, shortcut)).gap(gap(ui))
}

/// A check mark row. Returns the response, `changed()` when it was toggled.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, label: impl Into<RichText>) -> Response {
    let mut response = mark_row(ui, label, *checked, false);
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response
}

/// A radio row: a bullet when `selected`.
pub fn radio(ui: &mut Ui, selected: bool, label: impl Into<RichText>) -> Response {
    mark_row(ui, label, selected, true)
}

/// A row that opens a submenu, with the arrow in the right gutter. `open` when
/// its submenu is showing, which keeps the arrow white with the highlight.
pub fn submenu_row(ui: &mut Ui, label: impl Into<RichText>, open: bool) -> Response {
    let out = Button::new(gutter_atoms(label, None)).gap(gap(ui)).atom_ui(ui);
    let hot = open || is_hot(&out.response);
    if let Some(rect) = out.rect(Id::new(ARROW_GUTTER)) {
        let color = row_color(ui, hot);
        let origin = pos2(rect.center().x.round() - 2.0, rect.center().y.round() - 3.0);
        // A triangle 4 pixels wide and 7 tall, one column at a time.
        for (x, height) in [7.0, 5.0, 3.0, 1.0].into_iter().enumerate() {
            for y in 0..height as usize {
                dot(ui.painter(), origin + vec2(x as f32, (7.0 - height) / 2.0 + y as f32), color);
            }
        }
    }
    out.response
}

fn mark_row(ui: &mut Ui, label: impl Into<RichText>, marked: bool, radio: bool) -> Response {
    let out = Button::new(gutter_atoms(label, None)).gap(gap(ui)).atom_ui(ui);
    if let (true, Some(rect)) = (marked, out.rect(Id::new(MARK_GUTTER))) {
        let color = row_color(ui, is_hot(&out.response));
        let p = ui.painter();
        if radio {
            p.circle_filled(rect.center(), 3.0, color);
        } else {
            // Each column of the check mark is three pixels tall.
            const TOPS: [f32; 7] = [2.0, 3.0, 4.0, 3.0, 2.0, 1.0, 0.0];
            for (x, top) in TOPS.iter().enumerate() {
                for y in 0..3 {
                    dot(p, rect.min + vec2(x as f32, 3.0 + top + y as f32), color);
                }
            }
        }
    }
    out.response
}

fn is_hot(response: &Response) -> bool {
    response.hovered() || response.is_pointer_button_down_on()
}

/// The text color of a row: the highlighted one while it is hot.
fn row_color(ui: &Ui, hot: bool) -> Color32 {
    let widgets = &ui.visuals().widgets;
    if hot { widgets.hovered.fg_stroke.color } else { widgets.inactive.fg_stroke.color }
}
