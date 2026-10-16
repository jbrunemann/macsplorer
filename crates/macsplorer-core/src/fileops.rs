//! Copy and move for the clipboard's Paste.
//!
//! Windows does this in `CFileOperation` / `CCopyWorkItem` (Windows.Storage,
//! build 10240). [`transfer`] follows what `CCopyWorkItem::_CheckSameName`
//! and `CCopyWorkItem::ConfirmOverwrite` decide when the destination
//! already has an item of that name (`docs/file-browser-findings.md` §2.11):
//!
//! - The item itself (pasted into the folder it is in): a copy gets
//!   "Name - Copy.ext", then "Name - Copy (2).ext" and so on; a move does
//!   nothing.
//! - A folder onto a folder merges them. When nothing inside would collide
//!   (`GetMergeFolderStats` finds no conflicts) that happens without asking;
//!   otherwise the user confirms the merge (`COPYENGINE_E_ALREADY_EXISTS_FOLDER`).
//! - Anything else asks whether to replace (`COPYENGINE_E_ALREADY_EXISTS_NORMAL`).
//! - An answer "for all" is kept for the rest of the operation; the prompt is
//!   only shown while no such answer is stored.
//!
//! Not from Windows: a replaced item goes to the Trash rather than being
//! destroyed, so Undo can bring it back.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

use crate::rename::rename_no_replace;
use crate::undo::{Trash, Trashed, UndoOp};

/// What the destination already has, as the question puts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    /// A folder is pasted onto a folder: merge them?
    Folder,
    /// Anything else: replace the existing item?
    Item,
}

/// A name the destination already has, for the user to decide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub kind: ConflictKind,
    /// The item being copied or moved.
    pub source: PathBuf,
    /// The item of that name in the destination.
    pub existing: PathBuf,
    pub moving: bool,
}

/// The answer to a [`Conflict`]: the Win2000 confirmation dialog's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Yes,
    /// Yes for this and every later conflict of the operation.
    YesToAll,
    /// Skip this item.
    No,
    /// Stop the operation; what was done stays done.
    Cancel,
}

/// What a [`transfer`] did.
#[derive(Debug, Default)]
pub struct Transferred {
    /// The pasted items as they now are in the destination, to select.
    pub placed: Vec<PathBuf>,
    /// What Undo reverses: the copies or moves, and the replaced items.
    pub undo: Option<UndoOp>,
    /// The item that failed and why; the operation stopped there.
    pub error: Option<(PathBuf, io::Error)>,
    pub cancelled: bool,
}

/// Copies (or with `moving`, moves) `sources` into the folder `dest`,
/// asking `ask` about every name the destination already has (see the
/// module docs). Stops at the first error or a Cancel.
pub fn transfer(sources: &[PathBuf], dest: &Path, moving: bool, trash: &impl Trash, ask: impl FnMut(&Conflict) -> Choice) -> Transferred {
    let mut run = Run { moving, trash, ask, all: false, copies: Vec::new(), moved: Vec::new(), replaced: Vec::new() };
    let mut out = Transferred::default();
    for src in sources {
        match run.place(src, dest) {
            Ok(Placed::Done(path)) => out.placed.push(path),
            Ok(Placed::Skipped) => {}
            Ok(Placed::Cancelled) => {
                out.cancelled = true;
                break;
            }
            Err(e) => {
                out.error = Some(e);
                break;
            }
        }
    }
    let done = if moving { UndoOp::Move(run.moved) } else { UndoOp::Copy(run.copies) };
    out.undo = Some(if run.replaced.is_empty() { done } else { UndoOp::Group(vec![UndoOp::Delete(run.replaced), done]) });
    out
}

enum Placed {
    Done(PathBuf),
    Skipped,
    Cancelled,
}

struct Run<'a, T, A> {
    moving: bool,
    trash: &'a T,
    ask: A,
    /// A "Yes to All" was given.
    all: bool,
    copies: Vec<PathBuf>,
    moved: Vec<(PathBuf, PathBuf)>,
    replaced: Vec<Trashed>,
}

impl<T: Trash, A: FnMut(&Conflict) -> Choice> Run<'_, T, A> {
    /// Puts `src` into the folder `dir`, merging or replacing as answered.
    fn place(&mut self, src: &Path, dir: &Path) -> Result<Placed, (PathBuf, io::Error)> {
        let fail = |e: io::Error| (src.to_path_buf(), e);
        let name = src.file_name().ok_or_else(|| fail(io::Error::other("the item has no name")))?;
        let src_meta = src.symlink_metadata().map_err(fail)?;
        if src_meta.is_dir() && dir.starts_with(src) {
            let verb = if self.moving { "moved" } else { "copied" };
            return Err(fail(io::Error::other(format!("a folder can't be {verb} into itself"))));
        }
        let target = dir.join(name);
        // `_CheckSameName`: the item itself.
        if target == src {
            if self.moving {
                return Ok(Placed::Done(target));
            }
            let copy = copy_name(dir, name, src_meta.is_dir());
            copy_tree(src, &copy).map_err(fail)?;
            self.copies.push(copy.clone());
            return Ok(Placed::Done(copy));
        }
        let Ok(existing) = target.symlink_metadata() else {
            self.put(src, &target).map_err(fail)?;
            return Ok(Placed::Done(target));
        };
        let merge = src_meta.is_dir() && existing.is_dir();
        if merge && !has_conflicts(src, &target) {
            return self.merge(src, &target);
        }
        let kind = if merge { ConflictKind::Folder } else { ConflictKind::Item };
        let choice = if self.all {
            Choice::Yes
        } else {
            (self.ask)(&Conflict { kind, source: src.to_path_buf(), existing: target.clone(), moving: self.moving })
        };
        match choice {
            Choice::No => return Ok(Placed::Skipped),
            Choice::Cancel => return Ok(Placed::Cancelled),
            Choice::YesToAll => self.all = true,
            Choice::Yes => {}
        }
        if merge {
            return self.merge(src, &target);
        }
        if src.starts_with(&target) {
            return Err(fail(io::Error::other("an item can't replace the folder it is in")));
        }
        let gone = self.trash.trash(&target).map_err(|e| (target.clone(), e))?;
        self.replaced.push(gone);
        self.put(src, &target).map_err(fail)?;
        Ok(Placed::Done(target))
    }

    /// Puts each item of the folder `src` into the folder `target`. A move
    /// removes `src` once nothing is left in it.
    fn merge(&mut self, src: &Path, target: &Path) -> Result<Placed, (PathBuf, io::Error)> {
        let mut children: Vec<PathBuf> = std::fs::read_dir(src).map_err(|e| (src.to_path_buf(), e))?.filter_map(|e| e.ok().map(|e| e.path())).collect();
        children.sort();
        for child in children {
            if let Placed::Cancelled = self.place(&child, target)? {
                return Ok(Placed::Cancelled);
            }
        }
        if self.moving {
            // Fails, and stays, when skipped items are still in it.
            let _ = std::fs::remove_dir(src);
        }
        Ok(Placed::Done(target.to_path_buf()))
    }

    /// Copies or moves `src` to the free path `target`.
    fn put(&mut self, src: &Path, target: &Path) -> io::Result<()> {
        if self.moving {
            move_to(src, target)?;
            self.moved.push((src.to_path_buf(), target.to_path_buf()));
        } else {
            copy_tree(src, target)?;
            self.copies.push(target.to_path_buf());
        }
        Ok(())
    }
}

/// Whether merging the folder `src` into the folder `target` would meet a
/// name `target` already has, other than folders that merge in turn
/// (`CFileOperation::GetMergeFolderStats`).
fn has_conflicts(src: &Path, target: &Path) -> bool {
    let Ok(items) = std::fs::read_dir(src) else { return true };
    items.flatten().any(|item| {
        let there = target.join(item.file_name());
        match there.symlink_metadata() {
            Err(_) => false,
            Ok(meta) => !(meta.is_dir() && item.file_type().is_ok_and(|t| t.is_dir())) || has_conflicts(&item.path(), &there),
        }
    })
}

/// The first free "Name - Copy.ext", "Name - Copy (2).ext", ... in `dir`.
pub fn copy_name(dir: &Path, name: &OsStr, is_dir: bool) -> PathBuf {
    let path = Path::new(name);
    let (stem, ext) = match path.extension() {
        Some(ext) if !is_dir => (path.file_stem().unwrap_or(name), Some(ext)),
        _ => (name, None),
    };
    (1..)
        .map(|n| {
            let mut candidate = OsString::from(stem);
            candidate.push(if n == 1 { " - Copy".to_string() } else { format!(" - Copy ({n})") });
            if let Some(ext) = ext {
                candidate.push(".");
                candidate.push(ext);
            }
            dir.join(candidate)
        })
        .find(|p| p.symlink_metadata().is_err())
        .expect("an unbounded range always yields a free name")
}

/// Moves `src` to the path `dest`, across volumes too, never replacing.
pub fn move_to(src: &Path, dest: &Path) -> io::Result<()> {
    match rename_no_replace(src, dest) {
        Ok(()) => Ok(()),
        // Another volume: copy, then remove the original.
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            if dest.symlink_metadata().is_ok() {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            copy_tree(src, dest)?;
            if src.symlink_metadata()?.is_dir() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) }
        }
        Err(e) => Err(e),
    }
}

#[cfg(target_os = "macos")]
fn copy_tree(src: &Path, dest: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other);
    let (csrc, cdest) = (c(src)?, c(dest)?);
    let flags = libc::COPYFILE_METADATA | libc::COPYFILE_DATA | libc::COPYFILE_RECURSIVE | libc::COPYFILE_CLONE | libc::COPYFILE_NOFOLLOW;
    // SAFETY: both are valid NUL-terminated paths; a null state is allowed.
    if unsafe { libc::copyfile(csrc.as_ptr(), cdest.as_ptr(), std::ptr::null_mut(), flags) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn copy_tree(src: &Path, dest: &Path) -> io::Result<()> {
    let meta = src.symlink_metadata()?;
    if meta.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dest)
    } else if meta.is_dir() {
        std::fs::create_dir(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dest.join(entry.file_name()))?;
        }
        std::fs::set_permissions(dest, meta.permissions())
    } else {
        std::fs::copy(src, dest).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("macsplorer-fileops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copy_names_count_up() {
        let dir = scratch("names");
        let name = |n: &str, d| copy_name(&dir, OsStr::new(n), d).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name("a.txt", false), "a - Copy.txt");
        assert_eq!(name("a.b", true), "a.b - Copy");
        assert_eq!(name(".rc", false), ".rc - Copy");
        assert_eq!(name("noext", false), "noext - Copy");
        fs::write(dir.join("a - Copy.txt"), "").unwrap();
        assert_eq!(name("a.txt", false), "a - Copy (2).txt");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A Trash that moves items into a folder of the scratch directory.
    struct FolderTrash(PathBuf);

    impl Trash for FolderTrash {
        fn trash(&self, path: &Path) -> io::Result<Trashed> {
            let n = fs::read_dir(&self.0)?.count();
            let to = self.0.join(n.to_string());
            fs::rename(path, &to)?;
            Ok(Trashed { original: path.to_path_buf(), in_trash: Some(to) })
        }
        fn restore(&self, item: &Trashed) -> io::Result<()> {
            fs::rename(item.in_trash.as_ref().unwrap(), &item.original)
        }
    }

    fn trash_in(dir: &Path) -> FolderTrash {
        let bin = dir.join(".bin");
        fs::create_dir_all(&bin).unwrap();
        FolderTrash(bin)
    }

    fn never(_: &Conflict) -> Choice {
        panic!("nothing should be asked")
    }

    #[test]
    fn copy_next_to_source_and_into_other_folder() {
        let dir = scratch("copy");
        let other = dir.join("other");
        fs::create_dir(&other).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "hi").unwrap();
        let trash = trash_in(&dir);
        let twin = transfer(std::slice::from_ref(&file), &dir, false, &trash, never);
        assert_eq!(twin.placed, [dir.join("a - Copy.txt")]);
        assert_eq!(twin.undo, Some(UndoOp::Copy(vec![dir.join("a - Copy.txt")])));
        let there = transfer(std::slice::from_ref(&file), &other, false, &trash, never);
        assert_eq!(there.placed, [other.join("a.txt")]);
        assert_eq!(fs::read_to_string(other.join("a.txt")).unwrap(), "hi");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_folder_recursively_but_not_into_itself() {
        let dir = scratch("tree");
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/f"), "x").unwrap();
        std::os::unix::fs::symlink("sub/f", src.join("link")).unwrap();
        let trash = trash_in(&dir);
        let into_itself = transfer(std::slice::from_ref(&src), &src.join("sub"), false, &trash, never);
        assert!(into_itself.error.is_some());
        let copy = transfer(std::slice::from_ref(&src), &dir, false, &trash, never).placed;
        assert_eq!(copy, [dir.join("src - Copy")]);
        assert_eq!(fs::read_to_string(dir.join("src - Copy/sub/f")).unwrap(), "x");
        assert!(dir.join("src - Copy/link").symlink_metadata().unwrap().is_symlink());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn moving_into_its_own_folder_does_nothing() {
        let dir = scratch("samemove");
        let file = dir.join("a.txt");
        fs::write(&file, "x").unwrap();
        let done = transfer(std::slice::from_ref(&file), &dir, true, &trash_in(&dir), never);
        assert_eq!(done.placed, std::slice::from_ref(&file));
        assert_eq!(done.undo, Some(UndoOp::Move(vec![])));
        assert!(file.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn move_folder_into_itself_fails() {
        let dir = scratch("selfmove");
        let sub = dir.join("a/b");
        fs::create_dir_all(&sub).unwrap();
        assert!(transfer(&[dir.join("a")], &sub, true, &trash_in(&dir), never).error.is_some());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// `dir/src/a.txt` ("new") and `dir/dst/a.txt` ("old").
    fn clash(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = scratch(name);
        let (src, dst) = (dir.join("src"), dir.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a.txt"), "new").unwrap();
        fs::write(dst.join("a.txt"), "old").unwrap();
        (dir, src, dst)
    }

    #[test]
    fn a_file_conflict_asks_and_replace_trashes_the_old_one() {
        let (dir, src, dst) = clash("replace");
        let trash = trash_in(&dir);
        let mut asked = Vec::new();
        let done = transfer(&[src.join("a.txt")], &dst, false, &trash, |c| {
            asked.push(c.clone());
            Choice::Yes
        });
        assert_eq!(asked, [Conflict { kind: ConflictKind::Item, source: src.join("a.txt"), existing: dst.join("a.txt"), moving: false }]);
        assert_eq!(done.placed, [dst.join("a.txt")]);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "new");
        // Undo removes the copy and brings the old file back.
        done.undo.unwrap().undo(&trash).unwrap();
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_skips_and_cancel_stops() {
        let (dir, src, dst) = clash("skip");
        fs::write(src.join("b.txt"), "b").unwrap();
        let trash = trash_in(&dir);
        let sources = [src.join("a.txt"), src.join("b.txt")];
        let skipped = transfer(&sources, &dst, true, &trash, |_| Choice::No);
        assert_eq!(skipped.placed, [dst.join("b.txt")]);
        assert!(!skipped.cancelled);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
        assert!(src.join("a.txt").exists());

        fs::write(src.join("c.txt"), "c").unwrap();
        let sources = [src.join("a.txt"), src.join("c.txt")];
        let cancelled = transfer(&sources, &dst, true, &trash, |_| Choice::Cancel);
        assert!(cancelled.cancelled);
        assert!(cancelled.placed.is_empty());
        assert!(src.join("c.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn yes_to_all_stops_asking() {
        let (dir, src, dst) = clash("all");
        fs::write(src.join("b.txt"), "new").unwrap();
        fs::write(dst.join("b.txt"), "old").unwrap();
        let mut asked = 0;
        let done = transfer(&[src.join("a.txt"), src.join("b.txt")], &dst, false, &trash_in(&dir), |_| {
            asked += 1;
            Choice::YesToAll
        });
        assert_eq!(asked, 1);
        assert_eq!(done.placed.len(), 2);
        assert_eq!(fs::read_to_string(dst.join("b.txt")).unwrap(), "new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folders_without_inner_conflicts_merge_without_asking() {
        let dir = scratch("merge");
        fs::create_dir_all(dir.join("src/f/sub")).unwrap();
        fs::create_dir_all(dir.join("dst/f/sub")).unwrap();
        fs::write(dir.join("src/f/sub/new.txt"), "n").unwrap();
        fs::write(dir.join("dst/f/sub/old.txt"), "o").unwrap();
        let trash = trash_in(&dir);
        let done = transfer(&[dir.join("src/f")], &dir.join("dst"), true, &trash, never);
        assert_eq!(done.placed, [dir.join("dst/f")]);
        assert!(dir.join("dst/f/sub/new.txt").exists() && dir.join("dst/f/sub/old.txt").exists());
        // The emptied source folders are gone, and Undo puts them back.
        assert!(!dir.join("src/f").exists());
        done.undo.unwrap().undo(&trash).unwrap();
        assert!(dir.join("src/f/sub/new.txt").exists());
        assert!(!dir.join("dst/f/sub/new.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folders_with_inner_conflicts_ask_to_merge_then_about_each_file() {
        let dir = scratch("mergeask");
        fs::create_dir_all(dir.join("src/f")).unwrap();
        fs::create_dir_all(dir.join("dst/f")).unwrap();
        fs::write(dir.join("src/f/a.txt"), "new").unwrap();
        fs::write(dir.join("src/f/b.txt"), "b").unwrap();
        fs::write(dir.join("dst/f/a.txt"), "old").unwrap();
        let mut asked = Vec::new();
        let done = transfer(&[dir.join("src/f")], &dir.join("dst"), false, &trash_in(&dir), |c| {
            asked.push(c.kind);
            if c.kind == ConflictKind::Folder { Choice::Yes } else { Choice::No }
        });
        assert_eq!(asked, [ConflictKind::Folder, ConflictKind::Item]);
        assert_eq!(done.placed, [dir.join("dst/f")]);
        assert_eq!(fs::read_to_string(dir.join("dst/f/a.txt")).unwrap(), "old");
        assert!(dir.join("dst/f/b.txt").exists());
        assert_eq!(done.undo, Some(UndoOp::Copy(vec![dir.join("dst/f/b.txt")])));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_and_a_folder_of_one_name_ask_to_replace() {
        let dir = scratch("mixed");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("dst/x")).unwrap();
        fs::write(dir.join("src/x"), "file").unwrap();
        let mut kinds = Vec::new();
        transfer(&[dir.join("src/x")], &dir.join("dst"), false, &trash_in(&dir), |c| {
            kinds.push(c.kind);
            Choice::Yes
        });
        assert_eq!(kinds, [ConflictKind::Item]);
        assert!(dir.join("dst/x").is_file());
        fs::remove_dir_all(&dir).unwrap();
    }
}
