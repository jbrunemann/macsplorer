//! Undo for renames, pastes and deletes (the Undo toolbar button, Edit >
//! Undo, Cmd+Z).
//!
//! Windows 10 records each finished file operation in the thread's undo
//! manager (`CFileOperation::_AddUndoRootsToUndoManager` in Windows.Storage,
//! `SHGetThreadUndoManager` / `CUndoManager` in shell32), a stack of units
//! (`CCommonParentUndoUnit::_PushStack` / `_PopStack`) undone newest first.
//! The menu text is "Undo <operation>" (`GetFileUndoText`).
//!
//! Not traced, so these are guesses: Windows bounds the stack by memory use
//! (`CCommonParentUndoUnit::_GetMemoryUsage`), here it keeps the last
//! [`UNDO_LIMIT`] operations. Undoing a copy moves the copies to the Trash
//! rather than destroying them. An undo that fails part way stops there and
//! is not retried.

use std::io;
use std::path::{Path, PathBuf};

use crate::fileops::move_to;
use crate::rename::rename_no_replace;

/// How many operations can be undone (unverified, see the module docs).
pub const UNDO_LIMIT: usize = 10;

/// An item the Trash took, and where it went when the platform says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trashed {
    pub original: PathBuf,
    pub in_trash: Option<PathBuf>,
}

/// Moving items to and from the Trash, which the platform provides.
pub trait Trash {
    fn trash(&self, path: &Path) -> io::Result<Trashed>;
    fn restore(&self, item: &Trashed) -> io::Result<()>;
}

/// One finished operation, with what's needed to reverse it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoOp {
    Rename { from: PathBuf, to: PathBuf },
    /// (original path, path it was moved to)
    Move(Vec<(PathBuf, PathBuf)>),
    /// The copies that were made.
    Copy(Vec<PathBuf>),
    Delete(Vec<Trashed>),
    /// Several steps of one operation (a paste that replaced items), undone
    /// last to first.
    Group(Vec<UndoOp>),
}

impl UndoOp {
    /// The operation's name for "Undo <name>".
    pub fn label(&self) -> &'static str {
        match self {
            UndoOp::Rename { .. } => "Rename",
            UndoOp::Move(_) => "Move",
            UndoOp::Copy(_) => "Copy",
            UndoOp::Delete(_) => "Delete",
            UndoOp::Group(ops) => ops.last().map_or("Delete", UndoOp::label),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            UndoOp::Rename { .. } => false,
            UndoOp::Move(items) => items.is_empty(),
            UndoOp::Copy(items) => items.is_empty(),
            UndoOp::Delete(items) => items.is_empty(),
            UndoOp::Group(ops) => ops.iter().all(UndoOp::is_empty),
        }
    }

    /// The folders whose contents an undo changes.
    pub fn dirs(&self) -> Vec<PathBuf> {
        if let UndoOp::Group(ops) = self {
            let mut dirs: Vec<PathBuf> = ops.iter().flat_map(UndoOp::dirs).collect();
            dirs.sort();
            dirs.dedup();
            return dirs;
        }
        let paths: Vec<&Path> = match self {
            UndoOp::Rename { from, .. } => vec![from],
            UndoOp::Move(items) => items.iter().flat_map(|(a, b)| [a.as_path(), b.as_path()]).collect(),
            UndoOp::Copy(items) => items.iter().map(PathBuf::as_path).collect(),
            UndoOp::Delete(items) => items.iter().map(|t| t.original.as_path()).collect(),
            UndoOp::Group(_) => unreachable!("handled above"),
        };
        let mut dirs: Vec<PathBuf> = paths.into_iter().filter_map(Path::parent).map(Path::to_path_buf).collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }

    /// Reverses the operation and returns the items that are back, to be
    /// selected. Stops at the first item that fails, returning that item's
    /// path with the error.
    pub fn undo(&self, trash: &impl Trash) -> Result<Vec<PathBuf>, (PathBuf, io::Error)> {
        let mut back = Vec::new();
        match self {
            UndoOp::Rename { from, to } => {
                rename_no_replace(to, from).map_err(|e| (to.clone(), e))?;
                back.push(from.clone());
            }
            UndoOp::Move(items) => {
                for (original, moved) in items.iter().rev() {
                    // A merge removed the source folders it emptied.
                    if let Some(parent) = original.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| (moved.clone(), e))?;
                    }
                    move_to(moved, original).map_err(|e| (moved.clone(), e))?;
                    back.push(original.clone());
                }
            }
            UndoOp::Copy(copies) => {
                for copy in copies {
                    trash.trash(copy).map_err(|e| (copy.clone(), e))?;
                }
            }
            UndoOp::Delete(items) => {
                for item in items.iter().rev() {
                    trash.restore(item).map_err(|e| (item.original.clone(), e))?;
                    back.push(item.original.clone());
                }
            }
            UndoOp::Group(ops) => {
                for op in ops.iter().rev() {
                    back.extend(op.undo(trash)?);
                }
            }
        }
        Ok(back)
    }
}

/// The operations that can be undone, newest last.
#[derive(Default)]
pub struct UndoStack {
    ops: Vec<UndoOp>,
}

impl UndoStack {
    /// Records an operation; one that did nothing is dropped.
    pub fn push(&mut self, op: UndoOp) {
        if op.is_empty() {
            return;
        }
        self.ops.push(op);
        if self.ops.len() > UNDO_LIMIT {
            self.ops.remove(0);
        }
    }

    /// The operation Undo would reverse next.
    pub fn peek(&self) -> Option<&UndoOp> {
        self.ops.last()
    }

    pub fn pop(&mut self) -> Option<UndoOp> {
        self.ops.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("macsplorer-undo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A Trash that's just a folder.
    struct FakeTrash {
        bin: PathBuf,
        taken: RefCell<Vec<PathBuf>>,
    }

    impl Trash for FakeTrash {
        fn trash(&self, path: &Path) -> io::Result<Trashed> {
            let dest = self.bin.join(path.file_name().unwrap());
            fs::rename(path, &dest)?;
            self.taken.borrow_mut().push(path.to_path_buf());
            Ok(Trashed { original: path.to_path_buf(), in_trash: Some(dest) })
        }

        fn restore(&self, item: &Trashed) -> io::Result<()> {
            rename_no_replace(item.in_trash.as_ref().unwrap(), &item.original)
        }
    }

    fn fake_trash(dir: &Path) -> FakeTrash {
        let bin = dir.join(".bin");
        fs::create_dir_all(&bin).unwrap();
        FakeTrash { bin, taken: RefCell::new(Vec::new()) }
    }

    #[test]
    fn stack_drops_empty_ops_and_the_oldest_past_the_limit() {
        let mut stack = UndoStack::default();
        stack.push(UndoOp::Copy(Vec::new()));
        assert!(stack.peek().is_none());
        for i in 0..=UNDO_LIMIT {
            stack.push(UndoOp::Copy(vec![PathBuf::from(format!("/c{i}"))]));
        }
        let mut n = 0;
        while let Some(op) = stack.pop() {
            n += 1;
            if stack.peek().is_none() {
                assert_eq!(op, UndoOp::Copy(vec![PathBuf::from("/c1")]));
            }
        }
        assert_eq!(n, UNDO_LIMIT);
    }

    #[test]
    fn undo_rename_and_move() {
        let dir = scratch("rename-move");
        let trash = fake_trash(&dir);
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("b.txt"), "x").unwrap();
        let op = UndoOp::Rename { from: dir.join("a.txt"), to: dir.join("b.txt") };
        assert_eq!(op.undo(&trash).unwrap(), vec![dir.join("a.txt")]);
        assert!(dir.join("a.txt").exists() && !dir.join("b.txt").exists());

        fs::rename(dir.join("a.txt"), dir.join("sub/a.txt")).unwrap();
        let op = UndoOp::Move(vec![(dir.join("a.txt"), dir.join("sub/a.txt"))]);
        assert_eq!(op.dirs(), vec![dir.clone(), dir.join("sub")]);
        op.undo(&trash).unwrap();
        assert!(dir.join("a.txt").exists() && !dir.join("sub/a.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_never_replaces() {
        let dir = scratch("conflict");
        let trash = fake_trash(&dir);
        fs::write(dir.join("a.txt"), "new").unwrap();
        fs::write(dir.join("b.txt"), "renamed").unwrap();
        let op = UndoOp::Rename { from: dir.join("a.txt"), to: dir.join("b.txt") };
        let (path, e) = op.undo(&trash).unwrap_err();
        assert_eq!((path, e.kind()), (dir.join("b.txt"), io::ErrorKind::AlreadyExists));
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_copy_trashes_the_copies_and_undo_delete_restores() {
        let dir = scratch("copy-delete");
        let trash = fake_trash(&dir);
        fs::write(dir.join("a - Copy.txt"), "x").unwrap();
        let op = UndoOp::Copy(vec![dir.join("a - Copy.txt")]);
        assert_eq!(op.undo(&trash).unwrap(), Vec::<PathBuf>::new());
        assert_eq!(*trash.taken.borrow(), vec![dir.join("a - Copy.txt")]);

        fs::write(dir.join("b.txt"), "x").unwrap();
        let trashed = trash.trash(&dir.join("b.txt")).unwrap();
        let op = UndoOp::Delete(vec![trashed]);
        assert_eq!(op.undo(&trash).unwrap(), vec![dir.join("b.txt")]);
        assert!(dir.join("b.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
