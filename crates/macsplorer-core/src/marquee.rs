//! Marquee (rubber band) selection, after the comctl32 ListView's
//! `CLVSelectionManager::DragSelect` and `_DragSelectItem`. See
//! `docs/file-browser-findings.md` §2.5.
//!
//! Coordinates are content coordinates, as in [`crate::layout`]: the start
//! corner stays on the content while the view scrolls under it.

use crate::layout::Rect;

impl Rect {
    /// Whether the two rectangles overlap with a non-empty area
    /// (`IntersectRect`'s result), so a zero-size marquee hits nothing.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.min.0.max(other.min.0) < self.max.0.min(other.max.0)
            && self.min.1.max(other.min.1) < self.max.1.min(other.max.1)
    }

    /// The smallest rectangle holding both (`UnionRect`).
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min: (self.min.0.min(other.min.0), self.min.1.min(other.min.1)),
            max: (self.max.0.max(other.max.0), self.max.1.max(other.max.1)),
        }
    }
}

/// A marquee drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marquee {
    /// Where the button went down.
    pub origin: (f32, f32),
    /// The band: `origin` to the clamped pointer, normalized.
    pub rect: Rect,
}

impl Marquee {
    pub fn new(origin: (f32, f32)) -> Self {
        Self { origin, rect: Rect { min: origin, max: origin } }
    }

    /// Moves the free corner to `to`, clamped into `clip` (the part of the
    /// view that is showing), and returns the band it had before.
    pub fn drag_to(&mut self, to: (f32, f32), clip: Rect) -> Rect {
        let to = (to.0.clamp(clip.min.0, clip.max.0), to.1.clamp(clip.min.1, clip.max.1));
        let old = self.rect;
        self.rect = Rect {
            min: (self.origin.0.min(to.0), self.origin.1.min(to.1)),
            max: (self.origin.0.max(to.0), self.origin.1.max(to.1)),
        };
        old
    }

    /// An item's new selection state after the band moved from `old` to
    /// `self.rect`, or `None` to leave it. Only items the band's edge crossed
    /// change: they become "inside the band", or flip with `toggle` (Ctrl).
    /// `bounds` comes from [`hit_bounds`].
    pub fn item_state(&self, old: &Rect, bounds: &Rect, selected: bool, toggle: bool) -> Option<bool> {
        let (was, now) = (old.intersects(bounds), self.rect.intersects(bounds));
        if was == now {
            None
        } else if toggle {
            Some(!selected)
        } else {
            Some(now)
        }
    }
}

/// The part of an item's select bounds (icon plus label) the band has to
/// touch: shrunk by a quarter of its width and height on each side.
pub fn hit_bounds(select_bounds: Rect) -> Rect {
    let (w, h) = (select_bounds.max.0 - select_bounds.min.0, select_bounds.max.1 - select_bounds.min.1);
    Rect {
        min: (select_bounds.min.0 + w / 4.0, select_bounds.min.1 + h / 4.0),
        max: (select_bounds.max.0 - w / 4.0, select_bounds.max.1 - h / 4.0),
    }
}

/// The new Shift+click anchor after a move without modifiers: of the items
/// inside the band, the one whose [`hit_bounds`] top-left corner is farthest
/// from the pointer, the first one on ties.
pub fn anchor<T: Copy>(pointer: (f32, f32), inside: impl IntoIterator<Item = (T, Rect)>) -> Option<T> {
    let mut best: Option<(T, f32)> = None;
    for (item, r) in inside {
        let (dx, dy) = (r.min.0 - pointer.0, r.min.1 - pointer.1);
        let d = dx * dx + dy * dy;
        if best.is_none_or(|(_, b)| d > b) {
            best = Some((item, d));
        }
    }
    best.map(|(item, _)| item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { min: (x0, y0), max: (x1, y1) }
    }

    #[test]
    fn band_is_normalized_and_clamped() {
        let mut m = Marquee::new((50.0, 50.0));
        let clip = rect(0.0, 20.0, 100.0, 200.0);
        m.drag_to((10.0, 0.0), clip);
        assert_eq!(m.rect, rect(10.0, 20.0, 50.0, 50.0));
        let old = m.drag_to((500.0, 120.0), clip);
        assert_eq!(old, rect(10.0, 20.0, 50.0, 50.0));
        assert_eq!(m.rect, rect(50.0, 50.0, 100.0, 120.0));
    }

    #[test]
    fn empty_band_hits_nothing() {
        let m = Marquee::new((5.0, 5.0));
        assert!(!m.rect.intersects(&rect(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn items_change_only_when_the_edge_crosses_them() {
        let item = hit_bounds(rect(0.0, 0.0, 40.0, 40.0));
        assert_eq!(item, rect(10.0, 10.0, 30.0, 30.0));
        let mut m = Marquee::new((0.0, 0.0));
        let clip = rect(0.0, 0.0, 1000.0, 1000.0);
        // Touching the slot's outer quarter is not enough.
        let old = m.drag_to((8.0, 8.0), clip);
        assert_eq!(m.item_state(&old, &item, false, false), None);
        let old = m.drag_to((12.0, 12.0), clip);
        assert_eq!(m.item_state(&old, &item, false, false), Some(true));
        // Still inside: a preselected item keeps whatever it has.
        let old = m.drag_to((20.0, 20.0), clip);
        assert_eq!(m.item_state(&old, &item, true, false), None);
        // Leaving deselects; with Ctrl it flips instead.
        let old = m.drag_to((5.0, 5.0), clip);
        assert_eq!(m.item_state(&old, &item, true, false), Some(false));
        assert_eq!(m.item_state(&old, &item, false, true), Some(true));
    }

    #[test]
    fn anchor_is_the_farthest_item() {
        let items = [(0, rect(0.0, 0.0, 10.0, 10.0)), (1, rect(50.0, 50.0, 60.0, 60.0))];
        assert_eq!(anchor((60.0, 60.0), items), Some(0));
        assert_eq!(anchor((0.0, 0.0), items), Some(1));
        assert_eq!(anchor((0.0, 0.0), std::iter::empty::<(u8, Rect)>()), None);
    }
}
