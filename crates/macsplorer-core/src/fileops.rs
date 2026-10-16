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
//!
//! [`transfer_with`] reports its [`Progress`] for the progress dialog and
//! stops when it is cancelled. Cancel keeps what was done, like the
//! conflict question's Cancel, but removes the incomplete copy of the item
//! it was working on and puts back what that item replaced. How the work is
//! weighed (bytes, plus [`ITEM_UNITS`] per item) is not from Windows.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

/// What an item weighs in [`Progress`] units on top of its bytes, so that
/// many small files or a move within a volume, where no bytes are copied,
/// still move the bar.
pub const ITEM_UNITS: u64 = 4096;

/// How far a [`transfer_with`] got, shared with the thread that shows it,
/// and the progress dialog's Cancel button.
#[derive(Debug, Default)]
pub struct Progress {
    /// All the work, once the sources are counted; 0 before.
    total: AtomicU64,
    done: AtomicU64,
    cancelled: AtomicBool,
    /// The item being copied or moved, and the folder it goes to.
    current: Mutex<Option<(PathBuf, PathBuf)>>,
}

/// A look at [`Progress`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgressView {
    pub done: u64,
    /// 0 while the sources are still being counted.
    pub total: u64,
    pub item: Option<(PathBuf, PathBuf)>,
}

impl Progress {
    /// Asks the transfer to stop at the next file, or within the file being copied.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn view(&self) -> ProgressView {
        let total = self.total.load(Ordering::Relaxed);
        // Merges and files that grew can do more than was counted.
        let done = self.done.load(Ordering::Relaxed).min(total);
        ProgressView { done, total, item: self.current.lock().map_or(None, |c| c.clone()) }
    }

    fn add(&self, units: u64) {
        self.done.fetch_add(units, Ordering::Relaxed);
    }

    fn set_item(&self, src: &Path, dir: &Path) {
        if let Ok(mut current) = self.current.lock() {
            *current = Some((src.to_path_buf(), dir.to_path_buf()));
        }
    }

    fn cancelled_error(&self) -> io::Result<()> {
        if self.is_cancelled() { Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")) } else { Ok(()) }
    }
}

/// The units copying `path` takes: its files' bytes and [`ITEM_UNITS`] for
/// each item in it. `None` when cancelled while counting.
fn weight(path: &Path, progress: &Progress) -> Option<u64> {
    if progress.is_cancelled() {
        return None;
    }
    let Ok(meta) = path.symlink_metadata() else { return Some(ITEM_UNITS) };
    let mut units = ITEM_UNITS + if meta.is_file() { meta.len() } else { 0 };
    if meta.is_dir() {
        for item in std::fs::read_dir(path).into_iter().flatten().flatten() {
            units += weight(&item.path(), progress)?;
        }
    }
    Some(units)
}

/// Whether `src` can be renamed into `dir` rather than copied.
#[cfg(unix)]
pub fn same_volume(src: &Path, dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    matches!((src.symlink_metadata(), dir.metadata()), (Ok(a), Ok(b)) if a.dev() == b.dev())
}

#[cfg(not(unix))]
pub fn same_volume(_: &Path, _: &Path) -> bool {
    false
}

/// Copies (or with `moving`, moves) `sources` into the folder `dest`,
/// asking `ask` about every name the destination already has (see the
/// module docs). Stops at the first error or a Cancel.
pub fn transfer(sources: &[PathBuf], dest: &Path, moving: bool, trash: &impl Trash, ask: impl FnMut(&Conflict) -> Choice) -> Transferred {
    transfer_with(sources, dest, moving, trash, ask, &Progress::default())
}

/// [`transfer`], reporting to `progress` and stopping when it is cancelled.
/// The sources are counted first (a move within a volume counts one item
/// per source), so the total is known before anything is changed.
pub fn transfer_with(sources: &[PathBuf], dest: &Path, moving: bool, trash: &impl Trash, ask: impl FnMut(&Conflict) -> Choice, progress: &Progress) -> Transferred {
    let mut out = Transferred::default();
    let mut total = 0;
    for src in sources {
        let units = if moving && same_volume(src, dest) { Some(ITEM_UNITS) } else { weight(src, progress) };
        let Some(units) = units else {
            out.cancelled = true;
            out.undo = Some(if moving { UndoOp::Move(Vec::new()) } else { UndoOp::Copy(Vec::new()) });
            return out;
        };
        total += units;
    }
    progress.total.store(total.max(1), Ordering::Relaxed);
    let mut run = Run { moving, trash, ask, progress, all: false, copies: Vec::new(), moved: Vec::new(), replaced: Vec::new() };
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
    progress: &'a Progress,
    /// A "Yes to All" was given.
    all: bool,
    copies: Vec<PathBuf>,
    moved: Vec<(PathBuf, PathBuf)>,
    replaced: Vec<Trashed>,
}

impl<T: Trash, A: FnMut(&Conflict) -> Choice> Run<'_, T, A> {
    /// Puts `src` into the folder `dir`, merging or replacing as answered.
    fn place(&mut self, src: &Path, dir: &Path) -> Result<Placed, (PathBuf, io::Error)> {
        if self.progress.is_cancelled() {
            return Ok(Placed::Cancelled);
        }
        self.progress.set_item(src, dir);
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
                self.progress.add(ITEM_UNITS);
                return Ok(Placed::Done(target));
            }
            let copy = copy_name(dir, name, src_meta.is_dir());
            if self.copy(src, &copy).map_err(fail)? {
                return Ok(Placed::Cancelled);
            }
            self.copies.push(copy.clone());
            return Ok(Placed::Done(copy));
        }
        let Ok(existing) = target.symlink_metadata() else {
            return self.put(src, &target, None);
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
            Choice::No => {
                self.skip(src, dir);
                return Ok(Placed::Skipped);
            }
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
        self.put(src, &target, Some(gone))
    }

    /// Counts a skipped item as done.
    fn skip(&self, src: &Path, dir: &Path) {
        let units = if self.moving && same_volume(src, dir) { Some(ITEM_UNITS) } else { weight(src, self.progress) };
        self.progress.add(units.unwrap_or(0));
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
        self.progress.add(ITEM_UNITS);
        Ok(Placed::Done(target.to_path_buf()))
    }

    /// Copies or moves `src` to the free path `target`, which `replaced`
    /// was until it went to the Trash.
    fn put(&mut self, src: &Path, target: &Path, replaced: Option<Trashed>) -> Result<Placed, (PathBuf, io::Error)> {
        let fail = |e: io::Error| (src.to_path_buf(), e);
        let cancelled = if self.moving {
            let cancelled = move_with(src, target, self.progress).map_err(fail)?;
            if !cancelled {
                self.moved.push((src.to_path_buf(), target.to_path_buf()));
            }
            cancelled
        } else {
            let cancelled = self.copy(src, target).map_err(fail)?;
            if !cancelled {
                self.copies.push(target.to_path_buf());
            }
            cancelled
        };
        match replaced {
            // Nothing took its place: it comes back.
            Some(gone) if cancelled => self.trash.restore(&gone).map_err(|e| (gone.original.clone(), e))?,
            Some(gone) => self.replaced.push(gone),
            None => {}
        }
        Ok(if cancelled { Placed::Cancelled } else { Placed::Done(target.to_path_buf()) })
    }

    /// Copies `src` to the free path `dest`. `Ok(true)` when cancelled; the
    /// incomplete copy is removed then.
    fn copy(&self, src: &Path, dest: &Path) -> io::Result<bool> {
        copy_cancellable(src, dest, self.progress)
    }
}

/// [`copy_tree`] that removes what it made when it fails or is cancelled.
/// `Ok(true)` when cancelled.
fn copy_cancellable(src: &Path, dest: &Path, progress: &Progress) -> io::Result<bool> {
    match copy_tree(src, dest, progress) {
        Ok(()) => Ok(false),
        Err(e) => {
            // `dest` was free, so everything there is this copy's.
            if dest.symlink_metadata().is_ok_and(|m| m.is_dir()) {
                let _ = std::fs::remove_dir_all(dest);
            } else {
                let _ = std::fs::remove_file(dest);
            }
            if progress.is_cancelled() { Ok(true) } else { Err(e) }
        }
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

/// The first free "Name - Shortcut.ext", "Name - Shortcut (2).ext", ... in
/// `dir`. Windows 10 names a shortcut "Name.ext - Shortcut.lnk"; a symbolic
/// link has no `.lnk`, so the extension stays last and the link keeps its
/// file's type.
pub fn shortcut_name(dir: &Path, name: &OsStr, is_dir: bool) -> PathBuf {
    let path = Path::new(name);
    let (stem, ext) = match path.extension() {
        Some(ext) if !is_dir => (path.file_stem().unwrap_or(name), Some(ext)),
        _ => (name, None),
    };
    (1..)
        .map(|n| {
            let mut candidate = OsString::from(stem);
            candidate.push(if n == 1 { " - Shortcut".to_string() } else { format!(" - Shortcut ({n})") });
            if let Some(ext) = ext {
                candidate.push(".");
                candidate.push(ext);
            }
            dir.join(candidate)
        })
        .find(|p| p.symlink_metadata().is_err())
        .expect("an unbounded range always yields a free name")
}

/// Makes a symbolic link to each of `sources` in the folder `dest`, named by
/// [`shortcut_name`] (a drop with the shortcut effect). Stops at the first
/// error; the undo step removes the links made.
#[cfg(unix)]
pub fn link_into(sources: &[PathBuf], dest: &Path) -> Transferred {
    let mut out = Transferred::default();
    for src in sources {
        let name = src.file_name().map_or_else(|| OsString::from("Root"), OsStr::to_os_string);
        let link = shortcut_name(dest, &name, src.is_dir());
        match std::os::unix::fs::symlink(src, &link) {
            Ok(()) => out.placed.push(link),
            Err(e) => {
                out.error = Some((src.clone(), e));
                break;
            }
        }
    }
    out.undo = Some(UndoOp::Copy(out.placed.clone()));
    out
}

/// Moves `src` to the path `dest`, across volumes too, never replacing.
pub fn move_to(src: &Path, dest: &Path) -> io::Result<()> {
    move_with(src, dest, &Progress::default()).map(drop)
}

/// [`move_to`], reporting to `progress`. `Ok(true)` when cancelled while
/// copying to another volume; the original stays then.
fn move_with(src: &Path, dest: &Path, progress: &Progress) -> io::Result<bool> {
    match rename_no_replace(src, dest) {
        Ok(()) => {
            progress.add(ITEM_UNITS);
            Ok(false)
        }
        // Another volume: copy, then remove the original.
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            if dest.symlink_metadata().is_ok() {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            if copy_cancellable(src, dest, progress)? {
                return Ok(true);
            }
            if src.symlink_metadata()?.is_dir() { std::fs::remove_dir_all(src)? } else { std::fs::remove_file(src)? }
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Copies the file, folder or link `src` to the free path `dest`, with
/// what's inside, adding to `progress` as it goes. Stops with an error when
/// `progress` is cancelled, leaving a partial copy.
fn copy_tree(src: &Path, dest: &Path, progress: &Progress) -> io::Result<()> {
    progress.cancelled_error()?;
    let meta = src.symlink_metadata()?;
    if meta.is_dir() {
        std::fs::create_dir(dest)?;
        let mut items: Vec<_> = std::fs::read_dir(src)?.collect::<io::Result<_>>()?;
        items.sort_by_key(|i| i.file_name());
        for item in items {
            copy_tree(&item.path(), &dest.join(item.file_name()), progress)?;
        }
        // After the contents, so its times and a read-only mode stick.
        copy_folder_metadata(src, dest, &meta)?;
        progress.add(ITEM_UNITS);
        return Ok(());
    }
    let copied = copy_file(src, dest, &meta, progress)?;
    let size = if meta.is_file() { meta.len() } else { 0 };
    progress.add(size.saturating_sub(copied) + ITEM_UNITS);
    Ok(())
}

#[cfg(target_os = "macos")]
fn c_path(p: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other)
}

/// Copies a file or link with its metadata and extended attributes
/// (cloned on APFS). Returns the bytes it reported to `progress`.
#[cfg(target_os = "macos")]
fn copy_file(src: &Path, dest: &Path, _meta: &std::fs::Metadata, progress: &Progress) -> io::Result<u64> {
    use std::ffi::{c_char, c_int, c_void};
    struct Status<'a> {
        progress: &'a Progress,
        copied: u64,
    }
    extern "C" fn status(what: c_int, stage: c_int, state: libc::copyfile_state_t, _: *const c_char, _: *const c_char, ctx: *mut c_void) -> c_int {
        // SAFETY: `ctx` is the `Status` below, alive for the whole copyfile call.
        let status = unsafe { &mut *(ctx as *mut Status) };
        if what == libc::COPYFILE_COPY_DATA && stage == libc::COPYFILE_PROGRESS {
            let mut copied: libc::off_t = 0;
            // SAFETY: COPYFILE_STATE_COPIED writes an off_t.
            if unsafe { libc::copyfile_state_get(state, libc::COPYFILE_STATE_COPIED as u32, (&raw mut copied).cast()) } == 0 {
                let copied = copied.max(0) as u64;
                status.progress.add(copied.saturating_sub(status.copied));
                status.copied = status.copied.max(copied);
            }
        }
        if status.progress.is_cancelled() { libc::COPYFILE_QUIT } else { libc::COPYFILE_CONTINUE }
    }
    let (csrc, cdest) = (c_path(src)?, c_path(dest)?);
    let mut ctx = Status { progress, copied: 0 };
    let flags = libc::COPYFILE_METADATA | libc::COPYFILE_DATA | libc::COPYFILE_CLONE | libc::COPYFILE_NOFOLLOW | libc::COPYFILE_EXCL;
    // SAFETY: the state is freed below; the callback and its context outlive the call.
    let result = unsafe {
        let state = libc::copyfile_state_alloc();
        let callback: libc::copyfile_callback_t = Some(status);
        libc::copyfile_state_set(state, libc::COPYFILE_STATE_STATUS_CB as u32, callback.map_or(std::ptr::null(), |f| f as *const c_void));
        libc::copyfile_state_set(state, libc::COPYFILE_STATE_STATUS_CTX as u32, (&raw mut ctx).cast());
        let result = libc::copyfile(csrc.as_ptr(), cdest.as_ptr(), state, flags);
        let error = io::Error::last_os_error();
        libc::copyfile_state_free(state);
        if result == 0 { Ok(()) } else { Err(error) }
    };
    progress.cancelled_error()?;
    result.map(|()| ctx.copied)
}

/// A folder's permissions, flags, times, ACL and extended attributes.
#[cfg(target_os = "macos")]
fn copy_folder_metadata(src: &Path, dest: &Path, _meta: &std::fs::Metadata) -> io::Result<()> {
    let (csrc, cdest) = (c_path(src)?, c_path(dest)?);
    // SAFETY: both are valid NUL-terminated paths; a null state is allowed.
    if unsafe { libc::copyfile(csrc.as_ptr(), cdest.as_ptr(), std::ptr::null_mut(), libc::COPYFILE_METADATA | libc::COPYFILE_NOFOLLOW) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Copies a file in pieces, so the bar moves and Cancel works within a big
/// file, or recreates a link. Returns the bytes it reported to `progress`.
#[cfg(not(target_os = "macos"))]
fn copy_file(src: &Path, dest: &Path, meta: &std::fs::Metadata, progress: &Progress) -> io::Result<u64> {
    use std::io::Read;
    #[cfg(unix)]
    if meta.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dest)?;
        return Ok(0);
    }
    const PIECE: u64 = 1 << 20;
    let from = std::fs::File::open(src)?;
    let mut to = std::fs::File::create_new(dest)?;
    let mut copied = 0;
    loop {
        progress.cancelled_error()?;
        let n = io::copy(&mut (&from).take(PIECE), &mut to)?;
        if n == 0 {
            break;
        }
        copied += n;
        progress.add(n);
    }
    std::fs::set_permissions(dest, meta.permissions())?;
    Ok(copied)
}

#[cfg(not(target_os = "macos"))]
fn copy_folder_metadata(_src: &Path, dest: &Path, meta: &std::fs::Metadata) -> io::Result<()> {
    std::fs::set_permissions(dest, meta.permissions())
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
    fn shortcuts_link_to_their_items() {
        let dir = scratch("links");
        fs::create_dir_all(dir.join("folder")).unwrap();
        fs::write(dir.join("a.txt"), "a").unwrap();
        let out = dir.join("out");
        fs::create_dir(&out).unwrap();
        let sources = [dir.join("a.txt"), dir.join("folder")];
        let first = link_into(&sources, &out);
        assert!(first.error.is_none());
        assert_eq!(first.placed, [out.join("a - Shortcut.txt"), out.join("folder - Shortcut")]);
        assert_eq!(fs::read_link(&first.placed[0]).unwrap(), sources[0]);
        assert_eq!(link_into(&sources[..1], &out).placed, [out.join("a - Shortcut (2).txt")]);
        assert_eq!(first.undo, Some(UndoOp::Copy(first.placed.clone())));
        fs::remove_dir_all(&dir).unwrap();
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

    #[cfg(unix)]
    #[test]
    fn progress_adds_up_to_the_total() {
        let dir = scratch("progress");
        fs::create_dir_all(dir.join("src/sub")).unwrap();
        fs::write(dir.join("src/a"), vec![7u8; 300_000]).unwrap();
        fs::write(dir.join("src/sub/b"), "b").unwrap();
        std::os::unix::fs::symlink("a", dir.join("src/link")).unwrap();
        fs::create_dir(dir.join("dst")).unwrap();
        let progress = Progress::default();
        let done = transfer_with(&[dir.join("src")], &dir.join("dst"), false, &trash_in(&dir), never, &progress);
        assert!(done.error.is_none() && !done.cancelled);
        let view = progress.view();
        // Three items in src, src itself, and the bytes of a and b.
        assert_eq!(view.total, 5 * ITEM_UNITS + 300_001);
        assert_eq!(progress.done.load(Ordering::Relaxed), view.total);
        assert_eq!(view.item, Some((dir.join("src"), dir.join("dst"))));
        assert_eq!(fs::read(dir.join("dst/src/a")).unwrap().len(), 300_000);
        assert_eq!(fs::read_link(dir.join("dst/src/link")).unwrap(), PathBuf::from("a"));

        // A move within the volume renames: one item.
        let progress = Progress::default();
        fs::create_dir(dir.join("moved")).unwrap();
        transfer_with(&[dir.join("dst/src")], &dir.join("moved"), true, &trash_in(&dir), never, &progress);
        assert_eq!(progress.view(), ProgressView { done: ITEM_UNITS, total: ITEM_UNITS, item: Some((dir.join("dst/src"), dir.join("moved"))) });
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn copies_keep_attributes_times_and_read_only_folders() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("attrs");
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/f"), "x").unwrap();
        for path in [src.join("sub"), src.join("sub/f")] {
            let ok = std::process::Command::new("xattr").args(["-w", "dev.macsplorer.test", "kept"]).arg(&path).status().unwrap();
            assert!(ok.success());
        }
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::File::open(src.join("sub")).unwrap().set_modified(old).unwrap();
        fs::set_permissions(src.join("sub"), fs::Permissions::from_mode(0o555)).unwrap();
        let done = transfer(std::slice::from_ref(&src), &dir, false, &trash_in(&dir), never);
        assert!(done.error.is_none(), "{:?}", done.error);
        let copy = dir.join("src - Copy/sub");
        for path in [copy.clone(), copy.join("f")] {
            let out = std::process::Command::new("xattr").args(["-p", "dev.macsplorer.test"]).arg(&path).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "kept", "{}", path.display());
        }
        let meta = fs::metadata(&copy).unwrap();
        assert_eq!(meta.modified().unwrap(), old);
        assert_eq!(meta.permissions().mode() & 0o777, 0o555);
        for sub in [src.join("sub"), copy] {
            fs::set_permissions(sub, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancel_while_replacing_puts_the_old_item_back() {
        let (dir, src, dst) = clash("cancelreplace");
        fs::write(src.join("b.txt"), "b").unwrap();
        let progress = Progress::default();
        let trash = trash_in(&dir);
        let done = transfer_with(&[src.join("a.txt"), src.join("b.txt")], &dst, false, &trash, |_| {
            // Cancel clicked in the progress dialog while the question was up.
            progress.cancel();
            Choice::Yes
        }, &progress);
        assert!(done.cancelled);
        assert!(done.placed.is_empty());
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
        assert!(!dst.join("b.txt").exists());
        assert_eq!(done.undo, Some(UndoOp::Copy(Vec::new())));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancel_while_counting_changes_nothing() {
        let (dir, src, dst) = clash("cancelcount");
        let progress = Progress::default();
        progress.cancel();
        let done = transfer_with(&[src.join("a.txt")], &dst.join("sub"), false, &trash_in(&dir), never, &progress);
        assert!(done.cancelled && done.placed.is_empty());
        assert_eq!(progress.view().total, 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancelled_copy_leaves_nothing_behind() {
        let dir = scratch("cancelcopy");
        fs::create_dir_all(dir.join("src/sub")).unwrap();
        fs::write(dir.join("src/sub/f"), "x").unwrap();
        let progress = Progress::default();
        progress.total.store(1, Ordering::Relaxed);
        progress.cancel();
        assert!(copy_cancellable(&dir.join("src"), &dir.join("copy"), &progress).unwrap());
        assert!(!dir.join("copy").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
