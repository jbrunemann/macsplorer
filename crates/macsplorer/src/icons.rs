//! Picks and paints the 16×16 icon for a list item:
//!
//! - items with their own icon (app bundles, packages, Finder custom icons)
//!   use the icon macOS shows, rendered once and cached as a texture;
//! - folders use the Windows 2000 yellow folder; the ones Finder gives a
//!   sidebar glyph (Music, Pictures, ...) carry that SF Symbol as an emblem;
//! - zip and tar files, which open as folders, have a zipper: on the folder
//!   when large, on the page in the List and Details views;
//! - files are the Windows 2000 dog-eared page; recognized types carry
//!   VSCodium's Seti glyph as a colored emblem, the way Windows 2000 put
//!   an application's emblem on its documents;
//! - unrecognized files are the plain page with gray text lines.
//!
//! Selected and cut items are blended toward a color, as the list view's
//! `CLVImageListManager::DrawImageEx2` does (`docs/file-browser-findings.md` §2.5).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, ColorImage, FontId, Rect, Shape, TextureHandle, TextureOptions, Ui};
use egui::epaint::ColorMode;
use egui::layers::ShapeIdx;
use macsplorer_core::archive;
use macsplorer_core::folder_style::Emblem;
use macsplorer_core::listing::{Entry, is_hidden_name};
use macsplorer_core::places::known_folder;
use macsplorer_core::rename::RenameKind;
use macsplorer_core::seti::SetiTheme;

use crate::native_icon;
use crate::theme::{self, Icon};

/// Native icons rendered per frame; the rest wait for following frames so
/// opening a folder full of apps never stalls a frame.
const NATIVE_PER_FRAME: usize = 8;
/// Emblem size on the 16 px page (the glyphs carry their own padding).
const EMBLEM_SIZE: f32 = 15.0;
/// Folder emblem box on the 16 px folder: the body below the tab.
const FOLDER_EMBLEM_SIZE: f32 = 8.0;
const FOLDER_EMBLEM_CENTER: (f32, f32) = (8.0, 9.7);

/// How an item's icon is drawn for its state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum IconState {
    #[default]
    Normal,
    /// Selected while the view has focus: `ILD_BLEND50` toward `COLOR_HIGHLIGHT`.
    Selected,
    /// Selected while the view doesn't have focus: `ILD_BLEND25`.
    SelectedInactive,
    /// Cut (`LVIS_CUT`) or hidden (`SFGAO_GHOSTED`, which the shell reports
    /// as `LVIS_CUT` too): `ILD_BLEND50` toward the list background, ghosted.
    Cut,
}

impl IconState {
    pub fn of(selected: bool, focused: bool, ghosted: bool) -> Self {
        match (ghosted, selected, focused) {
            (true, _, _) => Self::Cut,
            (_, true, true) => Self::Selected,
            (_, true, false) => Self::SelectedInactive,
            _ => Self::Normal,
        }
    }

    /// The color to blend toward and how far.
    fn blend(self) -> Option<(Color32, f32)> {
        match self {
            Self::Normal => None,
            Self::Selected => Some((theme::color::selection(), 0.5)),
            Self::SelectedInactive => Some((theme::color::selection(), 0.25)),
            Self::Cut => Some((theme::color::window(), 0.5)),
        }
    }
}

/// Runs `paint` and blends what it drew as `state` says.
fn blended(ui: &Ui, state: IconState, paint: impl FnOnce()) {
    let layer = ui.painter().layer_id();
    let start = ui.ctx().graphics_mut(|g| g.entry(layer).next_idx());
    paint();
    if let Some((toward, t)) = state.blend() {
        ui.ctx().graphics_mut(|g| {
            let list = g.entry(layer);
            for i in start.0..list.next_idx().0 {
                list.mutate_shape(ShapeIdx(i), |s| blend_shape(&mut s.shape, toward, t));
            }
        });
    }
}

/// A rendered native icon and its blended variants.
struct Native {
    image: Arc<ColorImage>,
    textures: HashMap<IconState, TextureHandle>,
}

pub struct FileIcons {
    seti: Option<SetiTheme>,
    /// By path and pixel size.
    native: HashMap<(PathBuf, usize), Option<Native>>,
    /// The pictures drawn on folders (SF Symbols as tintable masks, emoji in
    /// color), by rendered pixel size.
    emblems: HashMap<(Emblem, usize), Option<TextureHandle>>,
    /// The emblems customized favorites carry in the Folders pane, by path.
    favorite_emblems: HashMap<PathBuf, Emblem>,
    /// The 16 px icons of the apps in the "Open With" menu, by path and pixel size.
    menu: HashMap<(PathBuf, usize), Option<TextureHandle>>,
    budget: usize,
}

impl FileIcons {
    pub fn new() -> Self {
        let seti = SetiTheme::parse(
            include_str!("../../../assets/seti/vs-seti-icon-theme.json"),
            include_str!("../../../assets/seti/languages.json"),
        );
        if let Err(e) = &seti {
            eprintln!("macsplorer: Seti icon theme unavailable: {e}");
        }
        Self { seti: seti.ok(), native: HashMap::new(), emblems: HashMap::new(), favorite_emblems: HashMap::new(), menu: HashMap::new(), budget: NATIVE_PER_FRAME }
    }

    /// The selection tint depends on the palette: drop the blended variants.
    pub fn palette_changed(&mut self) {
        for native in self.native.values_mut().flatten() {
            native.textures.clear();
        }
    }

    pub fn begin_frame(&mut self) {
        self.budget = NATIVE_PER_FRAME;
    }

    /// Forgets rendered native icons (an app update can change its icon).
    pub fn clear_native(&mut self) {
        self.native.clear();
        self.menu.clear();
    }

    /// Forgets the rendered icon of `path` (its folder style changed).
    pub fn forget_native(&mut self, path: &Path) {
        self.native.retain(|(p, _), _| p != path);
    }

    /// The Finder icon of the app at `app`, 16 points square, for a menu
    /// entry. Rendered on first use, `None` if the system has none.
    pub fn menu_icon(&mut self, ctx: &egui::Context, app: &Path) -> Option<egui::TextureId> {
        let px = (16.0 * ctx.pixels_per_point()).round() as usize;
        self.menu
            .entry((app.to_path_buf(), px))
            .or_insert_with(|| {
                let img = native_icon::render(app, px)?;
                Some(ctx.load_texture(format!("menu-icon:{}:{px}", app.display()), img, TextureOptions::LINEAR))
            })
            .as_ref()
            .map(TextureHandle::id)
    }

    /// A small icon (List, Details, dialogs' rows): a customized folder
    /// shows its color but not its emblem.
    pub fn paint(&mut self, ui: &Ui, rect: Rect, entry: &Entry, state: IconState) {
        self.paint_sized(ui, rect, entry, state, false);
    }

    /// A large icon (Large Icons, Properties): a customized folder shows its
    /// color and its emblem.
    pub fn paint_large(&mut self, ui: &Ui, rect: Rect, entry: &Entry, state: IconState) {
        self.paint_sized(ui, rect, entry, state, true);
    }

    fn paint_sized(&mut self, ui: &Ui, rect: Rect, entry: &Entry, state: IconState, large: bool) {
        // A customized folder: the Win2000 folder in its color, with the
        // emblem only when large. Pasted custom icons (no emblem, but the
        // custom icon flag) still take the native icon below.
        if let Some(style) = entry.folder_style.as_ref().filter(|s| entry.kind == RenameKind::Folder && (s.emblem.is_some() || !entry.custom_icon)) {
            let fill = style.color.map(|c| {
                let [r, g, b] = c.rgb();
                Color32::from_rgb(r, g, b)
            });
            let emblem = style.emblem.as_ref().filter(|_| large);
            blended(ui, state, || self.paint_folder(ui, rect, &entry.path, fill, emblem));
            return;
        }
        // An app in an archive isn't on disk for macOS to draw.
        let wants_native = (entry.kind == RenameKind::Package && !archive::is_inside(&entry.path)) || entry.custom_icon;
        if wants_native && self.paint_native(ui, rect, entry, state) {
            return;
        }
        blended(ui, state, || self.paint_vector(ui, rect, entry, large));
    }

    fn paint_vector(&mut self, ui: &Ui, rect: Rect, entry: &Entry, large: bool) {
        match entry.kind {
            RenameKind::Folder => self.paint_folder(ui, rect, &entry.path, None, None),
            RenameKind::Package => {
                let icon = if entry.extension() == Some("app") { Icon::App } else { Icon::Folder };
                theme::paint_icon(ui.painter(), rect, icon, true);
            }
            // An archive inside an archive is just a file there (`archive::split`).
            RenameKind::File if archive::format_of(&entry.name).is_some_and(archive::Format::browsable) && !archive::is_inside(&entry.path) => {
                theme::paint_icon(ui.painter(), rect, if large { Icon::CompressedFolder } else { Icon::CompressedFile }, true);
            }
            RenameKind::File => match self.seti.as_ref().and_then(|s| s.specific_icon_for(&entry.name, true)) {
                Some(g) => {
                    theme::paint_page(ui.painter(), rect, false);
                    let [r, g2, b] = g.rgb;
                    ui.painter().text(
                        theme::page_emblem_center(rect),
                        Align2::CENTER_CENTER,
                        g.ch,
                        FontId::new(EMBLEM_SIZE * rect.width() / 16.0, theme::seti_family()),
                        Color32::from_rgb(r, g2, b),
                    );
                }
                None => theme::paint_page(ui.painter(), rect, true),
            },
        }
    }

    /// The emblems of the customized favorites, read by the caller off the
    /// drawing path.
    pub fn set_favorite_emblems(&mut self, emblems: HashMap<PathBuf, Emblem>) {
        self.favorite_emblems = emblems;
    }

    /// Paints the yellow folder (never the folder's color), with the
    /// folder's own emblem if a favorite has one, else Finder's glyph for the
    /// user folders that have one.
    pub fn paint_tree_folder(&mut self, ui: &Ui, rect: Rect, path: &Path) {
        let hidden = path.file_name().is_some_and(|n| is_hidden_name(&n.to_string_lossy()));
        let emblem = self.favorite_emblems.get(path).cloned();
        blended(ui, if hidden { IconState::Cut } else { IconState::Normal }, || self.paint_folder(ui, rect, path, None, emblem.as_ref()));
    }

    /// The Win2000 folder, yellow or in `fill`, with `emblem` on it or else
    /// Finder's glyph for the user folders that have one.
    fn paint_folder(&mut self, ui: &Ui, rect: Rect, path: &Path, fill: Option<Color32>, emblem: Option<&Emblem>) {
        match fill {
            Some(fill) => theme::paint_colored_folder(ui.painter(), rect, fill),
            None => theme::paint_icon(ui.painter(), rect, Icon::Folder, true),
        }
        let known = known_folder(path).map(|k| Emblem::Symbol(k.symbol().to_owned()));
        let Some(emblem) = emblem.or(known.as_ref()) else { return };
        let scale = rect.width() / 16.0;
        let px = (FOLDER_EMBLEM_SIZE * scale * ui.ctx().pixels_per_point()).round().max(1.0) as usize;
        let texture = self.emblems.entry((emblem.clone(), px)).or_insert_with(|| {
            native_icon::render_emblem(emblem, px).map(|img| ui.ctx().load_texture(format!("emblem:{emblem:?}:{px}"), img, TextureOptions::LINEAR))
        });
        if let Some(tex) = texture {
            let center = egui::pos2(rect.left() + FOLDER_EMBLEM_CENTER.0 * scale, rect.top() + FOLDER_EMBLEM_CENTER.1 * scale);
            let target = Rect::from_center_size(center, egui::Vec2::splat(FOLDER_EMBLEM_SIZE * scale));
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            // A symbol is a mask in the title bar's blue; an emoji has its own colors.
            let tint = if matches!(emblem, Emblem::Symbol(_)) { theme::color::active_caption() } else { Color32::WHITE };
            ui.painter().image(tex.id(), target, uv, tint);
        }
    }

    /// Paints the cached native icon. Returns false if it isn't available
    /// (yet), so the caller can draw a fallback this frame.
    fn paint_native(&mut self, ui: &Ui, rect: Rect, entry: &Entry, state: IconState) -> bool {
        let px = (rect.width() * ui.ctx().pixels_per_point()).round().max(16.0) as usize;
        let key = (entry.path.clone(), px);
        if !self.native.contains_key(&key) {
            if self.budget == 0 {
                ui.ctx().request_repaint();
                return false;
            }
            self.budget -= 1;
            let native = native_icon::render(&entry.path, px).map(|img| Native { image: Arc::new(img), textures: HashMap::new() });
            self.native.insert(key.clone(), native);
        }
        let Some(Some(native)) = self.native.get_mut(&key) else { return false };
        draw_native(ui, rect, native, &format!("icon:{}:{px}", entry.path.display()), state);
        true
    }
}

/// Paints a rendered icon in `state`, making the blended texture on first use.
fn draw_native(ui: &Ui, rect: Rect, native: &mut Native, name: &str, state: IconState) {
    let tex = native.textures.entry(state).or_insert_with(|| {
        let mut img = (*native.image).clone();
        if let Some((toward, t)) = state.blend() {
            for p in &mut img.pixels {
                *p = blend_color(*p, toward, t);
            }
        }
        ui.ctx().load_texture(format!("{name}:{state:?}"), img, TextureOptions::LINEAR)
    });
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
}

/// Draws the Git branch glyph around `center`, as vector shapes so it needs
/// no icon font and works on every OS: three ring commits, a line and a
/// curve, with the layout of Ionicons' `git-branch-outline` (MIT; commit
/// centers on a 512 unit box). The rings and strokes are drawn smaller than
/// in the original, which is too heavy at icon size; `scale` enlarges all
/// pixel sizes, which are those of a 16 px icon cell at scale 1.
pub fn paint_git_branch(painter: &egui::Painter, center: egui::Pos2, scale: f32, color: Color32) {
    /// The commits' vertical distance (units 96..416).
    const SPAN: f32 = 3.6;
    /// Stretches the layout sideways so the curve has room at this size.
    const WIDEN: f32 = 1.5;
    const RING_RADIUS: f32 = 0.7;
    const STROKE_WIDTH: f32 = 0.8;
    let unit = SPAN * scale / 320.0;
    let at = |x: f32, y: f32| egui::pos2(center.x + (x - 256.0) * unit * WIDEN, center.y + (y - 256.0) * unit);
    let stroke = egui::Stroke::new(STROKE_WIDTH * scale, color);
    let ring = RING_RADIUS * scale;
    // Lines start and end at the rings' edges, not their centers.
    let (top, bottom, right) = (at(160.0, 96.0), at(160.0, 416.0), at(352.0, 160.0));
    painter.line_segment([egui::pos2(top.x, top.y + ring), egui::pos2(bottom.x, bottom.y - ring)], stroke);
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
        [egui::pos2(right.x, right.y + ring), at(352.0, 336.0), at(160.0, 256.0), egui::pos2(bottom.x, bottom.y - 2.0 * ring)],
        false,
        Color32::TRANSPARENT,
        stroke,
    ));
    for c in [top, bottom, right] {
        painter.circle_stroke(c, ring, stroke);
    }
}

/// Moves a color's RGB `t` of the way toward `toward`, keeping its alpha, so
/// only the icon's own pixels change (like an image list blend through the
/// icon's mask).
fn blend_color(c: Color32, toward: Color32, t: f32) -> Color32 {
    if c.a() == 0 {
        return c;
    }
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(mix(r, toward.r()), mix(g, toward.g()), mix(b, toward.b()), a)
}

fn blend_shape(shape: &mut Shape, toward: Color32, t: f32) {
    let blend = |c: &mut Color32| *c = blend_color(*c, toward, t);
    match shape {
        Shape::Vec(shapes) => shapes.iter_mut().for_each(|s| blend_shape(s, toward, t)),
        Shape::Circle(s) => {
            blend(&mut s.fill);
            blend(&mut s.stroke.color);
        }
        Shape::Ellipse(s) => {
            blend(&mut s.fill);
            blend(&mut s.stroke.color);
        }
        Shape::LineSegment { stroke, .. } => blend(&mut stroke.color),
        Shape::Path(s) => {
            blend(&mut s.fill);
            if let ColorMode::Solid(c) = &mut s.stroke.color {
                blend(c);
            }
        }
        Shape::Rect(s) => {
            blend(&mut s.fill);
            blend(&mut s.stroke.color);
        }
        Shape::Text(s) => {
            let mut c = s.override_text_color.unwrap_or(s.fallback_color);
            blend(&mut c);
            s.override_text_color = Some(c);
        }
        // Vertex colors: plain meshes and tinted masks (the SF Symbol emblems).
        Shape::Mesh(m) => Arc::make_mut(m).vertices.iter_mut().for_each(|v| blend(&mut v.color)),
        Shape::QuadraticBezier(s) => {
            blend(&mut s.fill);
            if let ColorMode::Solid(c) = &mut s.stroke.color {
                blend(c);
            }
        }
        Shape::CubicBezier(s) => {
            blend(&mut s.fill);
            if let ColorMode::Solid(c) = &mut s.stroke.color {
                blend(c);
            }
        }
        Shape::Noop | Shape::Callback(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_keeps_alpha_and_transparency() {
        let navy = theme::color::selection();
        assert_eq!(blend_color(Color32::TRANSPARENT, navy, 0.5), Color32::TRANSPARENT);
        assert_eq!(blend_color(Color32::WHITE, Color32::BLACK, 0.5), Color32::from_rgb(128, 128, 128));
        assert_eq!(blend_color(Color32::WHITE, navy, 0.0), Color32::WHITE);
        let half = Color32::from_rgba_unmultiplied(255, 255, 255, 128);
        assert_eq!(blend_color(half, Color32::BLACK, 1.0).a(), 128);
    }
}
