//! Moving the keyboard focus between the window's panes with Tab and F6.
//!
//! After `CShellBrowser::_CycleFocus` and `_MayGetNextToolbarFocus` in
//! ExplorerFrame.dll, and `IsVK_TABCycler`, see
//! `docs/file-browser-findings.md` §2.12. The frame keeps its panes in a
//! list. Tab steps from the view to the first pane that can take the focus,
//! on through the rest, and from the last one back to the view; Shift+Tab
//! goes the other way.

/// A place the focus cycle stops at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The item view (Explorer's `CDefView`), where the cycle starts and ends.
    View,
    /// The address bar, in edit mode with its text selected.
    Address,
    /// The search field beside it.
    Search,
    /// The Folders pane.
    Folders,
}

/// The panes after the view, in the order Tab visits them. **Inferred**
/// from Windows 2000, where the rebar with the address bar comes before the
/// Folders explorer bar; build 10240's toolbar order wasn't decoded. Inside
/// the rebar the bands go left to right (`CBandSite::_GetNextTabstopBand`),
/// so the search field follows the address bar.
pub const PANES: [Stop; 3] = [Stop::Address, Stop::Search, Stop::Folders];

/// Which way a key press moves the focus, if it is a cycling key
/// (`IsVK_TABCycler`): Tab or F6 while Ctrl is up, backwards with Shift.
/// `None` for any other key or with Ctrl held (Ctrl+Tab is left to the
/// view). Macsplorer also leaves out Alt and the Command key, whose Tab
/// chords belong to the system.
pub fn direction(tab_or_f6: bool, ctrl: bool, alt: bool, shift: bool) -> Option<bool> {
    (tab_or_f6 && !ctrl && !alt).then_some(!shift)
}

/// Where the focus goes from `from`: the next pane in [`PANES`] (the
/// previous one with `forward` false) for which `available` says yes, or
/// the view when none is left. From the view, forward starts at the first
/// pane and backward at the last.
pub fn next(from: Stop, forward: bool, available: impl Fn(Stop) -> bool) -> Stop {
    let at = PANES.iter().position(|&s| s == from);
    let rest: Vec<Stop> = match (at, forward) {
        (None, true) => PANES.to_vec(),
        (None, false) => PANES.iter().rev().copied().collect(),
        (Some(i), true) => PANES[i + 1..].to_vec(),
        (Some(i), false) => PANES[..i].iter().rev().copied().collect(),
    };
    rest.into_iter().find(|&s| available(s)).unwrap_or(Stop::View)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_and_f6_cycle_unless_ctrl_is_down() {
        assert_eq!(direction(true, false, false, false), Some(true));
        assert_eq!(direction(true, false, false, true), Some(false));
        assert_eq!(direction(true, true, false, false), None);
        assert_eq!(direction(true, false, true, false), None);
        assert_eq!(direction(false, false, false, false), None);
    }

    #[test]
    fn forward_visits_every_pane_and_returns_to_the_view() {
        let all = |_| true;
        let mut at = Stop::View;
        let mut seen = Vec::new();
        for _ in 0..4 {
            at = next(at, true, all);
            seen.push(at);
        }
        assert_eq!(seen, [Stop::Address, Stop::Search, Stop::Folders, Stop::View]);
    }

    #[test]
    fn backward_runs_the_other_way() {
        let all = |_| true;
        assert_eq!(next(Stop::View, false, all), Stop::Folders);
        assert_eq!(next(Stop::Folders, false, all), Stop::Search);
        assert_eq!(next(Stop::Address, false, all), Stop::View);
    }

    #[test]
    fn hidden_panes_are_skipped() {
        let no_folders = |s| s != Stop::Folders;
        assert_eq!(next(Stop::Search, true, no_folders), Stop::View);
        assert_eq!(next(Stop::View, false, no_folders), Stop::Search);
        assert_eq!(next(Stop::View, true, |s| s == Stop::Folders), Stop::Folders);
        assert_eq!(next(Stop::View, true, |_| false), Stop::View);
    }
}
