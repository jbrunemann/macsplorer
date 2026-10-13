//! Back/forward navigation history, Explorer's travel log (`CTravelLog`).
//!
//! The entry limit is a placeholder. The real `CTravelLog` limit has not been
//! traced yet.

use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 100;

#[derive(Debug, Default)]
pub struct History {
    entries: Vec<PathBuf>,
    cursor: usize,
}

impl History {
    pub fn new(start: PathBuf) -> Self {
        Self { entries: vec![start], cursor: 0 }
    }

    pub fn current(&self) -> &Path {
        &self.entries[self.cursor]
    }

    /// Records a navigation. Forward entries are dropped, as in a browser.
    /// Navigating to the current location is a no-op.
    pub fn push(&mut self, path: PathBuf) {
        if self.current() == path {
            return;
        }
        self.entries.truncate(self.cursor + 1);
        self.entries.push(path);
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }

    pub fn can_go_back(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn back(&mut self) -> Option<&Path> {
        self.can_go_back().then(|| {
            self.cursor -= 1;
            self.current()
        })
    }

    pub fn forward(&mut self) -> Option<&Path> {
        self.can_go_forward().then(|| {
            self.cursor += 1;
            self.current()
        })
    }

    /// Entries behind the cursor, most recent first, for the back dropdown.
    pub fn back_entries(&self) -> impl Iterator<Item = (usize, &Path)> {
        (0..self.cursor).rev().map(|i| (i, self.entries[i].as_path()))
    }

    /// Entries ahead of the cursor, nearest first, for the forward dropdown.
    pub fn forward_entries(&self) -> impl Iterator<Item = (usize, &Path)> {
        (self.cursor + 1..self.entries.len()).map(|i| (i, self.entries[i].as_path()))
    }

    /// Jumps to an index returned by `back_entries` or `forward_entries`.
    pub fn go_to(&mut self, index: usize) -> Option<&Path> {
        (index < self.entries.len()).then(|| {
            self.cursor = index;
            self.current()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn back_forward_and_truncation() {
        let mut h = History::new(p("/a"));
        h.push(p("/b"));
        h.push(p("/c"));
        assert_eq!(h.back(), Some(Path::new("/b")));
        assert_eq!(h.back(), Some(Path::new("/a")));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some(Path::new("/b")));
        h.push(p("/d"));
        assert!(!h.can_go_forward());
        let back: Vec<_> = h.back_entries().map(|(_, p)| p.to_owned()).collect();
        assert_eq!(back, [p("/b"), p("/a")]);
    }

    #[test]
    fn same_location_is_not_recorded_twice() {
        let mut h = History::new(p("/a"));
        h.push(p("/a"));
        assert!(!h.can_go_back());
    }

    #[test]
    fn caps_length() {
        let mut h = History::new(p("/0"));
        for i in 1..=150 {
            h.push(PathBuf::from(format!("/{i}")));
        }
        assert_eq!(h.back_entries().count(), MAX_ENTRIES - 1);
    }
}
