//! Extended attributes on macOS and Linux, which name the same calls with
//! different arguments. They follow symlinks, like the rest of the listing.

use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// What `getxattr` and `removexattr` set when the attribute isn't there.
#[cfg(target_os = "macos")]
const NO_ATTRIBUTE: i32 = libc::ENOATTR;
#[cfg(target_os = "linux")]
const NO_ATTRIBUTE: i32 = libc::ENODATA;

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)
}

/// Reads an attribute of up to 4 KB, `None` if it isn't there or can't be read.
pub fn read(path: &Path, name: &CStr) -> Option<Vec<u8>> {
    let cpath = c_path(path).ok()?;
    let mut buf = vec![0u8; 4096];
    // SAFETY: valid NUL-terminated path and name, and a buffer of buf.len() bytes.
    #[cfg(target_os = "macos")]
    let n = unsafe { libc::getxattr(cpath.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0, 0) };
    // SAFETY: as above.
    #[cfg(target_os = "linux")]
    let n = unsafe { libc::getxattr(cpath.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    let n = usize::try_from(n).ok()?;
    buf.truncate(n);
    Some(buf)
}

pub fn set(path: &Path, name: &CStr, value: &[u8]) -> io::Result<()> {
    let cpath = c_path(path)?;
    // SAFETY: valid NUL-terminated path and name, and `value.len()` bytes at `value`.
    #[cfg(target_os = "macos")]
    let r = unsafe { libc::setxattr(cpath.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
    // SAFETY: as above.
    #[cfg(target_os = "linux")]
    let r = unsafe { libc::setxattr(cpath.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Removes an attribute; one that isn't there is fine.
pub fn remove(path: &Path, name: &CStr) -> io::Result<()> {
    let cpath = c_path(path)?;
    // SAFETY: valid NUL-terminated path and name.
    #[cfg(target_os = "macos")]
    let r = unsafe { libc::removexattr(cpath.as_ptr(), name.as_ptr(), 0) };
    // SAFETY: as above.
    #[cfg(target_os = "linux")]
    let r = unsafe { libc::removexattr(cpath.as_ptr(), name.as_ptr()) };
    let err = io::Error::last_os_error();
    if r == 0 || err.raw_os_error() == Some(NO_ATTRIBUTE) { Ok(()) } else { Err(err) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skips quietly where the temporary folder's file system has no user
    /// attributes (some tmpfs).
    #[test]
    fn set_read_remove() {
        let dir = std::env::temp_dir().join(format!("macsplorer-xattr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = c"user.macsplorer.test";
        if set(&dir, name, b"#008080").is_ok() {
            assert_eq!(read(&dir, name).as_deref(), Some(&b"#008080"[..]));
            remove(&dir, name).unwrap();
            assert_eq!(read(&dir, name), None);
            remove(&dir, name).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
