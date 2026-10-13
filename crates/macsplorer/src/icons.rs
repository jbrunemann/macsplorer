//! Picks and paints the 16×16 icon for a list item:
//!
//! - items with their own icon (app bundles, packages, Finder custom icons)
//!   use the icon macOS shows, rendered once and cached as a texture;
//! - folders use the Windows 2000 yellow folder; the ones Finder gives a
//!   sidebar glyph (Music, Pictures, ...) carry that SF Symbol as an emblem;
//! - files are the Windows 2000 dog-eared page; recognized types carry
//!   VSCodium's Seti glyph as a colored emblem, the way Windows 2000 put
//!   an application's emblem on its documents;
//! - unrecognized files are the plain page with gray text lines.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, FontId, Rect, TextureHandle, TextureOptions, Ui};
use macsplorer_core::listing::Entry;
use macsplorer_core::places::{KnownFolder, known_folder};
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

pub struct FileIcons {
    seti: Option<SetiTheme>,
    native: HashMap<PathBuf, Option<TextureHandle>>,
    /// Tintable SF Symbol masks for the known folders, by rendered pixel size.
    symbols: HashMap<(KnownFolder, usize), Option<TextureHandle>>,
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
        Self { seti: seti.ok(), native: HashMap::new(), symbols: HashMap::new(), budget: NATIVE_PER_FRAME }
    }

    pub fn begin_frame(&mut self) {
        self.budget = NATIVE_PER_FRAME;
    }

    /// Forgets rendered native icons (an app update can change its icon).
    pub fn clear_native(&mut self) {
        self.native.clear();
    }

    pub fn paint(&mut self, ui: &Ui, rect: Rect, entry: &Entry) {
        let wants_native = entry.kind == RenameKind::Package || entry.custom_icon;
        if wants_native && self.paint_native(ui, rect, entry) {
            return;
        }
        match entry.kind {
            RenameKind::Folder => self.paint_folder(ui, rect, &entry.path),
            RenameKind::Package => {
                let icon = if entry.extension() == Some("app") { Icon::App } else { Icon::Folder };
                theme::paint_icon(ui.painter(), rect, icon, true);
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

    /// Paints the yellow folder, with Finder's glyph on it for the user
    /// folders that have one.
    pub fn paint_folder(&mut self, ui: &Ui, rect: Rect, path: &Path) {
        theme::paint_icon(ui.painter(), rect, Icon::Folder, true);
        let Some(known) = known_folder(path) else { return };
        let scale = rect.width() / 16.0;
        let px = (FOLDER_EMBLEM_SIZE * scale * ui.ctx().pixels_per_point()).round().max(1.0) as usize;
        let texture = self.symbols.entry((known, px)).or_insert_with(|| {
            native_icon::render_symbol(known.symbol(), px)
                .map(|img| ui.ctx().load_texture(format!("symbol:{}:{px}", known.symbol()), img, TextureOptions::LINEAR))
        });
        if let Some(tex) = texture {
            let center = egui::pos2(rect.left() + FOLDER_EMBLEM_CENTER.0 * scale, rect.top() + FOLDER_EMBLEM_CENTER.1 * scale);
            let target = Rect::from_center_size(center, egui::Vec2::splat(FOLDER_EMBLEM_SIZE * scale));
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(tex.id(), target, uv, theme::color::CAPTION);
        }
    }

    /// Paints the cached native icon. Returns false if it isn't available
    /// (yet), so the caller can draw a fallback this frame.
    fn paint_native(&mut self, ui: &Ui, rect: Rect, entry: &Entry) -> bool {
        if !self.native.contains_key(&entry.path) {
            if self.budget == 0 {
                ui.ctx().request_repaint();
                return false;
            }
            self.budget -= 1;
            let px = (rect.width() * ui.ctx().pixels_per_point()).round().max(16.0) as usize;
            let texture = native_icon::render(&entry.path, px).map(|img| {
                ui.ctx().load_texture(format!("icon:{}", entry.path.display()), img, TextureOptions::LINEAR)
            });
            self.native.insert(entry.path.clone(), texture);
        }
        match self.native.get(&entry.path) {
            Some(Some(tex)) => {
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
                true
            }
            _ => false,
        }
    }
}
