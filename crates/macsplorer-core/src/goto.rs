//! Go To: finding a folder to jump to by typing a few letters of its name.
//!
//! Not from Explorer, which has no such box (its address bar drops down a
//! list of typed history, `CAutoComplete`). This follows the quick openers of
//! the terminal file managers (ranger's `fzf` jump, yazi's `z`, Midnight
//! Commander's hotlist): the candidates are the places the user is likely to
//! mean, and the typed letters only have to appear in order in the name.

use std::path::{Path, PathBuf};

use crate::natural::fold;

/// Where a candidate came from, which is also the order they are offered in
/// when several match equally well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Visited in this window, most recent first.
    Recent,
    /// A sidebar place: favorites and volumes.
    Place,
    /// A folder open in the Folders pane.
    Open,
    /// A folder inside the open one.
    Inside,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    pub path: PathBuf,
    pub source: Source,
}

impl Candidate {
    pub fn new(label: impl Into<String>, path: impl Into<PathBuf>, source: Source) -> Self {
        Self { label: label.into(), path: path.into(), source }
    }

    /// A candidate named after the last part of its path.
    pub fn named(path: &Path, source: Source) -> Self {
        let label = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        Self::new(label, path, source)
    }
}

/// Drops later candidates for a path already offered, so each folder shows
/// once, under the first source that had it.
pub fn dedupe(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = std::collections::HashSet::new();
    candidates.into_iter().filter(|c| seen.insert(c.path.clone())).collect()
}

/// The positions in `candidates` that match `query`, best first. An empty
/// query matches all of them, in their given order. Matching is on the name;
/// a query with a `/` in it, or one the name doesn't match, is tried against
/// the whole path (less well). Names compare in the form `natural::fold` gives.
pub fn rank(query: &str, candidates: &[Candidate]) -> Vec<usize> {
    let query = fold(query.trim());
    if query.is_empty() {
        return (0..candidates.len()).collect();
    }
    let mut scored: Vec<(u32, usize)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let by_name = if query.contains('/') { None } else { score(&query, &fold(&c.label)) };
            by_name.or_else(|| score(&query, &fold(&c.path.to_string_lossy())).map(|s| s / 4)).map(|s| (s, i))
        })
        .collect();
    // Stable: equal scores keep the order the candidates came in.
    scored.sort_by_key(|&(s, _)| std::cmp::Reverse(s));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// How well `query` matches `text`, both already folded; `None` if its
/// letters don't all appear in order. An exact match beats a prefix, a
/// prefix a word start, a run of letters a scattered match.
fn score(query: &str, text: &str) -> Option<u32> {
    if query == text {
        return Some(10_000);
    }
    if text.starts_with(query) {
        return Some(8_000 - text.chars().count() as u32);
    }
    let q: Vec<char> = query.chars().collect();
    let t: Vec<char> = text.chars().collect();
    if let Some(at) = find_run(&q, &t) {
        let bonus = if at == 0 || !t[at - 1].is_alphanumeric() { 1_000 } else { 0 };
        return Some(5_000 + bonus - at as u32);
    }
    // Scattered: the letters in order, rewarding word starts and neighbors.
    let mut score = 1_000u32;
    let mut next = 0;
    let mut last: Option<usize> = None;
    for (i, &c) in t.iter().enumerate() {
        if next < q.len() && c == q[next] {
            if i == 0 || !t[i - 1].is_alphanumeric() {
                score += 40;
            }
            if last.is_some_and(|l| l + 1 == i) {
                score += 20;
            }
            last = Some(i);
            next += 1;
        }
    }
    (next == q.len()).then(|| score.saturating_sub(t.len() as u32))
}

fn find_run(q: &[char], t: &[char]) -> Option<usize> {
    if q.len() > t.len() {
        return None;
    }
    (0..=t.len() - q.len()).find(|&at| t[at..at + q.len()] == *q)
}

/// Moves a highlighted row by `delta` through `len` rows, stopping at the
/// ends (a list box doesn't wrap).
pub fn step(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    current.saturating_add_signed(delta).min(len - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands(names: &[&str]) -> Vec<Candidate> {
        names.iter().map(|n| Candidate::new(*n, format!("/u/{n}"), Source::Recent)).collect()
    }

    fn names(query: &str, c: &[Candidate]) -> Vec<String> {
        rank(query, c).into_iter().map(|i| c[i].label.clone()).collect()
    }

    #[test]
    fn empty_query_keeps_order() {
        let c = cands(&["b", "a", "c"]);
        assert_eq!(names("", &c), ["b", "a", "c"]);
        assert_eq!(names("  ", &c), ["b", "a", "c"]);
    }

    #[test]
    fn letters_must_appear_in_order() {
        let c = cands(&["Documents", "Downloads", "Desktop"]);
        assert_eq!(names("dwn", &c), ["Downloads"]);
        assert_eq!(names("sdt", &c), Vec::<String>::new());
    }

    #[test]
    fn ignores_case_and_composition() {
        let c = cands(&["Über uns", "Applications"]);
        assert_eq!(names("ÜBER", &c), ["Über uns"]);
        // "u" + combining diaeresis, as HFS+ stores it.
        assert_eq!(names("u\u{308}ber", &c), ["Über uns"]);
    }

    #[test]
    fn better_matches_come_first() {
        let c = cands(&["my-docs-old", "old", "folder", "docs", "d-o-c-s"]);
        // Exact, then prefix, then word start, then a run inside, then scattered.
        assert_eq!(names("docs", &c), ["docs", "my-docs-old", "d-o-c-s"]);
        assert_eq!(names("old", &c), ["old", "my-docs-old", "folder"]);
    }

    #[test]
    fn equal_scores_keep_given_order() {
        let c = cands(&["src-b", "src-a"]);
        assert_eq!(names("src", &c).len(), 2);
        let c = vec![Candidate::new("x", "/1/x", Source::Recent), Candidate::new("x", "/2/x", Source::Place)];
        assert_eq!(rank("x", &c), [0, 1]);
    }

    #[test]
    fn slash_matches_the_path() {
        let c = vec![Candidate::new("src", "/work/app/src", Source::Recent), Candidate::new("src", "/work/lib/src", Source::Recent)];
        assert_eq!(rank("lib/src", &c), [1]);
        assert_eq!(rank("work", &c), [0, 1], "a name miss falls back to the path");
    }

    #[test]
    fn dedupe_keeps_first_source() {
        let c = dedupe(vec![
            Candidate::named(Path::new("/a/b"), Source::Recent),
            Candidate::named(Path::new("/a/c"), Source::Place),
            Candidate::named(Path::new("/a/b"), Source::Open),
        ]);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].source, Source::Recent);
    }

    #[test]
    fn named_uses_last_component() {
        assert_eq!(Candidate::named(Path::new("/a/b"), Source::Open).label, "b");
        assert_eq!(Candidate::named(Path::new("/"), Source::Open).label, "/");
    }

    #[test]
    fn step_stops_at_the_ends() {
        assert_eq!(step(0, -1, 3), 0);
        assert_eq!(step(1, 1, 3), 2);
        assert_eq!(step(2, 1, 3), 2);
        assert_eq!(step(0, 5, 0), 0);
    }
}
