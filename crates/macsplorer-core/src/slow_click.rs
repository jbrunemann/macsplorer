//! Rename by "slow double-click": click the name of the selected item once,
//! wait, and its label turns into an edit box.
//!
//! Traced from comctl32 v6.10 (`CLVMouseManager::HandleMouse`,
//! `CLVInPlaceEditingManager::SetupPendingNameEdit` / `CancelPendingEdit`,
//! `CListView::OnTimer`), see `docs/file-browser-findings.md` §2.1.1.

/// How long after the click the rename starts. Windows waits
/// `GetDoubleClickTime()` (500 ms by default), so a second click inside the
/// wait is a double-click (open) rather than a rename. The app uses the same
/// value as its double-click interval to keep that true.
pub const DELAY_MS: u64 = 500;

/// One finished left click on an item, as the view saw it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Click {
    /// On the label text, not the icon.
    pub on_label: bool,
    /// The clicked item was the only selected item before this click.
    pub only_selected: bool,
    /// The item view had keyboard focus before this click.
    pub view_focused: bool,
    /// Cmd (Ctrl on Windows) or Shift was held.
    pub modifiers: bool,
}

impl Click {
    /// Whether this click arms a rename. Explorer also requires a plain left
    /// click that didn't become a drag; callers only report those.
    pub fn arms_rename(&self) -> bool {
        self.on_label && self.only_selected && self.view_focused && !self.modifiers
    }
}

/// A rename waiting for its delay to pass. `K` identifies the item.
#[derive(Debug)]
pub struct SlowClick<K> {
    delay_ms: u64,
    pending: Option<(K, u64)>,
}

impl<K> Default for SlowClick<K> {
    fn default() -> Self {
        Self::new(DELAY_MS)
    }
}

impl<K> SlowClick<K> {
    pub fn new(delay_ms: u64) -> Self {
        Self { delay_ms, pending: None }
    }

    /// A left click on `item` ended at `now_ms`. Arms the rename if the
    /// click qualifies; any other click drops a pending one.
    pub fn click(&mut self, item: K, click: Click, now_ms: u64) {
        self.pending = click.arms_rename().then(|| (item, now_ms + self.delay_ms));
    }

    /// Drops a pending rename: a double-click, a key press, a drag, or
    /// anything that changes the folder or selection.
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// The item to rename once the delay has passed (only once).
    pub fn due(&mut self, now_ms: u64) -> Option<K> {
        match self.pending {
            Some((_, at)) if now_ms >= at => self.pending.take().map(|(item, _)| item),
            _ => None,
        }
    }

    /// Milliseconds until a pending rename is due, for scheduling a repaint.
    pub fn wait_ms(&self, now_ms: u64) -> Option<u64> {
        self.pending.as_ref().map(|(_, at)| at.saturating_sub(now_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARMING: Click = Click { on_label: true, only_selected: true, view_focused: true, modifiers: false };

    #[test]
    fn fires_after_the_delay() {
        let mut s = SlowClick::default();
        s.click("a", ARMING, 1000);
        assert_eq!(s.wait_ms(1000), Some(DELAY_MS));
        assert_eq!(s.due(1000 + DELAY_MS - 1), None);
        assert_eq!(s.due(1000 + DELAY_MS), Some("a"));
        assert_eq!(s.due(5000), None, "fires once");
        assert_eq!(s.wait_ms(5000), None);
    }

    #[test]
    fn delay_is_configurable() {
        let mut s = SlowClick::new(800);
        s.click("a", ARMING, 0);
        assert_eq!(s.due(799), None);
        assert_eq!(s.due(800), Some("a"));
    }

    #[test]
    fn needs_every_condition() {
        for click in [
            Click { on_label: false, ..ARMING },
            Click { only_selected: false, ..ARMING },
            Click { view_focused: false, ..ARMING },
            Click { modifiers: true, ..ARMING },
        ] {
            let mut s = SlowClick::default();
            s.click("a", click, 0);
            assert_eq!(s.due(10_000), None, "{click:?}");
        }
    }

    #[test]
    fn other_clicks_and_cancel_drop_it() {
        let mut s = SlowClick::default();
        s.click("a", ARMING, 0);
        s.click("b", Click { only_selected: false, ..ARMING }, 100);
        assert_eq!(s.due(10_000), None);

        s.click("a", ARMING, 0);
        s.cancel();
        assert_eq!(s.due(10_000), None);
    }

    #[test]
    fn a_new_arming_click_restarts_the_wait() {
        let mut s = SlowClick::default();
        s.click("a", ARMING, 0);
        s.click("a", ARMING, 300);
        assert_eq!(s.due(DELAY_MS), None);
        assert_eq!(s.due(300 + DELAY_MS), Some("a"));
    }
}
