//! Windows 2000's "Copying..." and "Moving..." dialog for a paste, in its
//! own borderless window (also "Sending..." and "Receiving..." for a
//! LocalSend transfer, see `network`): the flying-paper animation, the item and where it
//! goes, the progress bar, the time left and Cancel. When it shows and
//! closes is traced (`macsplorer_core::copy_progress`); the layout and
//! wording are Windows 2000's from memory, and the animation is drawn with
//! the app's own folder and page icons rather than the AVI.

use std::path::Path;
use std::sync::Arc;

use eframe::egui;
use egui::{Key, Rect, Sense, Ui, pos2, vec2};

use macsplorer_core::copy_progress::{DialogClock, Phase};
use macsplorer_core::fileops::Progress;

use crate::theme::{self, Bevel, Icon};

const WIDTH: f32 = 380.0;
const HEIGHT: f32 = 196.0;
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
const MARGIN: f32 = 12.0;
const ANIMATION: egui::Vec2 = vec2(272.0, 60.0);
/// Seconds between animation frames, about the AVI's rate.
const FRAME_TIME: f64 = 1.0 / 15.0;
/// Seconds a sheet of paper takes from one folder to the other.
const FLIGHT: f64 = 1.2;

/// What the dialog shows of the work going on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub done: u64,
    /// 0 while the work is still being counted.
    pub total: u64,
    /// The item and where it goes, one line each.
    pub lines: [String; 2],
}

/// Work the dialog can follow and cancel: a paste's [`Progress`], or a send.
pub trait Tracked: Send + Sync {
    fn snapshot(&self) -> Snapshot;
    fn cancel(&self);
    fn is_cancelled(&self) -> bool;
}

impl Tracked for Progress {
    fn snapshot(&self) -> Snapshot {
        let view = self.view();
        let lines = match &view.item {
            Some((src, dir)) => [label(src), format!("From '{}' to '{}'", src.parent().map(label).unwrap_or_default(), label(dir))],
            None => Default::default(),
        };
        Snapshot { done: view.done, total: view.total, lines }
    }

    fn cancel(&self) {
        Progress::cancel(self)
    }

    fn is_cancelled(&self) -> bool {
        Progress::is_cancelled(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Copy,
    Move,
    /// To a device on the network: the paper flies to a computer.
    Send,
    /// From a device: the paper flies from a computer into a folder.
    Receive,
}

pub struct ProgressDialog {
    /// Matches the paste's `OpDone`, or the send's.
    pub id: u64,
    pub progress: Arc<dyn Tracked>,
    kind: Kind,
    clock: DialogClock,
    /// The paste worker is done; the dialog closes when the clock allows.
    pub finished: bool,
    viewport: egui::ViewportId,
}

impl ProgressDialog {
    pub fn new(id: u64, progress: Arc<dyn Tracked>, kind: Kind) -> Self {
        let viewport = egui::ViewportId::from_hash_of(("progress", kind, id));
        Self { id, progress, kind, clock: DialogClock::default(), finished: false, viewport }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn size() -> egui::Vec2 {
        vec2(WIDTH, HEIGHT)
    }

    /// Shows the dialog for this frame if it is due. `paused` while the paste
    /// waits for an answer. Returns false once it is closed for good.
    pub fn show(&mut self, ctx: &egui::Context, now: f64, paused: bool) -> bool {
        match self.clock.tick(now, paused, self.finished) {
            Phase::Closed => return false,
            Phase::Hidden => {
                if let Some(wait) = self.clock.wait(now, self.finished) {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
                }
                return true;
            }
            Phase::Shown => {}
        }
        let title = title(self.kind);
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(Self::size())
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_decorations(false);
        let view = self.progress.snapshot();
        let time_left = self.clock.time_left(view.done, view.total).to_owned();
        let cancelled = self.progress.is_cancelled();
        let cancel = ctx.show_viewport_immediate(self.viewport, builder, |ui, _| {
            let closed = ui.input(|i| i.viewport().close_requested());
            draw(ui, title, self.kind, &view, &time_left, now, !cancelled && !self.finished) || closed
        });
        if cancel {
            self.progress.cancel();
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(FRAME_TIME));
        true
    }
}

fn title(kind: Kind) -> &'static str {
    match kind {
        Kind::Copy => "Copying...",
        Kind::Move => "Moving...",
        Kind::Send => "Sending...",
        Kind::Receive => "Receiving...",
    }
}

/// Draws the whole window. Returns true when Cancel was clicked or Escape
/// pressed.
pub fn draw(ui: &mut Ui, title: &str, kind: Kind, view: &Snapshot, time_left: &str, now: f64, can_cancel: bool) -> bool {
    let (outer, _) = ui.allocate_exact_size(vec2(WIDTH, HEIGHT), Sense::hover());
    let p = ui.painter();
    p.rect_filled(outer, 0.0, theme::color::face());
    theme::bevel(p, outer, Bevel::Raised);
    let inner = outer.shrink(FRAME);
    let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    theme::dialog_title_bar(&mut caption, title);

    let (left, right) = (inner.left() + MARGIN, inner.right() - MARGIN);
    let mut y = inner.top() + 18.0 + 8.0;
    animation(ui, Rect::from_min_size(pos2(left, y), ANIMATION), kind, now);
    y += ANIMATION.y + 6.0;
    for line in &view.lines {
        text_line(ui, pos2(left, y), right - left, line);
        y += 15.0;
    }
    y += 6.0;
    let fraction = if view.total == 0 { 0.0 } else { view.done as f32 / view.total as f32 };
    progress_bar(ui, Rect::from_min_max(pos2(left, y), pos2(right, y + 16.0)), fraction);
    y += 16.0 + 8.0;
    let button = Rect::from_min_size(pos2(right - BUTTON.x, inner.bottom() - 10.0 - BUTTON.y), BUTTON);
    text_line(ui, pos2(left, y), button.left() - 8.0 - left, time_left);
    let escape = ui.input(|i| i.key_pressed(Key::Escape));
    let clicked = theme::push_button(ui, button, "Cancel", can_cancel, true).clicked();
    can_cancel && (clicked || escape)
}

/// The name a path is shown by: its last part, or the whole path for `/`.
fn label(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// One line of text, cut short with an ellipsis to fit `width`.
fn text_line(ui: &Ui, at: egui::Pos2, width: f32, text: &str) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, theme::color::text());
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(at, galley, theme::color::text());
}

/// The Windows Classic progress bar (comctl32's msctls_progress32 without
/// `PBS_SMOOTH`): a thin sunken edge around highlight-colored blocks, two
/// thirds as wide as they are tall, with a gap between them. The last block
/// is cut off at the bar's end.
fn progress_bar(ui: &Ui, rect: Rect, fraction: f32) {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, theme::color::face());
    let stroke = |a, b, c| p.line_segment([a, b], egui::Stroke::new(1.0, c));
    let r = rect.shrink(0.5);
    stroke(r.left_top(), r.right_top(), theme::color::shadow());
    stroke(r.left_top(), r.left_bottom(), theme::color::shadow());
    stroke(r.left_bottom(), r.right_bottom(), theme::color::highlight());
    stroke(r.right_top(), r.right_bottom(), theme::color::highlight());
    let bar = rect.shrink(2.0);
    let block = (bar.height() * 2.0 / 3.0).floor();
    let filled = bar.left() + bar.width() * fraction.clamp(0.0, 1.0);
    let mut x = bar.left();
    while x < filled {
        let block = Rect::from_min_max(pos2(x, bar.top()), pos2((x + block).min(bar.right()), bar.bottom()));
        p.rect_filled(block, 0.0, theme::color::selection());
        x += block.width() + 2.0;
    }
}

/// Paper flying from the folder on the left to the one on the right, or
/// between a computer and a folder for a send or a receive.
fn animation(ui: &Ui, area: Rect, kind: Kind, now: f64) {
    let p = ui.painter();
    let folder = vec2(32.0, 32.0);
    let from = Rect::from_min_size(pos2(area.left() + 4.0, area.bottom() - folder.y - 2.0), folder);
    let to = Rect::from_min_size(pos2(area.right() - folder.x - 4.0, area.bottom() - folder.y - 2.0), folder);
    let (from_icon, to_icon) = match kind {
        Kind::Copy | Kind::Move => (Icon::FolderOpen, Icon::Folder),
        Kind::Send => (Icon::FolderOpen, Icon::Computer),
        Kind::Receive => (Icon::Computer, Icon::FolderOpen),
    };
    theme::paint_icon(p, from, from_icon, true);
    theme::paint_icon(p, to, to_icon, true);
    // The same 15 frames a second as the AVI, so the flight steps like it.
    let t = (((now / FRAME_TIME).floor() * FRAME_TIME) % FLIGHT / FLIGHT) as f32;
    let start = from.center() + vec2(0.0, -8.0);
    let end = to.center() + vec2(0.0, -8.0);
    let lift = (t * std::f32::consts::PI).sin() * (area.height() - folder.y);
    let center = start.lerp(end, t) - vec2(0.0, lift);
    let page = Rect::from_center_size(center, vec2(16.0, 16.0));
    theme::paint_page(p, page, true);
}
