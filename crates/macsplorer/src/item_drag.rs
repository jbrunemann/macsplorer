//! Dragging items inside the window: the drop zones the views register while
//! they draw, what the pointer is over, and the drag image. What a drop does
//! is `macsplorer_core::dragdrop`.
//!
//! The drag image is the dragged items as they were drawn, selected and
//! half transparent, following the pointer: Windows 2000's shell drag image
//! (`CLVDragDropManager::OnCreateDragImage` in comctl32 makes it from the
//! items' icons and labels). The drop target is drawn selected
//! (`LVIS_DROPHILITED`, `TVIS_DROPHILITED`).
//!
//! A target is a folder, or a LocalSend device in the Folders pane
//! (`network`), which the items are sent to.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use egui::{Galley, Pos2, Rect, Ui};

use macsplorer_core::dragdrop::Effect;
use macsplorer_core::listing::Entry;
use macsplorer_core::tree::NodeKey;

use crate::icons::{FileIcons, IconState};
use crate::theme;

/// What a drop target draws as selected while the pointer is over it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mark {
    Item(PathBuf),
    Tree(NodeKey),
    /// A device row in the Folders pane, by its `network` key.
    Device(String),
}

/// Where a drop goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Folder(PathBuf),
    /// A LocalSend device, by its `network` key.
    Device(String),
}

/// A part of the window a drag can be over, in screen points, already cut to
/// what is visible of it.
pub struct DropZone {
    rect: Rect,
    /// Where a drop here goes; `None` refuses it (a file).
    target: Option<Target>,
    mark: Option<Mark>,
    /// A collapsed folder in the Folders pane, which expands when the
    /// pointer rests on it.
    expand: Option<NodeKey>,
}

/// One dragged item as drawn: icon and label box, on screen.
#[derive(Clone)]
pub struct Ghost {
    path: PathBuf,
    icon: Rect,
    label: Rect,
    galley: Arc<Galley>,
    /// A Folders pane row: drawn with the tree's folder icon.
    tree: bool,
}

/// What the views report while drawing one frame.
#[derive(Default)]
pub struct DragFrame {
    zones: Vec<DropZone>,
    drawn: Vec<Ghost>,
    /// Whether to keep what items are drawn: while a press might become a drag.
    collect: bool,
}

impl DragFrame {
    pub fn begin(&mut self, ui: &Ui, dragging: bool) {
        self.zones.clear();
        self.drawn.clear();
        self.collect = !dragging && ui.input(|i| i.pointer.primary_down());
    }

    /// A zone, cut to `clip`; later ones are on top of earlier ones.
    pub fn zone(&mut self, clip: Rect, rect: Rect, dir: Option<PathBuf>, mark: Option<Mark>, expand: Option<NodeKey>) {
        self.push(clip, DropZone { rect, target: dir.map(Target::Folder), mark, expand });
    }

    /// A device row, which the items are sent to.
    pub fn device(&mut self, clip: Rect, rect: Rect, key: &str) {
        self.push(clip, DropZone { rect, target: Some(Target::Device(key.to_owned())), mark: Some(Mark::Device(key.to_owned())), expand: None });
    }

    fn push(&mut self, clip: Rect, zone: DropZone) {
        let rect = zone.rect.intersect(clip);
        if rect.is_positive() {
            self.zones.push(DropZone { rect, ..zone });
        }
    }

    /// An item zone: its folder takes drops, a file refuses them.
    pub fn item(&mut self, clip: Rect, rect: Rect, entry: &Entry) {
        let dir = entry.is_folder().then(|| entry.path.clone());
        let mark = dir.as_ref().map(|d| Mark::Item(d.clone()));
        self.zone(clip, rect, dir, mark, None);
    }

    /// Remembers how an item was drawn, for the drag image.
    pub fn drawn(&mut self, clip: Rect, path: &Path, icon: Rect, label: Rect, galley: impl FnOnce() -> Arc<Galley>, tree: bool) {
        if self.collect && clip.intersects(icon.union(label)) {
            self.drawn.push(Ghost { path: path.to_path_buf(), icon, label, galley: galley(), tree });
        }
    }

    fn at(&self, p: Pos2) -> Option<&DropZone> {
        self.zones.iter().rev().find(|z| z.rect.contains(p))
    }
}

/// Where the pointer is during a drag and what a drop there does.
pub struct Over {
    pub target: Target,
    pub mark: Option<Mark>,
    pub effect: Effect,
}

/// Where the pointer is and what the button and Escape did, from egui or,
/// for a drag from another app, from AppKit.
pub struct Pointer {
    pub pos: Option<Pos2>,
    pub down: bool,
    pub escape: bool,
}

impl Pointer {
    pub fn of(ui: &Ui) -> Self {
        ui.input(|i| Self { pos: i.pointer.latest_pos(), down: i.pointer.primary_down(), escape: i.key_pressed(egui::Key::Escape) })
    }
}

/// A drag of items, from when it starts to the drop.
pub struct ItemDrag {
    /// Comes from another app (or left this window and came back): AppKit
    /// draws the drag image and the cursor.
    pub native: bool,
    pub paths: Vec<PathBuf>,
    /// Where the button went down, which the drag image is drawn relative to.
    grab: Pos2,
    ghosts: Vec<Ghost>,
    pub over: Option<Over>,
    /// The collapsed tree folder the pointer rests on, and since when.
    hover: Option<(NodeKey, f64)>,
}

/// What a frame of a drag comes to.
pub enum Step {
    Dragging,
    /// Expand this tree folder (the pointer rested on it).
    Expand(NodeKey),
    /// The button went up over a target.
    Drop { target: Target, effect: Effect },
    /// Up elsewhere, or Escape.
    Cancel,
}

impl ItemDrag {
    /// Starts dragging `paths`, keeping how those of them on screen were drawn.
    pub fn new(ctx: &egui::Context, paths: Vec<PathBuf>, frame: &DragFrame) -> Self {
        let grab = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
        let ghosts = frame.drawn.iter().filter(|g| paths.contains(&g.path)).cloned().collect();
        Self { native: false, paths, grab, ghosts, over: None, hover: None }
    }

    /// A drag of `paths` from another app.
    pub fn external(paths: Vec<PathBuf>) -> Self {
        Self { native: true, paths, grab: Pos2::ZERO, ghosts: Vec::new(), over: None, hover: None }
    }

    /// Whether this item or tree row is drawn as the drop target.
    pub fn marks(&self, mark: &Mark) -> bool {
        self.over.as_ref().and_then(|o| o.mark.as_ref()) == Some(mark)
    }

    /// Whether the drop target is a Folders pane row.
    pub fn over_tree(&self) -> bool {
        matches!(self.over.as_ref().and_then(|o| o.mark.as_ref()), Some(Mark::Tree(_) | Mark::Device(_)))
    }

    /// One frame: finds the target under the pointer with the zones drawn
    /// this frame and `effect`, which gives a target's drop effect (`None`:
    /// refused).
    pub fn step(&mut self, ui: &Ui, frame: &DragFrame, pointer: Pointer, effect: impl Fn(&[PathBuf], &Target) -> Option<Effect>) -> Step {
        let Pointer { pos, down, escape } = pointer;
        let now = ui.input(|i| i.time);
        if escape {
            return Step::Cancel;
        }
        let zone = pos.and_then(|p| frame.at(p));
        self.over = zone.and_then(|z| {
            let target = z.target.clone()?;
            let effect = effect(&self.paths, &target)?;
            Some(Over { target, mark: z.mark.clone(), effect })
        });
        if !down {
            return match self.over.take() {
                Some(Over { target, effect, .. }) => Step::Drop { target, effect },
                None => Step::Cancel,
            };
        }
        if !self.native {
            ui.ctx().set_cursor_icon(match self.over.as_ref().map(|o| o.effect) {
                None => egui::CursorIcon::NoDrop,
                Some(Effect::Copy) => egui::CursorIcon::Copy,
                Some(Effect::Move) => egui::CursorIcon::Default,
                Some(Effect::Link) => egui::CursorIcon::Alias,
            });
        }
        match zone.and_then(|z| z.expand.clone()) {
            Some(key) => match &self.hover {
                Some((k, since)) if *k == key => {
                    let left = macsplorer_core::dragdrop::HOVER_EXPAND - (now - since);
                    if left <= 0.0 {
                        self.hover = None;
                        return Step::Expand(key);
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left));
                }
                _ => {
                    self.hover = Some((key, now));
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(macsplorer_core::dragdrop::HOVER_EXPAND));
                }
            },
            None => self.hover = None,
        }
        Step::Dragging
    }

    /// Paints the drag image over everything, `entries` giving the items'
    /// icons.
    pub fn paint(&self, ctx: &egui::Context, icons: &mut FileIcons, entries: &[Entry]) {
        let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return };
        let offset = pos - self.grab;
        egui::Area::new(egui::Id::new("drag-image")).order(egui::Order::Tooltip).fixed_pos(Pos2::ZERO).interactable(false).show(ctx, |ui| {
            ui.multiply_opacity(0.5);
            for g in &self.ghosts {
                let icon = g.icon.translate(offset);
                match entries.iter().find(|e| e.path == g.path).filter(|_| !g.tree) {
                    Some(entry) => icons.paint(ui, icon, entry, IconState::Selected),
                    None => icons.paint_tree_folder(ui, icon, &g.path),
                }
                let label = g.label.translate(offset);
                ui.painter().rect_filled(label, 0.0, theme::color::selection());
                let x = if g.galley.job.halign == egui::Align::Center { label.center().x } else { label.left() + 2.0 };
                let y = label.top() + (label.height() - g.galley.size().y) / 2.0;
                ui.painter().galley(egui::pos2(x, y), g.galley.clone(), theme::color::selection_text());
            }
        });
    }
}

/// While dragging with the pointer inside `shown` and within a `line` of its
/// top or bottom edge (`axis` 1) or left or right one (`axis` 0), scrolls
/// toward that edge, as long as `content` goes on past it. Guess: shell32's
/// `DAD_AutoScroll` is an empty stub in build 10240, so the margin and the
/// speed (as `edge_scroll`'s, 20 lines a second) aren't traced.
pub fn edge_scroll(ui: &Ui, shown: Rect, content: Rect, line: f32, axis: usize) {
    let (pointer, dt) = ui.input(|i| (i.pointer.latest_pos(), i.stable_dt.min(0.1)));
    let Some(p) = pointer.filter(|p| shown.contains(*p)) else { return };
    let dir = if p[axis] >= shown.max[axis] - line && content.max[axis] > shown.max[axis] + 0.5 {
        -1.0
    } else if p[axis] < shown.min[axis] + line && content.min[axis] < shown.min[axis] - 0.5 {
        1.0
    } else {
        return;
    };
    let mut delta = egui::Vec2::ZERO;
    delta[axis] = dir * 20.0 * line * dt;
    ui.scroll_with_delta_animation(delta, egui::style::ScrollAnimation::none());
    ui.ctx().request_repaint();
}
