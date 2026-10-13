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

#[cfg(not(target_os = "macos"))]
pub fn move_to_trash(_path: &Path) -> io::Result<()> {
    Err(io::Error::other("the Trash is only available on macOS"))
}
