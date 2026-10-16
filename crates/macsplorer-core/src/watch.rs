//! Live updates: which of the folders on screen changed on disk.
//!
//! [`FolderWatcher`] watches the open folder and the Folders pane's expanded
//! folders (each without its subfolders) through `notify`: FSEvents on macOS,
//! inotify on Linux. A burst of changes reloads a folder once, after
//! [`QUIET`] without further changes; a folder that keeps changing (a
//! download being written) reloads every [`MAX_WAIT`]. Both delays are
//! guesses, not traced: Explorer gets these updates from `SHChangeNotify`
//! and the file system's change notifications, and its coalescing wasn't
//! looked at.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Seconds without changes before a folder reloads.
pub const QUIET: f64 = 0.2;
/// Seconds a folder that keeps changing waits at most.
pub const MAX_WAIT: f64 = 1.0;

/// Changed folders waiting for their pause, by when they first and last
/// changed (in seconds on any clock).
#[derive(Debug, Default)]
pub struct Settle {
    pending: HashMap<PathBuf, (f64, f64)>,
}

impl Settle {
    pub fn note(&mut self, dir: PathBuf, now: f64) {
        self.pending.entry(dir).and_modify(|(_, last)| *last = now).or_insert((now, now));
    }

    fn due_at((first, last): (f64, f64)) -> f64 {
        (last + QUIET).min(first + MAX_WAIT)
    }

    /// Removes and returns the folders whose wait is over.
    pub fn take_due(&mut self, now: f64) -> Vec<PathBuf> {
        let mut due: Vec<PathBuf> = self.pending.iter().filter(|(_, t)| Self::due_at(**t) <= now).map(|(p, _)| p.clone()).collect();
        due.sort();
        for dir in &due {
            self.pending.remove(dir);
        }
        due
    }

    /// When the next folder is due, if any is waiting.
    pub fn next_due(&self) -> Option<f64> {
        self.pending.values().map(|t| Self::due_at(*t)).min_by(f64::total_cmp)
    }
}

/// What the watcher thread reports.
enum Report {
    Paths(Vec<PathBuf>),
    /// Events were lost: everything watched may have changed.
    Rescan,
}

/// Watches a set of folders for added, removed, renamed and changed items.
pub struct FolderWatcher {
    /// `None` when the system watcher couldn't be set up; nothing updates then.
    watcher: Option<RecommendedWatcher>,
    /// The folders as asked for, and the canonical path FSEvents reports
    /// them by (`/private/var/...` for `/var/...`).
    watched: BTreeMap<PathBuf, PathBuf>,
    rx: Receiver<Report>,
    settle: Settle,
}

impl FolderWatcher {
    /// `wake` runs on the watcher's thread after each change, to get
    /// [`poll`](Self::poll) called.
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            let report = if event.need_rescan() {
                Report::Rescan
            } else if matches!(event.kind, EventKind::Access(_)) {
                return;
            } else {
                Report::Paths(event.paths)
            };
            if tx.send(report).is_ok() {
                wake();
            }
        });
        Self { watcher: watcher.ok(), watched: BTreeMap::new(), rx, settle: Settle::default() }
    }

    /// Watches exactly `dirs`. Cheap when nothing changed.
    pub fn watch_only(&mut self, dirs: &BTreeSet<PathBuf>) {
        if self.watched.keys().eq(dirs.iter()) {
            return;
        }
        let Some(watcher) = &mut self.watcher else { return };
        // One batch, so FSEvents restarts its stream once.
        let mut paths = watcher.paths_mut();
        self.watched.retain(|dir, _| {
            let keep = dirs.contains(dir);
            if !keep {
                let _ = paths.remove(dir);
            }
            keep
        });
        for dir in dirs {
            if !self.watched.contains_key(dir) {
                // A folder that can't be watched is still remembered, so it
                // isn't retried every frame.
                let _ = paths.add(dir, RecursiveMode::NonRecursive);
                self.watched.insert(dir.clone(), dir.canonicalize().unwrap_or_else(|_| dir.clone()));
            }
        }
        let _ = paths.commit();
        self.settle.pending.retain(|dir, _| dirs.contains(dir));
    }

    /// Takes in the reported changes and returns the watched folders that
    /// are due for a reload at `now` (seconds).
    pub fn poll(&mut self, now: f64) -> Vec<PathBuf> {
        let mut by_path: HashMap<&Path, Vec<&PathBuf>> = HashMap::new();
        for (dir, canonical) in &self.watched {
            by_path.entry(dir).or_default().push(dir);
            if canonical != dir {
                by_path.entry(canonical).or_default().push(dir);
            }
        }
        let mut changed = BTreeSet::new();
        while let Ok(report) = self.rx.try_recv() {
            match report {
                Report::Rescan => changed.extend(self.watched.keys().cloned()),
                Report::Paths(paths) => {
                    for path in &paths {
                        // The folder itself, or an item in it.
                        let dirs = by_path.get(path.as_path()).or_else(|| path.parent().and_then(|p| by_path.get(p)));
                        changed.extend(dirs.into_iter().flatten().map(|d| (*d).clone()));
                    }
                }
            }
        }
        for dir in changed {
            self.settle.note(dir, now);
        }
        self.settle.take_due(now)
    }

    /// When [`poll`](Self::poll) will next have a folder to return.
    pub fn next_due(&self) -> Option<f64> {
        self.settle.next_due()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};

    #[test]
    fn a_burst_settles_after_a_quiet_pause() {
        let mut s = Settle::default();
        let a = PathBuf::from("/a");
        s.note(a.clone(), 0.0);
        s.note(a.clone(), 0.1);
        assert!(s.take_due(0.25).is_empty());
        assert_eq!(s.next_due(), Some(0.1 + QUIET));
        assert_eq!(s.take_due(0.35), vec![a]);
        assert_eq!(s.next_due(), None);
    }

    #[test]
    fn a_folder_that_keeps_changing_still_reloads() {
        let mut s = Settle::default();
        let a = PathBuf::from("/a");
        let mut t = 0.0;
        while t < MAX_WAIT {
            s.note(a.clone(), t);
            assert!(s.take_due(t).is_empty());
            t += 0.1;
        }
        assert_eq!(s.take_due(MAX_WAIT), vec![a]);
    }

    #[test]
    fn folders_wait_separately() {
        let mut s = Settle::default();
        s.note("/a".into(), 0.0);
        s.note("/b".into(), 0.5);
        assert_eq!(s.take_due(0.5), vec![PathBuf::from("/a")]);
        assert_eq!(s.take_due(0.8), vec![PathBuf::from("/b")]);
    }

    #[test]
    fn reports_changes_in_watched_folders_only() {
        let base = std::env::temp_dir().join(format!("macsplorer-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (dir, sub, other) = (base.join("dir"), base.join("dir/sub"), base.join("other"));
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir_all(&other).unwrap();
        let mut w = FolderWatcher::new(|| {});
        w.watch_only(&[dir.clone()].into());
        // FSEvents may report what happened just before the stream started.
        std::thread::sleep(Duration::from_millis(300));
        let start = Instant::now();
        let secs = move || start.elapsed().as_secs_f64();
        let _ = w.poll(secs() + MAX_WAIT);
        let wait = |w: &mut FolderWatcher| {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut seen = BTreeSet::new();
            while Instant::now() < deadline {
                seen.extend(w.poll(secs()));
                if !seen.is_empty() && w.next_due().is_none() {
                    // Let stragglers arrive.
                    std::thread::sleep(Duration::from_millis(300));
                    seen.extend(w.poll(secs() + MAX_WAIT));
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            seen
        };
        fs::write(other.join("x"), "x").unwrap();
        fs::write(sub.join("x"), "x").unwrap();
        fs::write(dir.join("new.txt"), "hi").unwrap();
        assert_eq!(wait(&mut w), [dir.clone()].into());
        fs::remove_dir_all(&base).unwrap();
    }
}
