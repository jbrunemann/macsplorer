//! Directory listing and sorting for the details view.

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::natural::{natural_cmp, plain_cmp};
use crate::rename::RenameKind;

/// Directory extensions shown as a single item, like Finder does.
///
/// Finder asks LaunchServices; this list is an approximation until the app
/// calls `NSWorkspace.isFilePackageAtPath`.
const PACKAGE_EXTENSIONS: &[&str] = &[
    "app", "appex", "bundle", "framework", "plugin", "kext", "pkg", "mpkg", "rtfd",
    "photoslibrary", "musiclibrary", "xcodeproj", "xcworkspace", "playground", "scptd",
];

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: RenameKind,
    /// `None` for folders and packages.
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
    /// Finder shows its own icon for this item (`kHasCustomIcon`).
    pub custom_icon: bool,
}

impl Entry {
    pub fn is_folder(&self) -> bool {
        self.kind == RenameKind::Folder
    }

    /// The extension without its dot, if any.
    pub fn extension(&self) -> Option<&str> {
        Path::new(&self.name).extension().and_then(|e| e.to_str())
    }

    /// The extension the display name leaves out, if any.
    ///
    /// Folders always keep their full name. So does a name whose stem looks
    /// like it has an extension of its own: `Invoice.pdf.app` and
    /// `photo.jpg.command` show in full rather than as `Invoice.pdf` and
    /// `photo.jpg`, so an app or script can't pass for a document.
    pub fn hidden_extension(&self, extensions_shown: bool) -> Option<&str> {
        if extensions_shown || self.is_folder() {
            return None;
        }
        let stem = Path::new(&self.name).file_stem()?;
        if Path::new(stem).extension().is_some() {
            return None;
        }
        self.extension()
    }

    /// Display name, without the extension when extensions are hidden
    /// (see [`hidden_extension`](Self::hidden_extension)).
    pub fn display_name(&self, extensions_shown: bool) -> &str {
        match self.hidden_extension(extensions_shown) {
            Some(ext) => &self.name[..self.name.len() - ext.len() - 1],
            None => &self.name,
        }
    }

    /// Explorer-style type description: "Folder", "TXT File", "File".
    pub fn kind_label(&self) -> String {
        match (self.kind, self.extension()) {
            (RenameKind::Folder, _) => "Folder".into(),
            (RenameKind::Package, Some("app")) => "Application".into(),
            (RenameKind::Package, _) => "Package".into(),
            (RenameKind::File, Some(ext)) => format!("{} File", ext.to_uppercase()),
            (RenameKind::File, None) => "File".into(),
        }
    }
}

/// Reads one directory. Entries that vanish or can't be read while listing
/// are skipped rather than failing the whole listing.
pub fn list_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let hidden = name.starts_with('.');
        if hidden && !show_hidden {
            continue;
        }
        let path = item.path();
        // Follow symlinks so a link to a folder behaves like a folder.
        let Ok(meta) = fs::metadata(&path).or_else(|_| item.metadata()) else { continue };
        let kind = if !meta.is_dir() {
            RenameKind::File
        } else if is_package_name(&name) {
            RenameKind::Package
        } else {
            RenameKind::Folder
        };
        out.push(Entry {
            size: (kind == RenameKind::File).then_some(meta.len()),
            modified: meta.modified().ok(),
            custom_icon: has_custom_icon(&path),
            name,
            path,
            kind,
            hidden,
        });
    }
    Ok(out)
}

/// True if Finder has a custom icon for `path`: the `kHasCustomIcon` bit
/// (0x0400) in the Finder flags of the `com.apple.FinderInfo` attribute.
#[cfg(target_os = "macos")]
pub fn has_custom_icon(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    const K_HAS_CUSTOM_ICON: u16 = 0x0400;
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    let mut info = [0u8; 32];
    // SAFETY: valid NUL-terminated path, name and a 32-byte buffer.
    let n = unsafe {
        libc::getxattr(cpath.as_ptr(), c"com.apple.FinderInfo".as_ptr(), info.as_mut_ptr().cast(), info.len(), 0, 0)
    };
    n >= 10 && u16::from_be_bytes([info[8], info[9]]) & K_HAS_CUSTOM_ICON != 0
}

#[cfg(not(target_os = "macos"))]
pub fn has_custom_icon(_path: &Path) -> bool {
    false
}

pub(crate) fn is_package_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PACKAGE_EXTENSIONS.iter().any(|p| p.eq_ignore_ascii_case(e)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Modified,
    Kind,
    Size,
}

#[derive(Clone, Copy, Debug)]
pub struct SortSpec {
    pub column: SortColumn,
    pub ascending: bool,
    /// Keep folders above files regardless of column. Explorer's details
    /// view does this; the exact rule in `CFSFolder::CompareIDs` is not yet
    /// traced.
    pub folders_first: bool,
    /// `false` gives the `NoStrCmpLogical` plain ordering.
    pub natural: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self { column: SortColumn::Name, ascending: true, folders_first: true, natural: true }
    }
}

/// Sorts in place. Ties on the chosen column fall back to name order, so the
/// result is stable across refreshes.
pub fn sort_entries(entries: &mut [Entry], spec: SortSpec) {
    let names = if spec.natural { natural_cmp } else { plain_cmp };
    entries.sort_by(|a, b| {
        if spec.folders_first {
            let group = b.is_folder().cmp(&a.is_folder());
            if group != Ordering::Equal {
                return group;
            }
        }
        let by_column = match spec.column {
            SortColumn::Name => names(&a.name, &b.name),
            SortColumn::Modified => a.modified.cmp(&b.modified),
            SortColumn::Kind => names(&a.kind_label(), &b.kind_label()),
            SortColumn::Size => a.size.cmp(&b.size),
        };
        let by_column = if spec.ascending { by_column } else { by_column.reverse() };
        by_column.then_with(|| names(&a.name, &b.name))
    });
}

/// Bytes available to the current user on the volume holding `path`, like
/// `GetDiskFreeSpaceEx`'s `lpFreeBytesAvailable` (what the status bar's
/// "Disk free space" shows). `None` if the volume can't be queried.
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is plain old data, so all-zero is a valid value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid NUL-terminated path and a statvfs buffer to fill.
    if unsafe { libc::statvfs(cpath.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(u64::from(st.f_bavail) * st.f_frsize)
}

/// Formats a byte count the way Explorer's Size column does: whole KB,
/// rounded up, so a 1-byte file shows as "1 KB".
pub fn format_size(bytes: u64) -> String {
    let kb = bytes.div_ceil(1024);
    let digits = kb.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped} KB")
}

/// Formats a byte count like Windows' `StrFormatByteSize`, used by the
/// status bar: "512 bytes", "1.00 KB", "12.3 KB", "123 KB", "1.20 MB".
/// Three significant digits, truncated rather than rounded.
pub fn format_byte_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let units = ["KB", "MB", "GB", "TB", "PB", "EB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    let decimals = if value < 10.0 { 2 } else if value < 100.0 { 1 } else { 0 };
    let scale = 10f64.powi(decimals);
    let truncated = (value * scale).floor() / scale;
    format!("{truncated:.prec$} {}", units[unit], prec = decimals as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: RenameKind, size: Option<u64>) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from(name),
            kind,
            size,
            modified: None,
            hidden: false,
            custom_icon: false,
        }
    }

    #[test]
    fn folders_first_then_natural_names() {
        let mut v = vec![
            entry("b10.txt", RenameKind::File, Some(1)),
            entry("zeta", RenameKind::Folder, None),
            entry("b2.txt", RenameKind::File, Some(1)),
            entry("Alpha", RenameKind::Folder, None),
        ];
        sort_entries(&mut v, SortSpec::default());
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta", "b2.txt", "b10.txt"]);
    }

    #[test]
    fn descending_keeps_folders_on_top() {
        let mut v = vec![
            entry("a", RenameKind::Folder, None),
            entry("small", RenameKind::File, Some(1)),
            entry("big", RenameKind::File, Some(9)),
        ];
        sort_entries(&mut v, SortSpec { column: SortColumn::Size, ascending: false, ..Default::default() });
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a", "big", "small"]);
    }

    #[test]
    fn display_name_hides_extension_only_for_files() {
        let f = entry("photo.JPG", RenameKind::File, Some(1));
        assert_eq!(f.display_name(false), "photo");
        assert_eq!(entry(".zshrc", RenameKind::File, Some(1)).display_name(false), ".zshrc");
        assert_eq!(entry("My.Folder", RenameKind::Folder, None).display_name(false), "My.Folder");
        assert_eq!(entry("Safari.app", RenameKind::Package, None).display_name(false), "Safari");
    }

    #[test]
    fn double_extensions_stay_visible() {
        for name in ["Invoice.pdf.app", "photo.jpg.command", "a.tar.gz", "notes.txt .sh"] {
            let kind = if name.ends_with(".app") { RenameKind::Package } else { RenameKind::File };
            let e = entry(name, kind, None);
            assert_eq!(e.display_name(false), name);
            assert_eq!(e.hidden_extension(false), None);
        }
        assert_eq!(entry(".zshrc.bak", RenameKind::File, Some(1)).display_name(false), ".zshrc");
    }

    #[test]
    fn kind_labels() {
        assert_eq!(entry("a.txt", RenameKind::File, Some(1)).kind_label(), "TXT File");
        assert_eq!(entry("Safari.app", RenameKind::Package, None).kind_label(), "Application");
        assert_eq!(entry("Makefile", RenameKind::File, Some(1)).kind_label(), "File");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn custom_icon_flag() {
        let dir = std::env::temp_dir().join(format!("macsplorer-ci-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x");
        fs::write(&file, "x").unwrap();
        assert!(!has_custom_icon(&file));
        let mut info = [0u8; 32];
        info[8..10].copy_from_slice(&0x0400u16.to_be_bytes());
        let c = std::ffi::CString::new(file.to_str().unwrap()).unwrap();
        // SAFETY: valid path, name and buffer.
        let rc = unsafe {
            libc::setxattr(c.as_ptr(), c"com.apple.FinderInfo".as_ptr(), info.as_ptr().cast(), info.len(), 0, 0)
        };
        assert_eq!(rc, 0);
        assert!(has_custom_icon(&file));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn packages_detected() {
        assert!(is_package_name("Safari.app"));
        assert!(is_package_name("Thing.BUNDLE"));
        assert!(!is_package_name("My.Folder"));
    }

    #[test]
    fn size_format() {
        assert_eq!(format_size(0), "0 KB");
        assert_eq!(format_size(1), "1 KB");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1025), "2 KB");
        assert_eq!(format_size(5_000_000_000), "4,882,813 KB");
    }

    #[test]
    fn free_space_of_temp_dir() {
        assert!(free_space(&std::env::temp_dir()).is_some_and(|b| b > 0));
        assert_eq!(free_space(Path::new("/definitely/not/here")), None);
    }

    #[test]
    fn byte_size_format() {
        assert_eq!(format_byte_size(0), "0 bytes");
        assert_eq!(format_byte_size(1023), "1023 bytes");
        assert_eq!(format_byte_size(1024), "1.00 KB");
        assert_eq!(format_byte_size(12_700), "12.4 KB");
        assert_eq!(format_byte_size(1_023_999), "999 KB");
        assert_eq!(format_byte_size(1_258_291), "1.19 MB");
        assert_eq!(format_byte_size(5_000_000_000), "4.65 GB");
    }
}
