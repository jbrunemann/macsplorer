//! Copy and move for the clipboard's Paste.
//!
//! Windows does this in `CFileOperation` / `CCopyWorkItem` (Windows.Storage);
//! that engine is not traced yet (`docs/file-browser-findings.md` §4). What
//! is implemented here is the observable behavior:
//!
//! - Pasting a copy next to its source, or over an existing name, creates
//!   "Name - Copy.ext", then "Name - Copy (2).ext" and so on. Explorer shows a
//!   replace/skip dialog for other collisions; this never overwrites instead
//!   (unverified simplification, there is no dialog yet).
//! - Moving never replaces either: it fails with `AlreadyExists`.
//! - Moving into the folder an item already lives in does nothing.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

use crate::rename::rename_no_replace;

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

/// Copies `src` into the folder `dir` and returns the new path. Keeps
/// extended attributes (Finder custom icons, tags) and clones on APFS.
pub fn copy_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().ok_or_else(|| io::Error::other("the item has no name"))?;
    let is_dir = src.symlink_metadata()?.is_dir();
    if is_dir && dir.starts_with(src) {
        return Err(io::Error::other("a folder can't be copied into itself"));
    }
    let direct = dir.join(name);
    let dest = if direct.symlink_metadata().is_ok() { copy_name(dir, name, is_dir) } else { direct };
    copy_tree(src, &dest)?;
    Ok(dest)
}

/// Moves `src` into the folder `dir` and returns the new path.
pub fn move_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().ok_or_else(|| io::Error::other("the item has no name"))?;
    let dest = dir.join(name);
    if dest == src {
        return Ok(dest);
    }
    if src.symlink_metadata()?.is_dir() && dir.starts_with(src) {
        return Err(io::Error::other("a folder can't be moved into itself"));
    }
    move_to(src, &dest)?;
    Ok(dest)
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

#[cfg(not(target_os = "macos"))]
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

    #[test]
    fn copy_next_to_source_and_into_other_folder() {
        let dir = scratch("copy");
        let other = dir.join("other");
        fs::create_dir(&other).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "hi").unwrap();
        let twin = copy_into(&file, &dir).unwrap();
        assert_eq!(twin, dir.join("a - Copy.txt"));
        assert_eq!(copy_into(&file, &other).unwrap(), other.join("a.txt"));
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
        assert!(copy_into(&src, &src.join("sub")).is_err());
        let copy = copy_into(&src, &dir).unwrap();
        assert_eq!(copy, dir.join("src - Copy"));
        assert_eq!(fs::read_to_string(copy.join("sub/f")).unwrap(), "x");
        assert!(copy.join("link").symlink_metadata().unwrap().is_symlink());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn move_never_replaces() {
        let dir = scratch("move");
        let other = dir.join("other");
        fs::create_dir(&other).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "new").unwrap();
        assert_eq!(move_into(&file, &dir).unwrap(), file);
        fs::write(other.join("a.txt"), "old").unwrap();
        let err = move_into(&file, &other).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(other.join("a.txt")).unwrap(), "old");
        fs::remove_file(other.join("a.txt")).unwrap();
        assert_eq!(move_into(&file, &other).unwrap(), other.join("a.txt"));
        assert!(!file.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn move_folder_into_itself_fails() {
        let dir = scratch("selfmove");
        let sub = dir.join("a/b");
        fs::create_dir_all(&sub).unwrap();
        assert!(move_into(&dir.join("a"), &sub).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
