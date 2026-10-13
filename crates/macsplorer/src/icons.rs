//! Picks and paints the 16×16 icon for a list item:
//!
//! - items with their own icon (app bundles, packages, Finder custom icons)
//!   use the icon macOS shows, rendered once and cached as a texture;
//! - folders use the Windows 2000 yellow folder;
//! - files are the Windows 2000 dog-eared page; recognized types carry
//!   VSCodium's Seti glyph as a colored emblem, the way Windows 2000 put
//!   an application's emblem on its documents;
//! - unrecognized files are the plain page with gray text lines.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, Align2, Color32, FontId, Rect, TextureHandle, TextureOptions, Ui};
use macsplorer_core::listing::Entry;
use macsplorer_core::rename::RenameKind;
use macsplorer_core::seti::SetiTheme;

use crate::native_icon;
use crate::theme::{self, Icon};

/// Native icons rendered per frame; the rest wait for following frames so
/// opening a folder full of apps never stalls a frame.
const NATIVE_PER_FRAME: usize = 8;
/// Emblem size on the 16 px page (the glyphs carry their own padding).
const EMBLEM_SIZE: f32 = 15.0;

pub struct FileIcons {
    seti: Option<SetiTheme>,
    native: HashMap<PathBuf, Option<TextureHandle>>,
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
        Self { seti: seti.ok(), native: HashMap::new(), budget: NATIVE_PER_FRAME }
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
            RenameKind::Folder => theme::paint_icon(ui.painter(), rect, Icon::Folder, true),
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
