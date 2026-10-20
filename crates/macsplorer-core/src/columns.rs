//! Details view columns: which ones exist, which are shown in what order
//! (after shell32's `CDefView` header menu and `CColumnDlg`, §2.10), and the
//! column header's divider hit test and resize drag, after the comctl32
//! header control (`SysHeader32`: `Header_HitTest`, `Header_OnLButtonDown`,
//! `Header_OnMouseMove`, `Header_OnLButtonUp`, §2.6). Sections refer to
//! `docs/file-browser-findings.md`.
//!
//! The column layout is stored in `settings.json` (`preferences`) with
//! [`ColumnSet::to_json`] and [`ColumnSet::from_json`].
//!
//! X coordinates are relative to the header's left edge (column 0's left).

use serde_json::{Value, json};

/// A details view column. Also the sort key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Column {
    Name,
    Size,
    Type,
    Modified,
    /// The Unix permissions, where Windows shows its attribute letters.
    Attributes,
    /// Finder's comment (Linux: the `user.xdg.comment` attribute).
    Comment,
    Created,
    Accessed,
    Owner,
    Group,
    /// The Git branch of a folder that is a working tree. Not from Explorer.
    Branch,
}

impl Column {
    /// Every column, in the folder's own order (Windows 2000's `c_fs_cols`
    /// order for the first eight, as its header menu lists them).
    pub const ALL: [Column; 11] = [
        Column::Name,
        Column::Size,
        Column::Type,
        Column::Modified,
        Column::Attributes,
        Column::Comment,
        Column::Created,
        Column::Accessed,
        Column::Owner,
        Column::Group,
        Column::Branch,
    ];

    /// Shown in a new window. Windows shows Type where macsplorer shows
    /// Attributes.
    pub const DEFAULT: [Column; 4] = [Column::Name, Column::Size, Column::Modified, Column::Attributes];

    pub fn caption(self) -> &'static str {
        match self {
            Column::Name => "Name",
            Column::Size => "Size",
            Column::Type => "Type",
            Column::Modified => "Modified",
            Column::Attributes => "Attributes",
            Column::Comment => "Comment",
            Column::Created => "Created",
            Column::Accessed => "Accessed",
            Column::Owner => "Owner",
            Column::Group => "Group",
            Column::Branch => "Branch",
        }
    }

    /// Width when first shown. Guesses: Windows sizes these from each
    /// property's character count, not traced.
    pub fn default_width(self) -> f32 {
        match self {
            Column::Name => 300.0,
            Column::Size => 80.0,
            Column::Type => 140.0,
            Column::Modified | Column::Created | Column::Accessed => 120.0,
            Column::Attributes => 80.0,
            Column::Comment => 180.0,
            Column::Owner | Column::Group => 90.0,
            Column::Branch => 110.0,
        }
    }

    /// Numbers are right-aligned (`LVCFMT_RIGHT`).
    pub fn right_aligned(self) -> bool {
        self == Column::Size
    }

    /// The direction a first click on the heading sorts in: Size and Modified
    /// start with the biggest and the newest. Not traced from Explorer.
    pub fn first_sort_ascending(self) -> bool {
        !matches!(self, Column::Size | Column::Modified)
    }

    /// Listed in the header menu only while shown, otherwise only under
    /// "More..." (`SHCOLSTATE_SECONDARYUI`).
    pub fn secondary(self) -> bool {
        matches!(self, Column::Owner | Column::Group)
    }

    /// False for Branch on a machine without git, where the column is left
    /// out everywhere. (Tests don't depend on the machine having git.)
    pub fn available(self) -> bool {
        cfg!(test) || self != Column::Branch || crate::git::available()
    }

    /// Can't be hidden (`CM_STATE_ALWAYSVISIBLE`).
    pub fn always_visible(self) -> bool {
        self == Column::Name
    }

    fn index(self) -> usize {
        Column::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }

    /// The name a column is stored under, which stays the same whatever the
    /// caption says.
    pub fn key(self) -> &'static str {
        match self {
            Column::Name => "name",
            Column::Size => "size",
            Column::Type => "type",
            Column::Modified => "modified",
            Column::Attributes => "attributes",
            Column::Comment => "comment",
            Column::Created => "created",
            Column::Accessed => "accessed",
            Column::Owner => "owner",
            Column::Group => "group",
            Column::Branch => "branch",
        }
    }

    pub fn from_key(key: &str) -> Option<Column> {
        Column::ALL.into_iter().find(|c| c.key() == key)
    }
}

/// The widest a stored column width can be; a damaged file can't make a
/// column wider than any screen.
const MAX_STORED_WIDTH: f64 = 4000.0;

/// Header menu entries before "More..." takes over: Name plus 24
/// (`_AddColumnVerbsToMenu` stops at 25).
pub const MENU_LIMIT: usize = 25;

/// One header menu entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub column: Column,
    pub checked: bool,
    pub enabled: bool,
}

/// The details view's columns: every column in one order (the column
/// manager's list, which the header menu follows), which of them are shown,
/// and their widths. The shown columns appear in the list's order.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnSet {
    order: Vec<Column>,
    shown: Vec<bool>,
    widths: Vec<f32>,
}

impl Default for ColumnSet {
    fn default() -> Self {
        Self {
            order: Column::ALL.to_vec(),
            shown: Column::ALL.iter().map(|c| Column::DEFAULT.contains(c)).collect(),
            widths: Column::ALL.iter().map(|c| c.default_width()).collect(),
        }
    }
}

impl ColumnSet {
    /// The shown columns, left to right.
    pub fn visible(&self) -> Vec<Column> {
        self.order.iter().copied().filter(|&c| self.is_visible(c)).collect()
    }

    pub fn is_visible(&self, column: Column) -> bool {
        self.shown[column.index()] && column.available()
    }

    pub fn width(&self, column: Column) -> f32 {
        self.widths[column.index()]
    }

    pub fn set_width(&mut self, column: Column, width: f32) {
        self.widths[column.index()] = width.max(0.0);
    }

    /// Every column back to its default width. Which columns are shown, and
    /// their order, stay. Not from Explorer.
    pub fn reset_widths(&mut self) {
        self.widths = Column::ALL.iter().map(|c| c.default_width()).collect();
    }

    /// The shown columns' widths, left to right, for [`hit_test`].
    pub fn visible_widths(&self) -> Vec<f32> {
        self.visible().into_iter().map(|c| self.width(c)).collect()
    }

    /// The header menu (`CDefView::_AddColumnVerbsToMenu`): Name checked
    /// and grayed, then the other columns in list order, checked if shown.
    /// A secondary column is left out unless shown, and so is everything
    /// past [`MENU_LIMIT`]. The flag says whether "More..." follows, which
    /// it does when anything was left out.
    pub fn menu(&self) -> (Vec<MenuItem>, bool) {
        let mut items = vec![MenuItem { column: Column::Name, checked: true, enabled: false }];
        let mut more = false;
        for &column in self.order.iter().filter(|&&c| c != Column::Name && c.available()) {
            let checked = self.is_visible(column);
            if column.secondary() && !checked || items.len() >= MENU_LIMIT {
                more = true;
                continue;
            }
            items.push(MenuItem { column, checked, enabled: true });
        }
        (items, more)
    }

    /// A header menu click (`CDefView::ToggleVisibleColumn`): a shown column
    /// goes away; a hidden one is put at its index in the full list, or at
    /// the end if fewer columns are shown. Name stays.
    pub fn toggle(&mut self, column: Column) {
        let mut visible = self.visible();
        if let Some(i) = visible.iter().position(|&c| c == column) {
            if column.always_visible() {
                return;
            }
            visible.remove(i);
        } else {
            let at = self.order.iter().position(|&c| c == column).unwrap_or(0);
            visible.insert(at.min(visible.len()), column);
        }
        self.set_visible(&visible);
    }

    /// Shows exactly `columns`, in this order (`IColumnManager::SetColumns`).
    /// Hidden columns keep their place in the list as far as they can, so
    /// the header menu doesn't reshuffle. Inferred: the column manager's
    /// ordering of hidden columns wasn't traced.
    pub fn set_visible(&mut self, columns: &[Column]) {
        let mut columns: Vec<Column> = columns.to_vec();
        if !columns.contains(&Column::Name) {
            columns.insert(0, Column::Name);
        }
        let mut order = columns.clone();
        for (i, &c) in self.order.iter().enumerate() {
            if !columns.contains(&c) {
                order.insert(i.min(order.len()), c);
            }
        }
        for c in Column::ALL {
            self.shown[c.index()] = columns.contains(&c);
        }
        self.order = order;
    }

    /// The layout as stored in the settings file: the shown columns, left
    /// to right, each with its width. Hidden columns aren't stored.
    pub fn to_json(&self) -> Value {
        Value::Array(self.visible().into_iter().map(|c| json!({ "column": c.key(), "width": self.width(c) })).collect())
    }

    /// The layout stored by [`Self::to_json`]. Anything unreadable is
    /// skipped: unknown or repeated columns, a missing or absurd width (the
    /// column's default is used). An entry with `"shown": false`, as an
    /// earlier version wrote, counts as hidden. Name is always shown. `None`
    /// if the value isn't a list.
    pub fn from_json(value: &Value) -> Option<ColumnSet> {
        let mut set = ColumnSet::default();
        let mut shown = Vec::new();
        for entry in value.as_array()? {
            let Some(column) = entry.get("column").and_then(Value::as_str).and_then(Column::from_key) else { continue };
            if shown.contains(&column) || entry.get("shown").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            shown.push(column);
            if let Some(width) = entry.get("width").and_then(Value::as_f64).filter(|w| w.is_finite()) {
                set.widths[column.index()] = width.clamp(0.0, MAX_STORED_WIDTH) as f32;
            }
        }
        set.set_visible(&shown);
        Some(set)
    }

    /// The rows of the "More..." dialog (`CColumnDlg::_InitListView`): the
    /// shown columns checked, left to right, then the hidden ones unchecked
    /// and sorted by caption.
    pub fn dialog_rows(&self) -> Vec<(Column, bool)> {
        let mut hidden: Vec<Column> = self.order.iter().copied().filter(|&c| c.available() && !self.is_visible(c)).collect();
        hidden.sort_by_key(|c| c.caption().to_lowercase());
        self.visible().into_iter().map(|c| (c, true)).chain(hidden.into_iter().map(|c| (c, false))).collect()
    }
}

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
    fn size_and_modified_sort_descending_first() {
        assert!(Column::Name.first_sort_ascending());
        assert!(!Column::Size.first_sort_ascending());
        assert!(!Column::Modified.first_sort_ascending());
    }

    impl ColumnSet {
        fn widths(&self) -> Vec<f32> {
            Column::ALL.iter().map(|&c| self.width(c)).collect()
        }
    }

    fn names(v: &[Column]) -> Vec<&'static str> {
        v.iter().map(|c| c.caption()).collect()
    }

    #[test]
    fn reset_widths_keeps_the_shown_columns() {
        let mut set = ColumnSet::default();
        set.toggle(Column::Owner);
        set.set_width(Column::Name, 410.0);
        set.set_width(Column::Owner, 33.0);
        let shown = set.visible();
        set.reset_widths();
        assert_eq!(set.width(Column::Name), Column::Name.default_width());
        assert_eq!(set.width(Column::Owner), Column::Owner.default_width());
        assert_eq!(set.visible(), shown);
    }

    #[test]
    fn menu_lists_primary_columns_then_more() {
        let set = ColumnSet::default();
        let (items, more) = set.menu();
        let captions: Vec<_> = items.iter().map(|i| i.column.caption()).collect();
        assert_eq!(captions, ["Name", "Size", "Type", "Modified", "Attributes", "Comment", "Created", "Accessed", "Branch"]);
        assert_eq!(items[0], MenuItem { column: Column::Name, checked: true, enabled: false });
        assert!(items[1].checked && !items[2].checked && items[4].checked);
        assert!(more);
    }

    #[test]
    fn shown_secondary_columns_join_the_menu() {
        let mut set = ColumnSet::default();
        set.toggle(Column::Owner);
        let (items, more) = set.menu();
        assert!(items.iter().any(|i| i.column == Column::Owner && i.checked));
        // Group is still only under More...
        assert!(more);
    }

    #[test]
    fn toggling_puts_a_column_back_in_its_place() {
        let mut set = ColumnSet::default();
        set.toggle(Column::Size);
        assert_eq!(names(&set.visible()), ["Name", "Modified", "Attributes"]);
        set.toggle(Column::Size);
        assert_eq!(names(&set.visible()), ["Name", "Size", "Modified", "Attributes"]);
        // Type's index (2) is inside the shown columns, so it goes there.
        set.toggle(Column::Type);
        assert_eq!(names(&set.visible()), ["Name", "Size", "Type", "Modified", "Attributes"]);
        set.toggle(Column::Created);
        assert_eq!(names(&set.visible()), ["Name", "Size", "Type", "Modified", "Attributes", "Created"]);
        // The menu order didn't move.
        let (items, _) = set.menu();
        let captions: Vec<_> = items.iter().map(|i| i.column.caption()).collect();
        assert_eq!(captions, ["Name", "Size", "Type", "Modified", "Attributes", "Comment", "Created", "Accessed", "Branch"]);
    }

    #[test]
    fn a_layout_survives_the_settings_file() {
        let mut set = ColumnSet::default();
        set.set_visible(&[Column::Name, Column::Created, Column::Size, Column::Owner]);
        set.set_width(Column::Name, 410.0);
        set.set_width(Column::Owner, 33.0);
        let back = ColumnSet::from_json(&set.to_json()).unwrap();
        assert_eq!(names(&back.visible()), ["Name", "Created", "Size", "Owner"]);
        assert_eq!(back.widths(), set.widths());
        assert_eq!(back.width(Column::Name), 410.0);
        assert_eq!(back.width(Column::Owner), 33.0);
    }

    #[test]
    fn only_shown_columns_are_stored() {
        let json = ColumnSet::default().to_json();
        let stored: Vec<_> = json.as_array().unwrap().iter().map(|e| e["column"].as_str().unwrap()).collect();
        assert_eq!(stored, ["name", "size", "modified", "attributes"]);
        assert_eq!(json[0], json!({ "column": "name", "width": 300.0 }));
    }

    #[test]
    fn the_default_layout_round_trips() {
        let set = ColumnSet::default();
        assert_eq!(ColumnSet::from_json(&set.to_json()), Some(set));
    }

    #[test]
    fn hidden_columns_come_back_hidden_with_default_widths() {
        let mut set = ColumnSet::default();
        set.set_width(Column::Type, 12.0);
        let back = ColumnSet::from_json(&set.to_json()).unwrap();
        assert!(!back.is_visible(Column::Type));
        assert_eq!(back.width(Column::Type), Column::Type.default_width());
        // They are all still in the header menu or under More...
        assert_eq!(back.menu().0.len(), 9);
    }

    #[test]
    fn unreadable_entries_are_skipped() {
        let value = json!([
            { "column": "size", "width": 55 },
            { "column": "bogus" },
            { "column": "size", "width": 1 },
            { "column": "modified", "width": "wide" },
            "nonsense",
            { "column": "type", "width": -5 },
            { "column": "created", "width": 1e9 },
        ]);
        let set = ColumnSet::from_json(&value).unwrap();
        // Name was never mentioned: it is added in front, shown.
        assert_eq!(names(&set.visible()), ["Name", "Size", "Modified", "Type", "Created"]);
        assert_eq!(set.width(Column::Size), 55.0);
        assert_eq!(set.width(Column::Type), 0.0);
        assert_eq!(set.width(Column::Created), 4000.0);
        assert_eq!(set.width(Column::Modified), Column::Modified.default_width());
    }

    #[test]
    fn entries_marked_hidden_by_the_earlier_format_stay_hidden() {
        let value = json!([
            { "column": "name", "shown": true, "width": 200 },
            { "column": "size", "shown": false, "width": 80 },
            { "column": "owner", "shown": true, "width": 90 },
        ]);
        let set = ColumnSet::from_json(&value).unwrap();
        assert_eq!(names(&set.visible()), ["Name", "Owner"]);
    }

    #[test]
    fn name_is_shown_even_if_the_file_leaves_it_out() {
        let set = ColumnSet::from_json(&json!([{ "column": "name", "shown": false }, { "column": "size" }])).unwrap();
        assert_eq!(names(&set.visible()), ["Name", "Size"]);
        assert!(ColumnSet::from_json(&json!([])).unwrap().is_visible(Column::Name));
    }

    #[test]
    fn something_other_than_a_list_is_rejected() {
        assert_eq!(ColumnSet::from_json(&json!({ "name": 1 })), None);
        assert_eq!(ColumnSet::from_json(&Value::Null), None);
    }

    #[test]
    fn name_cannot_be_hidden() {
        let mut set = ColumnSet::default();
        set.toggle(Column::Name);
        assert!(set.is_visible(Column::Name));
        set.set_visible(&[Column::Size]);
        assert_eq!(names(&set.visible()), ["Name", "Size"]);
    }

    #[test]
    fn dialog_lists_shown_then_hidden_by_caption() {
        let mut set = ColumnSet::default();
        set.set_visible(&[Column::Modified, Column::Name, Column::Size]);
        let rows: Vec<_> = set.dialog_rows().into_iter().map(|(c, on)| (c.caption(), on)).collect();
        assert_eq!(
            rows,
            [
                ("Modified", true),
                ("Name", true),
                ("Size", true),
                ("Accessed", false),
                ("Attributes", false),
                ("Branch", false),
                ("Comment", false),
                ("Created", false),
                ("Group", false),
                ("Owner", false),
                ("Type", false),
            ]
        );
    }

    #[test]
    fn widths_follow_the_visible_order() {
        let mut set = ColumnSet::default();
        set.set_width(Column::Modified, 50.0);
        set.toggle(Column::Size);
        assert_eq!(set.visible_widths(), [300.0, 50.0, 80.0]);
    }

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
