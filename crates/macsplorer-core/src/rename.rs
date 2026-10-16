//! In-place rename rules.
//!
//! The initial selection follows Explorer's `SHBeginLabelEdit` (shell32,
//! build 10240; see `docs/file-browser-findings.md` §2.1):
//!
//! - real folders get the whole name selected;
//! - when extensions are hidden the whole (extension-less) name is selected;
//! - otherwise the name minus its extension is selected, provided the
//!   extension is shorter than the whole name.
//!
//! Explorer treats zip files (folder + stream) like files. The macOS
//! equivalent is a package such as `Foo.app`, so packages count as files.
//!
//! Name validation does not copy Windows' 128-character default. It follows
//! APFS/HFS+ instead.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::Path;

/// Longest file name APFS and HFS+ accept, in UTF-8 bytes (`NAME_MAX`).
pub const MAX_NAME_BYTES: usize = 255;

/// What kind of item is being renamed, as far as selection is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameKind {
    File,
    Folder,
    /// A directory presented as a single item (`.app`, `.bundle`, …).
    Package,
}

/// Returns the character range to select when the rename box opens.
///
/// `name` is the text shown in the box. Ranges are in `char`s, not bytes,
/// because that is what text widgets position cursors by.
pub fn initial_selection(name: &str, kind: RenameKind, extensions_shown: bool) -> Range<usize> {
    let len = name.chars().count();
    if kind == RenameKind::Folder || !extensions_shown {
        return 0..len;
    }
    match extension_len(name) {
        Some(ext) if ext < len => 0..len - ext,
        _ => 0..len,
    }
}

/// Length in chars of the extension including its dot, if the name has one.
///
/// Mirrors `Path::extension`: a leading dot (`.gitignore`) is not an
/// extension and neither is a trailing one (`notes.`).
fn extension_len(name: &str) -> Option<usize> {
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 == name.len() {
        return None;
    }
    Some(name[dot..].chars().count())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    Reserved,
    InvalidChar(char),
    TooLong { bytes: usize },
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NameError::Empty => write!(f, "A name can't be empty."),
            NameError::Reserved => write!(f, "\".\" and \"..\" are reserved names."),
            NameError::InvalidChar(c) => write!(f, "A name can't contain {c:?}."),
            NameError::TooLong { bytes } => {
                write!(f, "The name is {bytes} bytes long; the limit is {MAX_NAME_BYTES}.")
            }
        }
    }
}

impl std::error::Error for NameError {}

/// Returns true for characters the rename box should refuse as they are typed.
pub fn is_invalid_char(c: char) -> bool {
    c == '/' || c == '\0'
}

/// Checks a proposed name before it is committed.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.trim().is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::Reserved);
    }
    if let Some(c) = name.chars().find(|&c| is_invalid_char(c)) {
        return Err(NameError::InvalidChar(c));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(NameError::TooLong { bytes: name.len() });
    }
    Ok(())
}

/// Renames `from` to `to` without ever replacing another item.
///
/// On macOS this is one atomic `renamex_np(RENAME_EXCL)`, which still allows
/// a case-only rename (`a` → `A`) of the same item on a case-insensitive
/// volume but fails with `AlreadyExists` if `to` is a different item. Volumes
/// without `RENAME_EXCL` (some network and FAT drives) fall back to checking
/// first, which leaves a small race.
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c = |p: &Path| CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other);
        let (cfrom, cto) = (c(from)?, c(to)?);
        // SAFETY: both are valid NUL-terminated paths.
        if unsafe { libc::renamex_np(cfrom.as_ptr(), cto.as_ptr(), libc::RENAME_EXCL) } == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if !matches!(err.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL)) {
            return Err(err);
        }
    }
    #[cfg(unix)]
    if let (Ok(a), Ok(b)) = (fs::symlink_metadata(from), fs::symlink_metadata(to)) {
        use std::os::unix::fs::MetadataExt;
        if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
    }
    fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RenameKind::*;

    #[test]
    fn rename_never_replaces() {
        let dir = std::env::temp_dir().join(format!("macsplorer-rename-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        let err = rename_no_replace(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&b).unwrap(), "b");
        // Case-only renames work on case-insensitive and case-sensitive volumes.
        rename_no_replace(&a, &dir.join("A")).unwrap();
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(names.iter().any(|n| n == "A"), "{names:?}");
        assert!(!names.iter().any(|n| n == "a"), "{names:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    fn sel(name: &str, kind: RenameKind) -> &str {
        let r = initial_selection(name, kind, true);
        let end = name.char_indices().nth(r.end).map_or(name.len(), |(i, _)| i);
        &name[..end]
    }

    // The table in docs/file-browser-findings.md §2.1.
    #[test]
    fn matches_explorer_table() {
        assert_eq!(sel("report.final.docx", File), "report.final");
        assert_eq!(sel("archive.tar.gz", File), "archive.tar");
        assert_eq!(sel(".gitignore", File), ".gitignore");
        assert_eq!(sel("Makefile", File), "Makefile");
        assert_eq!(sel("My.Folder", Folder), "My.Folder");
        assert_eq!(initial_selection("photo", File, false), 0..5);
    }

    #[test]
    fn packages_behave_like_files() {
        assert_eq!(sel("Safari.app", Package), "Safari");
    }

    #[test]
    fn trailing_dot_is_not_an_extension() {
        assert_eq!(sel("notes.", File), "notes.");
    }

    #[test]
    fn counts_chars_not_bytes() {
        assert_eq!(initial_selection("über.txt", File, true), 0..4);
    }

    #[test]
    fn validation() {
        assert_eq!(validate_name("ok.txt"), Ok(()));
        assert_eq!(validate_name("  "), Err(NameError::Empty));
        assert_eq!(validate_name(".."), Err(NameError::Reserved));
        assert_eq!(validate_name("a/b"), Err(NameError::InvalidChar('/')));
        assert_eq!(validate_name("a:b"), Ok(()));
        assert!(matches!(validate_name(&"x".repeat(256)), Err(NameError::TooLong { .. })));
    }
}
