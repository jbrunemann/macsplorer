//! Type-ahead (incremental) search in the item view.
//!
//! Typing letters in the list selects the first item whose name starts with
//! them. Traced from comctl32 v6.10 (`CLVKeyboardManager::OnChar`,
//! `IsISearchTimedOut`, `CLVItemStore::OnFindItem`), see
//! `docs/file-browser-findings.md` §2.9.

/// Longest pause between two characters that still extends the search.
/// `IsISearchTimedOut` compares message times against a fixed 1000 ms.
pub const TIMEOUT_MS: u64 = 1000;

/// What a typed character did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Select and focus the item at this position in `names`.
    Found(usize),
    /// Nothing starts with the typed text. The view keeps its selection.
    /// (Windows beeps once per search here, `MessageBeep(0)`.)
    NotFound,
    /// The character isn't part of a search.
    Ignored,
}

/// The characters typed so far and when the last one arrived.
#[derive(Debug, Default)]
pub struct IncrementalSearch {
    text: String,
    last_ms: u64,
}

impl IncrementalSearch {
    /// Forgets the typed text. Explorer does this for control characters
    /// (Backspace, Enter, Esc, Tab) and after any arrow/Home/End/Page key
    /// moved the focus.
    pub fn reset(&mut self) {
        self.text.clear();
    }

    /// Adds `c`, typed at `now_ms`, and looks for a match in `names` (the
    /// displayed names in view order). `focus` is the focused item's position.
    ///
    /// - After a pause longer than [`TIMEOUT_MS`] the text starts over.
    /// - The first character searches from the item after the focus, so
    ///   pressing the same letter again steps through the items with that
    ///   letter. Typing only repeats of one letter ("aaa") does the same, with
    ///   just that letter as the text.
    /// - Any other text searches from the focused item itself, so extending
    ///   the text keeps the current item while it still matches.
    /// - The search wraps around the end of the list.
    /// - A space can't start a search (Explorer uses it to toggle the focused
    ///   item's selection instead); inside the text it is a normal character.
    pub fn type_char(&mut self, c: char, now_ms: u64, names: &[&str], focus: Option<usize>) -> Outcome {
        if c.is_control() {
            self.reset();
            return Outcome::Ignored;
        }
        if now_ms.saturating_sub(self.last_ms) > TIMEOUT_MS {
            self.text.clear();
        }
        self.last_ms = now_ms;
        if self.text.is_empty() && c == ' ' {
            return Outcome::Ignored;
        }
        self.text.push(c);

        let first = self.text.chars().next().unwrap_or(c);
        let repeated = self.text.chars().all(|t| t == first);
        let (query, from_focus) = if repeated { (c.to_string(), false) } else { (self.text.clone(), true) };

        let n = names.len();
        if n == 0 {
            return Outcome::NotFound;
        }
        let start = match (focus.filter(|&f| f < n), from_focus) {
            (Some(f), true) => f,
            (Some(f), false) => f + 1,
            (None, _) => 0,
        };
        (0..n)
            .map(|k| (start + k) % n)
            .find(|&i| starts_with_ignore_case(names[i], &query))
            .map_or(Outcome::NotFound, Outcome::Found)
    }
}

/// `CompareStringW` on the first characters with `NORM_IGNORECASE`, user locale.
fn starts_with_ignore_case(name: &str, prefix: &str) -> bool {
    let mut name = name.chars().flat_map(char::to_lowercase);
    prefix.chars().flat_map(char::to_lowercase).all(|p| name.next() == Some(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 6] = ["Applications", "archive.zip", "Bin", "build.rs", "Desktop", "docs"];

    fn typed(s: &mut IncrementalSearch, text: &str, mut t: u64, focus: Option<usize>) -> Outcome {
        let mut out = Outcome::Ignored;
        for c in text.chars() {
            out = s.type_char(c, t, &NAMES, focus);
            t += 100;
        }
        out
    }

    #[test]
    fn first_letter_selects_first_match_after_focus() {
        let mut s = IncrementalSearch::default();
        assert_eq!(typed(&mut s, "d", 0, None), Outcome::Found(4));
        s.reset();
        assert_eq!(typed(&mut s, "b", 0, Some(0)), Outcome::Found(2));
    }

    #[test]
    fn same_letter_again_steps_to_the_next_item_and_wraps() {
        let mut s = IncrementalSearch::default();
        assert_eq!(s.type_char('a', 0, &NAMES, None), Outcome::Found(0));
        assert_eq!(s.type_char('a', 100, &NAMES, Some(0)), Outcome::Found(1));
        assert_eq!(s.type_char('a', 200, &NAMES, Some(1)), Outcome::Found(0));
    }

    #[test]
    fn the_focused_item_is_found_again_when_it_is_the_only_match() {
        let mut s = IncrementalSearch::default();
        assert_eq!(s.type_char('e', 0, &["x", "e"], Some(1)), Outcome::Found(1));
    }

    #[test]
    fn more_letters_search_from_the_focused_item() {
        let mut s = IncrementalSearch::default();
        assert_eq!(s.type_char('a', 0, &NAMES, None), Outcome::Found(0));
        assert_eq!(s.type_char('r', 100, &NAMES, Some(0)), Outcome::Found(1));
        assert_eq!(s.type_char('c', 200, &NAMES, Some(1)), Outcome::Found(1));
        assert_eq!(s.type_char('x', 300, &NAMES, Some(1)), Outcome::NotFound);
    }

    #[test]
    fn matching_ignores_case() {
        let mut s = IncrementalSearch::default();
        assert_eq!(typed(&mut s, "DESK", 0, None), Outcome::Found(4));
        s.reset();
        assert_eq!(typed(&mut s, "BUI", 0, None), Outcome::Found(3));
    }

    #[test]
    fn a_pause_starts_a_new_search() {
        let mut s = IncrementalSearch::default();
        assert_eq!(s.type_char('b', 0, &NAMES, None), Outcome::Found(2));
        // Within a second the text is "bu"; after more than one it is just "u".
        assert_eq!(s.type_char('u', 1000, &NAMES, Some(2)), Outcome::Found(3));
        assert_eq!(s.type_char('u', 2001, &NAMES, Some(3)), Outcome::NotFound);
    }

    #[test]
    fn control_characters_reset() {
        let mut s = IncrementalSearch::default();
        assert_eq!(s.type_char('b', 0, &NAMES, None), Outcome::Found(2));
        assert_eq!(s.type_char('\u{8}', 50, &NAMES, Some(2)), Outcome::Ignored);
        // Starts over: "u" alone, not "bu".
        assert_eq!(s.type_char('u', 100, &NAMES, Some(2)), Outcome::NotFound);
    }

    #[test]
    fn space_only_counts_inside_the_text() {
        let names = ["My File", "my notes", "Mz"];
        let mut s = IncrementalSearch::default();
        assert_eq!(s.type_char(' ', 0, &names, None), Outcome::Ignored);
        assert_eq!(s.type_char('m', 100, &names, None), Outcome::Found(0));
        assert_eq!(s.type_char('y', 200, &names, Some(0)), Outcome::Found(0));
        assert_eq!(s.type_char(' ', 300, &names, Some(0)), Outcome::Found(0));
        assert_eq!(s.type_char('n', 400, &names, Some(0)), Outcome::Found(1));
    }

    #[test]
    fn empty_list_and_stale_focus() {
        let mut s = IncrementalSearch::default();
        assert_eq!(s.type_char('a', 0, &[], None), Outcome::NotFound);
        s.reset();
        assert_eq!(s.type_char('b', 0, &NAMES, Some(99)), Outcome::Found(2));
    }
}
