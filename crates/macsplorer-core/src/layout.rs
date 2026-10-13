//! Geometry and keyboard movement for the Large Icons view, after
//! the comctl32 ListView (SysListView32) that Windows 2000's Explorer used.
//! See `docs/file-browser-findings.md` §2.5 for the traced functions.
//!
//! Coordinates are in points relative to the view's content origin (top-left
//! of the scrollable area, before scrolling). Metrics are Win2000's at 96 DPI.

use std::ops::Range;

/// Icon slot size: `SM_CXICONSPACING` × `SM_CYICONSPACING` with 32 px icons
/// (`CLVImageListManager::OnSetIconSpacing`). The 75 px default is user32's
/// "Icon Spacing" setting, not read from a binary.
pub const SLOT: f32 = 75.0;
/// Large icon image size.
pub const ICON: f32 = 32.0;
/// Image top inside its slot, `2 · SM_CYBORDER` (`GetIconBufferY`).
pub const ICON_TOP: f32 = 2.0;
/// Gap from the image bottom to the label, `SM_CYBORDER + SM_CYEDGE`.
pub const LABEL_GAP: f32 = 3.0;
/// Horizontal padding on each side of a label, `SM_CXEDGE`.
pub const LABEL_PAD_X: f32 = 2.0;
/// Extra label height, `SM_CYEDGE`.
pub const LABEL_PAD_Y: f32 = 2.0;
/// Width large icon labels wrap in, `SLOT - 2 · SM_CXEDGE`
/// (`CLVView::_ComputeLabelSizeWorker`).
pub const LABEL_WRAP: f32 = SLOT - 2.0 * LABEL_PAD_X;
/// Lines a folded label shows (`CLVIconView::_RefoldLabelRect`).
pub const FOLDED_LINES: usize = 2;
/// The icon hit area reaches this far left and right of the image,
/// `8 · SM_CXBORDER` (`CLVIconView::v_ItemHitTest`).
pub const ICON_HIT_PAD: f32 = 8.0;

/// Keyboard movement within a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// An axis-aligned rectangle, `min` inclusive, `max` exclusive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: (f32, f32),
    pub max: (f32, f32),
}

impl Rect {
    pub fn from_min_size(min: (f32, f32), size: (f32, f32)) -> Self {
        Self { min, max: (min.0 + size.0, min.1 + size.1) }
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        (self.min.0..self.max.0).contains(&p.0) && (self.min.1..self.max.1).contains(&p.1)
    }
}

/// Auto-arranged Large Icons slots, filled left to right, then top to bottom
/// (`LVS_ALIGNTOP`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconGrid {
    pub count: usize,
    pub per_row: usize,
}

impl IconGrid {
    /// Slots per row for a client area of `width` × `height`
    /// (`CLVSlotsManager::GetSlotCountEx`): first without a vertical scroll
    /// bar, and if the slots that fit can't hold every item, again with
    /// `scrollbar` taken off the width.
    pub fn new(count: usize, width: f32, height: f32, scrollbar: f32) -> Self {
        let fit = |w: f32| ((w / SLOT).floor() as usize).max(1);
        let rows = fit(height);
        let mut per_row = fit(width);
        if per_row * rows < count {
            per_row = fit(width - scrollbar);
        }
        Self { count, per_row }
    }

    pub fn rows(&self) -> usize {
        self.count.div_ceil(self.per_row)
    }

    /// Top-left corner of slot `i` (`CLVSlotsManager::CalcSlotRect`).
    pub fn slot(&self, i: usize) -> (f32, f32) {
        ((i % self.per_row) as f32 * SLOT, (i / self.per_row) as f32 * SLOT)
    }

    pub fn content_height(&self) -> f32 {
        self.rows() as f32 * SLOT
    }

    /// Items in the rows that overlap `top..bottom`.
    pub fn items_in(&self, top: f32, bottom: f32) -> Range<usize> {
        let first = (top.max(0.0) / SLOT).floor() as usize;
        let last = (bottom.max(0.0) / SLOT).ceil() as usize;
        (first * self.per_row).min(self.count)..(last * self.per_row).min(self.count)
    }

    /// The item the cursor moves to from `from`, or `None` to stay
    /// (`CLVKeyboardManager::OnArrow` / `FindNearestItem`). Arrows pick the
    /// nearest slot within 45° of the direction, so they don't wrap at row
    /// ends. Page keys move a page from the focused slot and take the nearest
    /// item above that point.
    pub fn navigate(&self, from: usize, nav: Nav, client_height: f32) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        let slots = (0..self.count).map(|i| (i, self.slot(i)));
        let (x, y) = self.slot(from);
        match nav {
            Nav::Home => Some(0),
            Nav::End => Some(self.count - 1),
            Nav::Left | Nav::Right | Nav::Up | Nav::Down => nearest((x, y), nav, slots),
            Nav::PageUp => {
                let y = y - (client_height - SLOT / 2.0).max(SLOT / 2.0);
                if y < 0.0 { Some(0) } else { nearest((x, y), Nav::Up, slots) }
            }
            Nav::PageDown => {
                let y = y + (client_height + SLOT / 2.0).max(SLOT * 1.5);
                if y > self.content_height() { Some(self.count - 1) } else { nearest((x, y), Nav::Up, slots) }
            }
        }
    }
}

/// The nearest point to `from` in direction `nav` (Left/Right/Up/Down), as
/// `CLVKeyboardManager::_NearestItemCommon` picks it: candidates must lie
/// strictly on that side and at least as far along the direction as across it;
/// the smallest squared distance wins, the first one on ties.
pub fn nearest(from: (f32, f32), nav: Nav, points: impl IntoIterator<Item = (usize, (f32, f32))>) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, (px, py)) in points {
        let (dx, dy) = (px - from.0, py - from.1);
        let ok = match nav {
            Nav::Left => dx < 0.0 && dx.abs() >= dy.abs(),
            Nav::Right => dx > 0.0 && dx.abs() >= dy.abs(),
            Nav::Up => dy < 0.0 && dy.abs() >= dx.abs(),
            Nav::Down => dy > 0.0 && dy.abs() >= dx.abs(),
            _ => false,
        };
        let d = dx * dx + dy * dy;
        if ok && best.is_none_or(|(_, b)| d < b) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// Label box size from the measured text: text width plus `SM_CXEDGE` each
/// side, text height plus `SM_CYEDGE`. A folded label is cut to
/// [`FOLDED_LINES`] lines of `line_height`.
pub fn label_size(text: (f32, f32), line_height: f32, folded: bool) -> (f32, f32) {
    let full = text.1 + LABEL_PAD_Y;
    let h = if folded { full.min(LABEL_PAD_Y + FOLDED_LINES as f32 * line_height) } else { full };
    (text.0 + 2.0 * LABEL_PAD_X, h)
}

/// One Large Icons item placed in its slot (`CLVIconView::GetRects`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconCell {
    /// The 32 px image, centered horizontally in the slot.
    pub image: Rect,
    /// The label box, centered under the image.
    pub label: Rect,
}

impl IconCell {
    /// `label` is the label box size from [`label_size`].
    pub fn new(slot: (f32, f32), label: (f32, f32)) -> Self {
        let image = Rect::from_min_size((slot.0 + (SLOT - ICON) / 2.0, slot.1 + ICON_TOP), (ICON, ICON));
        let label_min = (slot.0 + (SLOT - label.0) / 2.0, image.max.1 + LABEL_GAP);
        Self { image, label: Rect::from_min_size(label_min, label) }
    }

    /// Whether `p` hits the item (`CLVIconView::_HitTestItemRect`): the image
    /// widened by [`ICON_HIT_PAD`] each side, from `ICON_TOP` above it down to
    /// the label, or the label box. The rest of the slot is empty space.
    pub fn hit(&self, p: (f32, f32)) -> bool {
        let icon = Rect {
            min: (self.image.min.0 - ICON_HIT_PAD, self.image.min.1 - ICON_TOP),
            max: (self.image.max.0 + ICON_HIT_PAD, self.label.min.1),
        };
        icon.contains(p) || self.label.contains(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_fits_slots_and_makes_room_for_the_scroll_bar() {
        // 300 wide fits 4 slots; 2 rows of 150 hold 8 items.
        assert_eq!(IconGrid::new(8, 300.0, 150.0, 16.0).per_row, 4);
        // 9 items need a scroll bar, which leaves room for 3.
        assert_eq!(IconGrid::new(9, 300.0, 150.0, 16.0).per_row, 3);
        assert_eq!(IconGrid::new(5, 10.0, 10.0, 16.0).per_row, 1);
    }

    #[test]
    fn grid_slots_fill_rows() {
        let g = IconGrid { count: 7, per_row: 3 };
        assert_eq!(g.slot(0), (0.0, 0.0));
        assert_eq!(g.slot(4), (SLOT, SLOT));
        assert_eq!(g.rows(), 3);
        assert_eq!(g.items_in(80.0, 140.0), 3..6);
        assert_eq!(g.items_in(0.0, 1000.0), 0..7);
    }

    #[test]
    fn grid_arrows_do_not_wrap() {
        // 0 1 2
        // 3 4 5
        // 6
        let g = IconGrid { count: 7, per_row: 3 };
        let nav = |i, n| g.navigate(i, n, 1000.0);
        assert_eq!(nav(1, Nav::Right), Some(2));
        assert_eq!(nav(2, Nav::Right), None);
        assert_eq!(nav(3, Nav::Left), None);
        assert_eq!(nav(4, Nav::Up), Some(1));
        assert_eq!(nav(1, Nav::Down), Some(4));
        // Diagonal at 45° still counts: 4 -> 6 is one column left, one row down.
        assert_eq!(nav(4, Nav::Down), Some(6));
        // Two columns left is too far off the direction.
        assert_eq!(nav(5, Nav::Down), None);
        assert_eq!(nav(6, Nav::Up), Some(3));
        assert_eq!(nav(4, Nav::Home), Some(0));
        assert_eq!(nav(0, Nav::End), Some(6));
    }

    #[test]
    fn grid_page_keys() {
        // 2 per row, 10 rows; a 160 tall client shows about 2 rows.
        let g = IconGrid { count: 20, per_row: 2 };
        // Down by max(160 + 37.5, 112.5) = 197.5 from row 0, then the nearest
        // slot above that point: row 2 (y 150), same column.
        assert_eq!(g.navigate(1, Nav::PageDown, 160.0), Some(5));
        assert_eq!(g.navigate(5, Nav::PageUp, 160.0), Some(1));
        assert_eq!(g.navigate(19, Nav::PageDown, 160.0), Some(19));
        assert_eq!(g.navigate(3, Nav::PageUp, 160.0), Some(0));
    }

    #[test]
    fn nearest_prefers_the_closest_in_the_cone() {
        let pts = [(0, (100.0, 0.0)), (1, (60.0, 50.0)), (2, (40.0, 50.0))];
        // 1 is closer than 0 and still within 45° of Right; 2 is not.
        assert_eq!(nearest((0.0, 0.0), Nav::Right, pts), Some(1));
        assert_eq!(nearest((0.0, 0.0), Nav::Down, pts), Some(2));
        assert_eq!(nearest((0.0, 0.0), Nav::Left, pts), None);
    }

    #[test]
    fn labels_fold_to_two_lines() {
        assert_eq!(label_size((40.0, 15.0), 15.0, true), (44.0, 17.0));
        assert_eq!(label_size((70.0, 45.0), 15.0, true), (74.0, 32.0));
        assert_eq!(label_size((70.0, 45.0), 15.0, false), (74.0, 47.0));
    }

    #[test]
    fn icon_cell_geometry_and_hits() {
        let c = IconCell::new((75.0, 0.0), (44.0, 17.0));
        assert_eq!(c.image, Rect::from_min_size((75.0 + 21.5, 2.0), (32.0, 32.0)));
        assert_eq!(c.label, Rect::from_min_size((75.0 + 15.5, 37.0), (44.0, 17.0)));
        assert!(c.hit((75.0 + 15.0, 10.0))); // within 8 px left of the image
        assert!(!c.hit((75.0 + 5.0, 10.0))); // slot corner
        assert!(c.hit((75.0 + 37.0, 50.0))); // label
        assert!(!c.hit((75.0 + 37.0, 60.0))); // below the label
    }
}
