//! Details view column header: where the dividers can be grabbed and how a
//! drag resizes a column, after the comctl32 header control (`SysHeader32`:
//! `Header_HitTest`, `Header_OnLButtonDown`, `Header_OnMouseMove`,
//! `Header_OnLButtonUp`). See `docs/file-browser-findings.md` §2.6.
//!
//! X coordinates are relative to the header's left edge (column 0's left).

/// How far a divider can be grabbed on either side: 8 × `SM_CXBORDER`. A
/// column narrower than four times this gives a quarter of its width.
pub const DIVIDER_SLOP: f32 = 8.0;

/// What a point in the header is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderHit {
    /// A column's caption; a click sorts by it.
    Column(usize),
    /// The divider at a column's right edge (`HHT_ONDIVIDER`): a drag resizes
    /// that column, a double-click auto-sizes it.
    Divider(usize),
    /// Just right of a zero-width column's divider (`HHT_ONDIVOPEN`): a drag
    /// pulls the hidden column back open.
    DividerOpen(usize),
    /// The empty header past the last column (`HHT_NOWHERE`).
    Nowhere,
}

impl HeaderHit {
    /// The column a press here would resize.
    pub fn divider(self) -> Option<usize> {
        match self {
            HeaderHit::Divider(i) | HeaderHit::DividerOpen(i) => Some(i),
            _ => None,
        }
    }
}

/// Hit-tests `x` against columns of `widths`, in a header `client_width`
/// wide. The space past the last column acts as one more column, so the
/// last divider can be grabbed from its right side too.
pub fn hit_test(widths: &[f32], client_width: f32, x: f32) -> HeaderHit {
    let mut left = 0.0;
    // Whether the previous column had zero width.
    let mut after_zero = false;
    for i in 0..=widths.len() {
        let right = widths.get(i).map_or(client_width, |w| left + w);
        if right <= left {
            after_zero = i < widths.len();
            left = left.max(right);
            continue;
        }
        let slop = DIVIDER_SLOP.min((right - left) / 4.0);
        if left <= x && x < right {
            if i > 0 && x < left + slop {
                return if after_zero && x > left { HeaderHit::DividerOpen(i - 1) } else { HeaderHit::Divider(i - 1) };
            }
            if i < widths.len() && x >= right - slop {
                return HeaderHit::Divider(i);
            }
            return if i < widths.len() { HeaderHit::Column(i) } else { HeaderHit::Nowhere };
        }
        after_zero = false;
        left = right;
    }
    HeaderHit::Nowhere
}

/// The left edge of column `i`.
pub fn column_left(widths: &[f32], i: usize) -> f32 {
    widths[..i].iter().sum()
}

/// A divider drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnDrag {
    pub column: usize,
    /// The width before the drag, restored if it's cancelled.
    pub original: f32,
    /// The column's right edge minus the press x, so the divider keeps its
    /// distance to the pointer.
    grab: f32,
    left: f32,
}

impl ColumnDrag {
    /// Starts dragging the divider of `column`, pressed at `x`.
    pub fn new(widths: &[f32], column: usize, x: f32) -> Self {
        let left = column_left(widths, column);
        Self { column, original: widths[column], grab: left + widths[column] - x, left }
    }

    /// The column's width with the pointer at `x`. The divider can't pass
    /// the column's left edge; there's no maximum.
    pub fn width_at(&self, x: f32) -> f32 {
        (x + self.grab - self.left).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: [f32; 3] = [100.0, 50.0, 20.0];

    #[test]
    fn dividers_reach_eight_px_each_side() {
        assert_eq!(hit_test(&W, 400.0, 50.0), HeaderHit::Column(0));
        assert_eq!(hit_test(&W, 400.0, 91.0), HeaderHit::Column(0));
        assert_eq!(hit_test(&W, 400.0, 92.0), HeaderHit::Divider(0));
        assert_eq!(hit_test(&W, 400.0, 107.9), HeaderHit::Divider(0));
        assert_eq!(hit_test(&W, 400.0, 108.0), HeaderHit::Column(1));
    }

    #[test]
    fn narrow_columns_give_a_quarter() {
        // Column 2 is 20 wide: 5 px zones inside it.
        assert_eq!(hit_test(&W, 400.0, 154.0), HeaderHit::Divider(1));
        assert_eq!(hit_test(&W, 400.0, 155.0), HeaderHit::Column(2));
        assert_eq!(hit_test(&W, 400.0, 164.0), HeaderHit::Column(2));
        assert_eq!(hit_test(&W, 400.0, 165.0), HeaderHit::Divider(2));
    }

    #[test]
    fn last_divider_reaches_into_the_empty_header() {
        assert_eq!(hit_test(&W, 400.0, 177.0), HeaderHit::Divider(2));
        assert_eq!(hit_test(&W, 400.0, 178.0), HeaderHit::Nowhere);
        assert_eq!(hit_test(&W, 400.0, 500.0), HeaderHit::Nowhere);
        // No empty header when the columns fill it.
        assert_eq!(hit_test(&W, 170.0, 172.0), HeaderHit::Nowhere);
    }

    #[test]
    fn zero_width_column_opens_from_the_right() {
        let w = [100.0, 0.0, 50.0];
        // Left of x = 100 is column 0's divider; right of it opens column 1.
        assert_eq!(hit_test(&w, 400.0, 95.0), HeaderHit::Divider(0));
        assert_eq!(hit_test(&w, 400.0, 100.0), HeaderHit::Divider(1));
        assert_eq!(hit_test(&w, 400.0, 104.0), HeaderHit::DividerOpen(1));
        assert_eq!(hit_test(&w, 400.0, 108.0), HeaderHit::Column(2));
    }

    #[test]
    fn drag_keeps_grab_offset_and_stops_at_zero() {
        let d = ColumnDrag::new(&W, 1, 147.0);
        assert_eq!(d.original, 50.0);
        assert_eq!(d.width_at(147.0), 50.0);
        assert_eq!(d.width_at(250.0), 153.0);
        assert_eq!(d.width_at(90.0), 0.0);
    }
}
