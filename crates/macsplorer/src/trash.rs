//! Delete = move to the Trash, like Explorer's Delete moves to the Recycle
//! Bin (`CFileOperation::DeleteItems` with the recycle flag, not traced yet).
//! Nothing is destroyed, so there is no confirmation dialog.

use std::io;
use std::path::Path;

#[cfg(target_os = "macos")]
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    use objc2_foundation::{NSFileManager, NSURL};

    let url = NSURL::from_file_path(path).ok_or_else(|| io::Error::other("invalid path"))?;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|e| io::Error::other(e.localizedDescription().to_string()))
}

/// The freedesktop trash (`~/.local/share/Trash`, or a per-volume `.Trash-uid`).
#[cfg(not(target_os = "macos"))]
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    trash::delete(path).map_err(|e| io::Error::other(e.to_string()))
}
