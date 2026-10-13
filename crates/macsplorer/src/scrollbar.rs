//! Windows Classic scroll bars: arrow buttons, a dithered track and a raised
//! thumb. egui's own bars are hidden; these read and write the `ScrollArea`
//! state instead.
//!
//! Behavior follows the classic user32 scroll bar: arrows scroll a line,
//! clicks in the track scroll a page toward the pointer until the thumb
//! reaches it, both auto-repeat while held, and the pressed part of the
//! track is drawn inverted. Not traced from the binaries (user32 isn't in
//! the analysis set): the repeat timings and the 8 px minimum thumb are
//! guesses from memory of Windows 2000.

use eframe::egui;
use egui::scroll_area::{ScrollAreaOutput, ScrollBarVisibility};
use egui::{Color32, Id, Mesh, Painter, Rangef, Rect, Sense, Stroke, Ui, Vec2, Vec2b, pos2, vec2};

use crate::theme::color::{FACE, HIGHLIGHT, SHADOW, TEXT};
use crate::theme::{Bevel, bevel};

/// `SM_CXVSCROLL` / `SM_CYHSCROLL`.
pub const WIDTH: f32 = 16.0;
/// Guess: half the bar width, as Windows sizes the minimum thumb.
const MIN_THUMB: f32 = 8.0;
/// Guess: delay before a held arrow or track click starts repeating.
const FIRST_DELAY: f64 = 0.25;
/// Guess: interval between repeats.
const REPEAT_DELAY: f64 = 0.05;

/// The track while a page click is held: `COLOR_3DFACE` / white, inverted.
const INVERTED_FACE: Color32 = Color32::from_rgb(0xFF - 0xD4, 0xFF - 0xD0, 0xFF - 0xC8);

/// Classic bars around a `ScrollArea`: [`Bars::begin`] carves the bars out
/// of the given rect, the caller shows its scroll area in the returned
/// viewport with [`hidden`] egui bars, then [`Bars::end`] draws and drives
/// the classic ones.
pub struct Bars {
    id: Id,
    outer: Rect,
    dirs: Vec2b,
    show: Vec2b,
    line: f32,
}

/// Turns off egui's own bars (and the space they reserve) for a scroll
/// area inside `ui`.
pub fn hidden(ui: &mut Ui) -> ScrollBarVisibility {
    ui.spacing_mut().scroll.bar_width = 0.0;
    ScrollBarVisibility::AlwaysHidden
}

impl Bars {
    /// `line` is how far an arrow click scrolls. Which bars show is decided
    /// from the previous frame's content size.
    pub fn begin(ui: &Ui, id: Id, outer: Rect, dirs: impl Into<Vec2b>, line: f32) -> (Self, Rect) {
        let show = ui.data(|d| d.get_temp::<Vec2b>(id)).unwrap_or(Vec2b::FALSE);
        let viewport = Rect::from_min_max(outer.min, outer.max - vec2(bar_use(show.y), bar_use(show.x)));
        (Self { id, outer, dirs: dirs.into(), show, line }, viewport)
    }

    pub fn end<R>(self, ui: &mut Ui, out: &ScrollAreaOutput<R>) {
        self.end_split(ui, out, out);
    }

    /// Like [`Bars::end`] with the horizontal bar driving `h` and the
    /// vertical one `v`, for a vertical scroll area nested in a horizontal
    /// one. Room in the viewport outside a scroll area's own rect (a list
    /// header above it) doesn't count as page.
    pub fn end_split<R, S>(self, ui: &mut Ui, h: &ScrollAreaOutput<R>, v: &ScrollAreaOutput<S>) {
        let Self { id, outer, dirs, show, line } = self;
        let viewport = Rect::from_min_max(outer.min, outer.max - vec2(bar_use(show.y), bar_use(show.x)));
        let content = vec2(h.content_size.x, v.content_size.y);
        let chrome = vec2(viewport.width() - h.inner_rect.width(), viewport.height() - v.inner_rect.height()).max(Vec2::ZERO);
        let size = outer.size() - chrome;
        // A bar is needed when the content overflows; one bar can make the
        // other necessary by taking away room.
        let mut need = Vec2b::new(false, dirs.y && content.y > size.y + 0.5);
        need.x = dirs.x && content.x > size.x - bar_use(need.y) + 0.5;
        need.y |= dirs.y && content.y > size.y - bar_use(need.x) + 0.5;
        if need != show {
            ui.data_mut(|d| d.insert_temp(id, need));
            ui.ctx().request_repaint();
            return;
        }

        for (d, out_id, out_state) in [(0, h.id, h.state), (1, v.id, v.state)] {
            if !show[d] {
                continue;
            }
            let rect = if d == 1 {
                Rect::from_x_y_ranges(viewport.right()..=outer.right(), viewport.y_range())
            } else {
                Rect::from_x_y_ranges(viewport.x_range(), viewport.bottom()..=outer.bottom())
            };
            let mut state = egui::scroll_area::State::load(ui.ctx(), out_id).unwrap_or(out_state);
            let metrics = Metrics { content: content[d], page: viewport.size()[d] - chrome[d], line };
            if bar(ui, id.with(d), rect, d, &mut state.offset[d], metrics) {
                state.store(ui.ctx(), out_id);
                ui.ctx().request_repaint();
            }
        }
        if show.x && show.y {
            ui.painter().rect_filled(Rect::from_min_max(viewport.max, outer.max), 0.0, FACE);
        }
    }
}

fn bar_use(shown: bool) -> f32 {
    if shown { WIDTH } else { 0.0 }
}

#[derive(Clone, Copy)]
struct Metrics {
    content: f32,
    page: f32,
    line: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Thumb,
}

/// What a press on the bar is doing, kept until the button is released.
#[derive(Clone, Copy, Debug)]
struct Track {
    part: Part,
    /// Pointer distance from the thumb's start when it was grabbed.
    grab: f32,
    next: f64,
    repeating: bool,
}

/// Geometry of one bar along its axis `d` (0 horizontal, 1 vertical).
struct Layout {
    d: usize,
    cross: Rangef,
    up: Rangef,
    down: Rangef,
    track: Rangef,
    thumb: Option<Rangef>,
}

impl Layout {
    fn new(rect: Rect, d: usize, offset: f32, m: Metrics) -> Self {
        let (lo, hi) = (rect.min[d], rect.max[d]);
        let arrow = WIDTH.min(((hi - lo) / 2.0).floor());
        let track = Rangef::new(lo + arrow, hi - arrow);
        let max_offset = (m.content - m.page).max(0.0);
        let len = (track.span() * m.page / m.content.max(1.0)).round().max(MIN_THUMB);
        let thumb = (len < track.span() && max_offset > 0.0).then(|| {
            let start = track.min + ((track.span() - len) * offset / max_offset).round();
            Rangef::new(start, start + len)
        });
        Self {
            d,
            cross: if d == 1 { rect.x_range() } else { rect.y_range() },
            up: Rangef::new(lo, lo + arrow),
            down: Rangef::new(hi - arrow, hi),
            track,
            thumb,
        }
    }

    fn rect(&self, along: Rangef) -> Rect {
        if self.d == 1 { Rect::from_x_y_ranges(self.cross, along) } else { Rect::from_x_y_ranges(along, self.cross) }
    }

    fn hit(&self, p: f32) -> Part {
        if self.up.contains(p) && p < self.up.max {
            Part::LineUp
        } else if self.down.contains(p) && p > self.down.min {
            Part::LineDown
        } else {
            match self.thumb {
                Some(t) if p < t.min => Part::PageUp,
                Some(t) if p >= t.max => Part::PageDown,
                Some(_) => Part::Thumb,
                // Without a thumb the track pages toward its nearer end.
                None if p < self.track.center() => Part::PageUp,
                None => Part::PageDown,
            }
        }
    }

    /// The track segment a held page click scrolls through.
    fn page_segment(&self, part: Part) -> Option<Rangef> {
        let (t0, t1) = self.thumb.map_or((self.track.center(), self.track.center()), |t| (t.min, t.max));
        match part {
            Part::PageUp => Some(Rangef::new(self.track.min, t0)),
            Part::PageDown => Some(Rangef::new(t1, self.track.max)),
            _ => None,
        }
    }
}

/// Draws one bar and handles presses and the wheel over it. Returns
/// whether `offset` changed.
fn bar(ui: &mut Ui, id: Id, rect: Rect, d: usize, offset: &mut f32, m: Metrics) -> bool {
    let max_offset = (m.content - m.page).max(0.0);
    let before = *offset;
    let layout = Layout::new(rect, d, *offset, m);
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let (now, pointer, wheel) = ui.input(|i| (i.time, i.pointer.interact_pos(), i.smooth_scroll_delta));

    let mut track = ui.data(|data| data.get_temp::<Track>(id));
    let mut active = None;
    match (resp.is_pointer_button_down_on(), pointer) {
        (true, Some(p)) => {
            let t = track.get_or_insert_with(|| {
                let part = layout.hit(p[d]);
                let grab = layout.thumb.map_or(0.0, |t| p[d] - t.min);
                Track { part, grab, next: now, repeating: false }
            });
            match t.part {
                Part::Thumb => {
                    if let Some(thumb) = layout.thumb {
                        let travel = layout.track.span() - thumb.span();
                        let start = p[d] - t.grab - layout.track.min;
                        *offset = start / travel * max_offset;
                    }
                    active = Some(Part::Thumb);
                }
                part => {
                    // Arrows and page clicks act only while the pointer stays
                    // on their part; a page click stops once the thumb
                    // reaches the pointer.
                    let here = layout.hit(p[d]) == part && rect.contains(p);
                    if here {
                        active = Some(part);
                        if now >= t.next {
                            *offset += match part {
                                Part::LineUp => -m.line,
                                Part::LineDown => m.line,
                                Part::PageUp => -m.page,
                                _ => m.page,
                            };
                            t.next = now + if t.repeating { REPEAT_DELAY } else { FIRST_DELAY };
                            t.repeating = true;
                        }
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((t.next - now).max(0.0)));
                }
            }
            ui.data_mut(|data| data.insert_temp(id, *t));
        }
        _ => {
            if track.take().is_some() {
                ui.data_mut(|data| data.remove::<Track>(id));
            }
        }
    }

    // The wheel scrolls over the bar too, not just over the content.
    if resp.hovered() && wheel[d] != 0.0 {
        *offset -= wheel[d];
        ui.input_mut(|i| i.smooth_scroll_delta[d] = 0.0);
    }
    *offset = offset.clamp(0.0, max_offset);

    let layout = Layout::new(rect, d, *offset, m);
    paint(ui, &layout, active);
    *offset != before
}

fn paint(ui: &Ui, layout: &Layout, active: Option<Part>) {
    let p = ui.painter();
    let d = layout.d;
    dither(p, layout.rect(layout.track), FACE, HIGHLIGHT);
    if let Some(seg) = active.and_then(|part| layout.page_segment(part)) {
        dither(p, layout.rect(seg), INVERTED_FACE, Color32::BLACK);
    }
    if let Some(thumb) = layout.thumb {
        let r = layout.rect(thumb);
        p.rect_filled(r, 0.0, FACE);
        bevel(p, r, Bevel::Raised);
    }
    let (up, down) = if d == 1 { (Dir::Up, Dir::Down) } else { (Dir::Left, Dir::Right) };
    arrow_button(p, layout.rect(layout.up), up, active == Some(Part::LineUp));
    arrow_button(p, layout.rect(layout.down), down, active == Some(Part::LineDown));
}

#[derive(Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// `DrawFrameControl(DFC_SCROLL)`: a raised button, or flat with a shadow
/// border and the glyph nudged by a pixel while pressed.
fn arrow_button(p: &Painter, r: Rect, dir: Dir, pressed: bool) {
    p.rect_filled(r, 0.0, FACE);
    if pressed {
        p.rect_stroke(r.shrink(0.5), 0.0, Stroke::new(1.0, SHADOW), egui::StrokeKind::Middle);
    } else {
        bevel(p, r, Bevel::Raised);
    }
    // The Marlett arrow: four rows of 7, 5, 3 and 1 pixels, set a little
    // above and left of center like Windows draws it.
    let nudge = if pressed { 1.0 } else { 0.0 };
    let c = pos2((r.center().x - 0.5).floor() + nudge, (r.center().y - 0.5).floor() + nudge);
    for i in 0..4 {
        // Row `i` counted from the tip.
        let half = i as f32;
        let row = match dir {
            Dir::Up => Rect::from_min_max(pos2(c.x - half, c.y - 2.0 + i as f32), pos2(c.x + half + 1.0, c.y - 1.0 + i as f32)),
            Dir::Down => Rect::from_min_max(pos2(c.x - half, c.y + 1.0 - i as f32), pos2(c.x + half + 1.0, c.y + 2.0 - i as f32)),
            Dir::Left => Rect::from_min_max(pos2(c.x - 2.0 + i as f32, c.y - half), pos2(c.x - 1.0 + i as f32, c.y + half + 1.0)),
            Dir::Right => Rect::from_min_max(pos2(c.x + 1.0 - i as f32, c.y - half), pos2(c.x + 2.0 - i as f32, c.y + half + 1.0)),
        };
        p.rect_filled(row, 0.0, TEXT);
    }
}

/// The 50% checkerboard `COLOR_SCROLLBAR` brush: `base` with every other
/// pixel in `dots`, anchored to the screen so it doesn't crawl.
fn dither(p: &Painter, r: Rect, base: Color32, dots: Color32) {
    let r = Rect::from_min_max(r.min.round(), r.max.round());
    if !r.is_positive() {
        return;
    }
    p.rect_filled(r, 0.0, base);
    let mut mesh = Mesh::default();
    for y in (r.top() as i32)..(r.bottom() as i32) {
        let mut x = r.left() as i32;
        x += (x + y).rem_euclid(2);
        while x < r.right() as i32 {
            let min = pos2(x as f32, y as f32);
            mesh.add_colored_rect(Rect::from_min_size(min, vec2(1.0, 1.0)), dots);
            x += 2;
        }
    }
    p.add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(offset: f32) -> Layout {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(16.0, 132.0));
        Layout::new(rect, 1, offset, Metrics { content: 400.0, page: 100.0, line: 17.0 })
    }

    #[test]
    fn thumb_is_proportional_and_tracks_offset() {
        // Track is 132 - 2 × 16 = 100 long; a quarter of the content is visible.
        assert_eq!(layout(0.0).thumb, Some(Rangef::new(16.0, 41.0)));
        assert_eq!(layout(300.0).thumb, Some(Rangef::new(91.0, 116.0)));
    }

    #[test]
    fn hits_arrows_track_and_thumb() {
        let l = layout(150.0);
        let t = l.thumb.unwrap();
        assert_eq!(l.hit(3.0), Part::LineUp);
        assert_eq!(l.hit(130.0), Part::LineDown);
        assert_eq!(l.hit(t.min - 1.0), Part::PageUp);
        assert_eq!(l.hit(t.center()), Part::Thumb);
        assert_eq!(l.hit(t.max + 1.0), Part::PageDown);
    }

    #[test]
    fn short_bar_drops_the_thumb_and_shrinks_arrows() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(16.0, 24.0));
        let l = Layout::new(rect, 1, 0.0, Metrics { content: 400.0, page: 24.0, line: 17.0 });
        assert_eq!(l.up, Rangef::new(0.0, 12.0));
        assert!(l.thumb.is_none());
    }
}
