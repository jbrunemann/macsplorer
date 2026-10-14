//! Navigation pane folder tree, modeled on Explorer's `CNscTree`.
//!
//! Like `CNscTree`, the tree:
//! - shows folders only, sorted by name (`_SortByName`);
//! - loads children on demand, off the UI thread (`_StartBackgroundEnum`);
//! - can expand itself to the folder open in the view (`_ExpandToItem`);
//! - reloads expanded folders on refresh (`_OnSHNotifyUpdateDir`).
//!
//! The model never touches the file system. [`load_children`] does, and the
//! app runs it on a worker thread and hands the result to
//! [`FolderTree::loaded`].

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::listing::{is_hidden_name, list_dir};
use crate::natural::natural_cmp;
use crate::places::Place;

/// Children beyond this many are not probed for subfolders; they are
/// assumed expandable so huge folders still load quickly.
const PROBE_LIMIT: usize = 300;
/// Entries scanned per folder when probing for a subfolder.
const PROBE_SCAN: usize = 2000;
/// Guards against symlink loops.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Favorites,
    Locations,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Favorites => "Favorites",
            Section::Locations => "Locations",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Root {
    pub label: String,
    pub path: PathBuf,
    pub section: Section,
}

impl Root {
    pub fn from_places(places: &[Place], section: Section) -> impl Iterator<Item = Root> + '_ {
        places.iter().map(move |p| Root { label: p.label.clone(), path: p.path.clone(), section })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub has_subfolders: bool,
}

/// Identifies one row. The same folder can appear under several roots
/// (Desktop under Favorites and under Home), and each copy expands
/// independently, as tree items do in Explorer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeKey {
    pub root: usize,
    pub path: PathBuf,
}

#[derive(Debug, Default)]
struct ChildState {
    nodes: Vec<Node>,
    loaded: bool,
    loading: bool,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: NodeKey,
    pub label: String,
    pub depth: usize,
    pub section: Section,
    pub expandable: bool,
    pub expanded: bool,
    pub selected: bool,
    /// A "Loading…" placeholder under an expanded folder.
    pub placeholder: bool,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct FolderTree {
    roots: Vec<Root>,
    expanded: HashSet<NodeKey>,
    children: HashMap<PathBuf, ChildState>,
    selected: Option<NodeKey>,
    scroll_to_selected: bool,
}

impl FolderTree {
    pub fn new(roots: Vec<Root>) -> Self {
        Self { roots, ..Default::default() }
    }

    /// Replaces the roots, e.g. after a volume is mounted. Expansion and
    /// selection survive for roots whose path is still present.
    pub fn set_roots(&mut self, roots: Vec<Root>) {
        let remap: HashMap<usize, usize> = self
            .roots
            .iter()
            .enumerate()
            .filter_map(|(old, r)| {
                roots.iter().position(|n| n.path == r.path && n.section == r.section).map(|new| (old, new))
            })
            .collect();
        let move_key = |k: &NodeKey| remap.get(&k.root).map(|&root| NodeKey { root, path: k.path.clone() });
        self.expanded = self.expanded.iter().filter_map(move_key).collect();
        self.selected = self.selected.as_ref().and_then(move_key);
        self.roots = roots;
    }

    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    pub fn selected(&self) -> Option<&NodeKey> {
        self.selected.as_ref()
    }

    pub fn select(&mut self, key: NodeKey) {
        self.selected = Some(key);
    }

    pub fn is_expanded(&self, key: &NodeKey) -> bool {
        self.expanded.contains(key)
    }

    /// Expands a node. Returns the folder to load if its children aren't
    /// known yet.
    pub fn expand(&mut self, key: NodeKey) -> Option<PathBuf> {
        let path = key.path.clone();
        self.expanded.insert(key);
        self.request(&path)
    }

    pub fn collapse(&mut self, key: &NodeKey) {
        self.expanded.remove(key);
        // Collapsing an ancestor of the selection moves the selection up to
        // it, as a tree view does.
        if let Some(sel) = &self.selected
            && sel.root == key.root
            && sel.path != key.path
            && sel.path.starts_with(&key.path)
        {
            self.selected = Some(key.clone());
        }
    }

    pub fn toggle(&mut self, key: NodeKey) -> Option<PathBuf> {
        if self.is_expanded(&key) {
            self.collapse(&key);
            None
        } else {
            self.expand(key)
        }
    }

    /// Marks a folder as loading unless it is loaded or already loading.
    fn request(&mut self, path: &Path) -> Option<PathBuf> {
        let state = self.children.entry(path.to_path_buf()).or_default();
        if state.loaded || state.loading {
            return None;
        }
        state.loading = true;
        Some(path.to_path_buf())
    }

    /// Stores the result of [`load_children`].
    pub fn loaded(&mut self, path: PathBuf, result: Result<Vec<Node>, String>) {
        let state = self.children.entry(path).or_default();
        state.loading = false;
        state.loaded = true;
        match result {
            Ok(nodes) => {
                state.nodes = nodes;
                state.error = None;
            }
            Err(e) => {
                state.nodes.clear();
                state.error = Some(e);
            }
        }
    }

    /// Expands the ancestors of `target` under the best-matching root and
    /// selects it. Returns folders that need loading.
    ///
    /// The best root is the one with the longest path that contains
    /// `target`, so `~/Documents/x` opens under "Documents", not "Home".
    pub fn reveal(&mut self, target: &Path) -> Vec<PathBuf> {
        let Some(root) = self.best_root(target) else {
            self.selected = None;
            return Vec::new();
        };
        let root_path = self.roots[root].path.clone();
        let mut loads = Vec::new();
        let mut cur = root_path.clone();
        let rest: Vec<_> = target.strip_prefix(&root_path).map(|r| r.components().collect()).unwrap_or_default();
        for comp in rest {
            loads.extend(self.expand(NodeKey { root, path: cur.clone() }));
            cur.push(comp);
        }
        self.selected = Some(NodeKey { root, path: target.to_path_buf() });
        self.scroll_to_selected = true;
        loads
    }

    /// Selects the row for `target` if one is already visible, without
    /// expanding anything. Used when "expand to open folder" is off.
    pub fn select_visible(&mut self, target: &Path) {
        let rows = self.rows();
        let keep = self.selected.as_ref().is_some_and(|s| s.path == target && rows.iter().any(|r| &r.key == s));
        if !keep {
            self.selected = rows.iter().find(|r| !r.placeholder && r.key.path == target).map(|r| r.key.clone());
        }
    }

    fn best_root(&self, target: &Path) -> Option<usize> {
        self.roots
            .iter()
            .enumerate()
            .filter(|(_, r)| target.starts_with(&r.path))
            .max_by_key(|(i, r)| (r.path.components().count(), std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
    }

    /// Returns (once) whether the view should scroll the selection into view.
    pub fn take_scroll_request(&mut self) -> bool {
        std::mem::take(&mut self.scroll_to_selected)
    }

    /// Reloads every loaded folder that is currently expanded. Old children
    /// stay visible until the new ones arrive, so the tree doesn't flicker.
    pub fn refresh(&mut self) -> Vec<PathBuf> {
        let expanded: HashSet<PathBuf> = self.expanded.iter().map(|k| k.path.clone()).collect();
        self.children.retain(|p, _| expanded.contains(p));
        let mut loads = Vec::new();
        for (path, state) in &mut self.children {
            if state.loaded && !state.loading {
                state.loading = true;
                loads.push(path.clone());
            }
        }
        loads
    }

    /// Reloads one folder if the tree has it loaded (e.g. after a rename).
    pub fn refresh_path(&mut self, path: &Path) -> Option<PathBuf> {
        let state = self.children.get_mut(path)?;
        if !state.loaded || state.loading {
            return None;
        }
        state.loading = true;
        Some(path.to_path_buf())
    }

    /// Drops all cached children (e.g. when hidden items are toggled) and
    /// returns the expanded folders to load again.
    pub fn reset_children(&mut self) -> Vec<PathBuf> {
        self.children.clear();
        let paths: HashSet<PathBuf> = self.expanded.iter().map(|k| k.path.clone()).collect();
        paths.into_iter().filter_map(|p| self.request(&p)).collect()
    }

    /// Flattens the visible part of the tree, top to bottom.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        for (i, root) in self.roots.iter().enumerate() {
            let key = NodeKey { root: i, path: root.path.clone() };
            // Roots are assumed expandable until their children are known.
            let expandable = self.children.get(&root.path).is_none_or(|c| !c.loaded || !c.nodes.is_empty());
            self.push_rows(&mut out, key, root.label.clone(), 0, root.section, expandable);
        }
        out
    }

    fn push_rows(&self, out: &mut Vec<Row>, key: NodeKey, label: String, depth: usize, section: Section, expandable: bool) {
        let state = self.children.get(&key.path);
        let expanded = expandable && self.expanded.contains(&key);
        out.push(Row {
            selected: self.selected.as_ref() == Some(&key),
            error: state.and_then(|s| s.error.clone()),
            expanded,
            expandable,
            label,
            depth,
            section,
            placeholder: false,
            key: key.clone(),
        });
        if !expanded || depth >= MAX_DEPTH {
            return;
        }
        match state {
            Some(s) if s.loaded => {
                for n in &s.nodes {
                    let child = NodeKey { root: key.root, path: n.path.clone() };
                    self.push_rows(out, child, n.name.clone(), depth + 1, section, n.has_subfolders);
                }
            }
            _ => out.push(Row {
                key: NodeKey { root: key.root, path: key.path.join("\u{0}loading") },
                label: "Loading…".into(),
                depth: depth + 1,
                section,
                expandable: false,
                expanded: false,
                selected: false,
                placeholder: true,
                error: None,
            }),
        }
    }
}

/// Reads the subfolders of `dir` for the tree. Packages (`.app` and the
/// like) are items, not folders, so they are left out.
pub fn load_children(dir: &Path, show_hidden: bool) -> Result<Vec<Node>, String> {
    let entries = list_dir(dir, show_hidden).map_err(|e| e.to_string())?;
    let mut folders: Vec<_> = entries.into_iter().filter(|e| e.is_folder()).collect();
    folders.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    let probe = folders.len() <= PROBE_LIMIT;
    Ok(folders
        .into_iter()
        .map(|e| Node { has_subfolders: !probe || has_subfolders(&e.path, show_hidden), name: e.name, path: e.path })
        .collect())
}

/// True if `dir` contains at least one folder the tree would show.
/// Unreadable folders report `true` so the user can try to expand them and
/// see the error.
fn has_subfolders(dir: &Path, show_hidden: bool) -> bool {
    let Ok(rd) = fs::read_dir(dir) else { return true };
    rd.flatten().take(PROBE_SCAN).any(|e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        (show_hidden || !is_hidden_name(&name))
            && fs::metadata(e.path()).is_ok_and(|m| m.is_dir())
            && !crate::listing::is_package_name(&name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(label: &str, path: &str, section: Section) -> Root {
        Root { label: label.into(), path: path.into(), section }
    }

    fn node(path: &str, sub: bool) -> Node {
        let p = PathBuf::from(path);
        Node { name: p.file_name().unwrap().to_string_lossy().into(), path: p, has_subfolders: sub }
    }

    fn key(root: usize, path: &str) -> NodeKey {
        NodeKey { root, path: path.into() }
    }

    fn labels(t: &FolderTree) -> Vec<String> {
        t.rows().iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.label)).collect()
    }

    fn tree() -> FolderTree {
        FolderTree::new(vec![
            root("Home", "/u", Section::Favorites),
            root("Documents", "/u/Docs", Section::Favorites),
            root("Macintosh HD", "/", Section::Locations),
        ])
    }

    #[test]
    fn expand_shows_placeholder_then_children() {
        let mut t = tree();
        assert_eq!(t.expand(key(0, "/u")), Some(PathBuf::from("/u")));
        assert_eq!(labels(&t), ["Home", "  Loading…", "Documents", "Macintosh HD"]);
        // A second expand while loading doesn't request again.
        assert_eq!(t.expand(key(0, "/u")), None);
        t.loaded("/u".into(), Ok(vec![node("/u/Docs", true), node("/u/Music", false)]));
        assert_eq!(labels(&t), ["Home", "  Docs", "  Music", "Documents", "Macintosh HD"]);
        let music = t.rows().into_iter().find(|r| r.label == "Music").unwrap();
        assert!(!music.expandable);
    }

    #[test]
    fn same_folder_expands_independently_per_root() {
        let mut t = tree();
        t.expand(key(0, "/u"));
        t.loaded("/u".into(), Ok(vec![node("/u/Docs", true)]));
        t.expand(key(1, "/u/Docs"));
        t.loaded("/u/Docs".into(), Ok(vec![node("/u/Docs/a", false)]));
        // Expanded under the "Documents" root, still collapsed under Home.
        assert_eq!(labels(&t), ["Home", "  Docs", "Documents", "  a", "Macintosh HD"]);
    }

    #[test]
    fn reveal_uses_longest_root_and_selects() {
        let mut t = tree();
        let loads = t.reveal(Path::new("/u/Docs/a/b"));
        assert_eq!(loads, [PathBuf::from("/u/Docs"), PathBuf::from("/u/Docs/a")]);
        assert_eq!(t.selected(), Some(&key(1, "/u/Docs/a/b")));
        assert!(t.take_scroll_request());
        assert!(!t.take_scroll_request());
    }

    #[test]
    fn reveal_outside_any_root_clears_selection() {
        let mut t = FolderTree::new(vec![root("Home", "/u", Section::Favorites)]);
        t.select(key(0, "/u"));
        assert!(t.reveal(Path::new("/elsewhere")).is_empty());
        assert_eq!(t.selected(), None);
    }

    #[test]
    fn collapse_moves_selection_to_collapsed_node() {
        let mut t = tree();
        t.reveal(Path::new("/u/Docs/a"));
        t.collapse(&key(1, "/u/Docs"));
        assert_eq!(t.selected(), Some(&key(1, "/u/Docs")));
    }

    #[test]
    fn refresh_reloads_only_expanded_folders() {
        let mut t = tree();
        t.expand(key(0, "/u"));
        t.loaded("/u".into(), Ok(vec![node("/u/Docs", true)]));
        t.expand(key(0, "/u/Docs"));
        t.loaded("/u/Docs".into(), Ok(vec![]));
        t.collapse(&key(0, "/u/Docs"));
        assert_eq!(t.refresh(), [PathBuf::from("/u")]);
        // Children stay visible while reloading.
        assert_eq!(labels(&t)[1], "  Docs");
    }

    #[test]
    fn empty_root_loses_expander() {
        let mut t = tree();
        t.expand(key(2, "/"));
        t.loaded("/".into(), Ok(vec![]));
        let r = &t.rows()[2];
        assert!(!r.expandable && !r.expanded);
    }

    #[test]
    fn errors_are_reported_on_the_row() {
        let mut t = tree();
        t.expand(key(0, "/u"));
        t.loaded("/u".into(), Err("Permission denied".into()));
        assert_eq!(t.rows()[0].error.as_deref(), Some("Permission denied"));
    }

    #[test]
    fn set_roots_keeps_state_by_path() {
        let mut t = tree();
        t.expand(key(2, "/"));
        t.select(key(2, "/"));
        t.set_roots(vec![root("Macintosh HD", "/", Section::Locations)]);
        assert!(t.is_expanded(&key(0, "/")));
        assert_eq!(t.selected(), Some(&key(0, "/")));
    }

    #[test]
    fn select_visible_does_not_expand() {
        let mut t = tree();
        t.select_visible(Path::new("/u/Docs/a"));
        assert_eq!(t.selected(), None);
        t.select_visible(Path::new("/u/Docs"));
        assert_eq!(t.selected(), Some(&key(1, "/u/Docs")));
    }

    #[test]
    fn loads_real_folders_sorted_without_packages() {
        let dir = std::env::temp_dir().join(format!("macsplorer-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for d in ["b10", "b2", "b2/inner", "Thing.app", ".hidden"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        fs::write(dir.join("file.txt"), "x").unwrap();
        let nodes = load_children(&dir, false).unwrap();
        let names: Vec<_> = nodes.iter().map(|n| (n.name.as_str(), n.has_subfolders)).collect();
        assert_eq!(names, [("b2", true), ("b10", false)]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
