//! Delete = move to the Trash, like Explorer's Delete moves to the Recycle
//! Bin (`CFileOperation::DeleteItems` with the recycle flag, not traced yet).
//! Nothing is destroyed, so there is no confirmation dialog. Undo puts the
//! item back where it was.

use std::io;
use std::path::Path;

use macsplorer_core::undo::{Trash, Trashed};

pub struct SystemTrash;

impl Trash for SystemTrash {
    fn trash(&self, path: &Path) -> io::Result<Trashed> {
        let in_trash = move_to_trash(path)?;
        Ok(Trashed { original: path.to_path_buf(), in_trash })
    }

    fn restore(&self, item: &Trashed) -> io::Result<()> {
        restore(item)
    }
}

/// Returns where the item ended up in the Trash.
#[cfg(target_os = "macos")]
fn move_to_trash(path: &Path) -> io::Result<Option<std::path::PathBuf>> {
    use objc2_foundation::{NSFileManager, NSURL};

    let url = NSURL::from_file_path(path).ok_or_else(|| io::Error::other("invalid path"))?;
    let mut resulting = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting))
        .map_err(|e| io::Error::other(e.localizedDescription().to_string()))?;
    Ok(resulting.and_then(|u| u.to_file_path()))
}

#[cfg(target_os = "macos")]
fn restore(item: &Trashed) -> io::Result<()> {
    let from = item.in_trash.as_deref().ok_or_else(|| io::Error::other("the Trash didn't say where it put the item"))?;
    macsplorer_core::fileops::move_to(from, &item.original)
}

/// The freedesktop trash (`~/.local/share/Trash`, or a per-volume `.Trash-uid`).
/// It doesn't report where the item went; `restore` looks it up by its
/// original path.
#[cfg(not(target_os = "macos"))]
fn move_to_trash(path: &Path) -> io::Result<Option<std::path::PathBuf>> {
    trash::delete(path).map(|()| None).map_err(|e| io::Error::other(e.to_string()))
}

/// Restores the most recently trashed item with this original path.
#[cfg(not(target_os = "macos"))]
fn restore(item: &Trashed) -> io::Result<()> {
    use trash::os_limited;

    if item.original.symlink_metadata().is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    let err = |e: trash::Error| io::Error::other(e.to_string());
    let found = os_limited::list()
        .map_err(err)?
        .into_iter()
        .filter(|t| t.original_path() == item.original)
        .max_by_key(|t| t.time_deleted)
        .ok_or_else(|| io::Error::other("it is no longer in the Trash"))?;
    os_limited::restore_all([found]).map_err(err)
}
