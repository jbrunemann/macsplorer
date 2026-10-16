//! Directory listing and sorting for the details view.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::os::unix::fs::MetadataExt;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::columns::Column;
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
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    /// `st_mode`: file type and permission bits. A symlink's own.
    pub mode: u32,
    pub owner: String,
    pub group: String,
    /// Finder's comment, if any.
    pub comment: Option<String>,
    pub hidden: bool,
    /// Finder shows its own icon for this item (`kHasCustomIcon`).
    pub custom_icon: bool,
    /// For a folder that is the top of a Git working tree (see
    /// [`is_git_repo`]): its branch, as `git::branch_label` writes it.
    pub branch: Option<String>,
    /// The item is a symlink or Finder alias.
    pub link: Option<Link>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Symlink,
    /// A symlink whose target doesn't exist.
    Broken,
    /// A Finder alias file (`kIsAlias`).
    Alias,
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

    /// The Attributes column: the permissions as `ls -l` writes them.
    pub fn attributes(&self) -> String {
        format_mode(self.mode)
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

/// Whether a file name makes the item hidden (a leading dot).
pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// Reads one directory. Entries that vanish or can't be read while listing
/// are skipped rather than failing the whole listing.
pub fn list_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    list_dir_with(dir, show_hidden, false)
}

/// [`list_dir`], and with `branches` the [`Entry::branch`] of every folder
/// that is a Git working tree. That costs a look for `.git` in each folder,
/// so only ask for it while the Branch column is shown.
pub fn list_dir_with(dir: &Path, show_hidden: bool, branches: bool) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if is_hidden_name(&name) && !show_hidden {
            continue;
        }
        let Ok(own) = item.metadata() else { continue };
        out.push(read_entry(item.path(), name, &own, branches));
    }
    Ok(out)
}

/// The listing's entry for one item, or `None` when it can't be read.
pub fn entry_at(path: &Path) -> Option<Entry> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let own = fs::symlink_metadata(path).ok()?;
    Some(read_entry(path.to_path_buf(), name, &own, false))
}

/// An entry from the item's own metadata (`own`, not following a link).
fn read_entry(path: PathBuf, name: String, own: &fs::Metadata, branches: bool) -> Entry {
    // Follow symlinks so a link to a folder behaves like a folder.
    let target = fs::metadata(&path);
    let broken = target.is_err();
    let meta = target.unwrap_or_else(|_| own.clone());
    // The Attributes column describes the link itself, like `ls -l`.
    let symlink = own.file_type().is_symlink();
    let kind = if !meta.is_dir() {
        RenameKind::File
    } else if is_package_name(&name) {
        RenameKind::Package
    } else {
        RenameKind::Folder
    };
    let flags = finder_flags(&path);
    let link = match (symlink, broken) {
        (true, true) => Some(Link::Broken),
        (true, false) => Some(Link::Symlink),
        // An alias is always a file. Copy tools sometimes leave the flag
        // on a real folder, which must still open as one.
        _ => (!meta.is_dir() && flags & K_IS_ALIAS != 0).then_some(Link::Alias),
    };
    let own = if symlink { own } else { &meta };
    Entry {
        size: (kind == RenameKind::File).then_some(meta.len()),
        modified: meta.modified().ok(),
        created: meta.created().ok(),
        accessed: meta.accessed().ok(),
        mode: own.mode(),
        owner: user_name(own.uid()),
        group: group_name(own.gid()),
        comment: comment(&path),
        custom_icon: flags & K_HAS_CUSTOM_ICON != 0,
        branch: (branches && kind == RenameKind::Folder && is_git_repo(&path)).then(|| crate::git::branch_label(&path)).flatten(),
        link,
        hidden: is_hidden_name(&name),
        name,
        path,
        kind,
    }
}

/// `kHasCustomIcon` in the Finder flags: Finder shows the item's own icon.
const K_HAS_CUSTOM_ICON: u16 = 0x0400;
/// `kIsAlias` in the Finder flags: the file is a Finder alias.
const K_IS_ALIAS: u16 = 0x8000;

/// True if Finder has a custom icon for `path`.
pub fn has_custom_icon(path: &Path) -> bool {
    finder_flags(path) & K_HAS_CUSTOM_ICON != 0
}

/// True if `dir` is the top of a Git working tree: it holds a `.git` folder,
/// or a `.git` file as linked worktrees and submodules do. Folders inside a
/// working tree don't count. Not from Explorer: it has no version control.
pub fn is_git_repo(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(".git")).is_ok()
}

/// The Finder flags of `path`: bytes 8 and 9 of its `com.apple.FinderInfo`
/// attribute, 0 if there is none.
#[cfg(target_os = "macos")]
fn finder_flags(path: &Path) -> u16 {
    use std::os::unix::ffi::OsStrExt;
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return 0 };
    let mut info = [0u8; 32];
    // SAFETY: valid NUL-terminated path, name and a 32-byte buffer.
    let n = unsafe {
        libc::getxattr(cpath.as_ptr(), c"com.apple.FinderInfo".as_ptr(), info.as_mut_ptr().cast(), info.len(), 0, 0)
    };
    if n >= 10 { u16::from_be_bytes([info[8], info[9]]) } else { 0 }
}

#[cfg(not(target_os = "macos"))]
fn finder_flags(_path: &Path) -> u16 {
    0
}

/// The 10-character permissions string of `ls -l`: the file type, then
/// read, write and execute for owner, group and others, with set-user-ID,
/// set-group-ID and sticky shown as `s`/`S` and `t`/`T`.
pub fn format_mode(mode: u32) -> String {
    let kind = match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '-',
    };
    let mut out = String::with_capacity(10);
    out.push(kind);
    for (shift, special, set_char) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let bits = mode >> shift;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(match (bits & 1 != 0, mode & special != 0) {
            (true, true) => set_char,
            (false, true) => set_char.to_ascii_uppercase(),
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    out
}

/// The account name for a user ID, or the number if there is none.
pub fn user_name(uid: u32) -> String {
    static CACHE: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
    cached(&CACHE, uid, || {
        // SAFETY: getpwuid returns null or a pointer to static storage,
        // read right away while the cache lock serializes callers.
        let pw = unsafe { libc::getpwuid(uid) };
        // SAFETY: a non-null passwd has a NUL-terminated pw_name.
        (!pw.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }.to_string_lossy().into_owned())
    })
}

/// The group name for a group ID, or the number if there is none.
pub fn group_name(gid: u32) -> String {
    static CACHE: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
    cached(&CACHE, gid, || {
        // SAFETY: as in `user_name`, for getgrgid.
        let gr = unsafe { libc::getgrgid(gid) };
        // SAFETY: a non-null group has a NUL-terminated gr_name.
        (!gr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr((*gr).gr_name) }.to_string_lossy().into_owned())
    })
}

fn cached(cache: &OnceLock<Mutex<HashMap<u32, String>>>, id: u32, look_up: impl FnOnce() -> Option<String>) -> String {
    let mut map = cache.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    map.entry(id).or_insert_with(|| look_up().unwrap_or_else(|| id.to_string())).clone()
}

/// Reads an extended attribute, `None` if it isn't there.
fn read_xattr(path: &Path, name: &std::ffi::CStr) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
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

/// Finder's comment: the `kMDItemFinderComment` attribute, a binary plist
/// holding one string. Finder keeps the same text in `.DS_Store`.
#[cfg(target_os = "macos")]
pub fn comment(path: &Path) -> Option<String> {
    let data = read_xattr(path, c"com.apple.metadata:kMDItemFinderComment")?;
    plist_string(&data).filter(|s| !s.is_empty())
}

/// The freedesktop.org comment attribute, which Dolphin and Nautilus edit.
#[cfg(target_os = "linux")]
pub fn comment(path: &Path) -> Option<String> {
    let data = read_xattr(path, c"user.xdg.comment")?;
    Some(String::from_utf8_lossy(&data).into_owned()).filter(|s| !s.is_empty())
}

/// The string in a binary plist whose top object is one string.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn plist_string(data: &[u8]) -> Option<String> {
    if data.len() < 8 + 32 || &data[..8] != b"bplist00" {
        return None;
    }
    let trailer = &data[data.len() - 32..];
    let offset_size = trailer[6] as usize;
    let top = u64::from_be_bytes(trailer[16..24].try_into().ok()?) as usize;
    let table = u64::from_be_bytes(trailer[24..32].try_into().ok()?) as usize;
    let entry = data.get(table + top * offset_size..table + (top + 1) * offset_size)?;
    let at = entry.iter().fold(0usize, |n, &b| n << 8 | b as usize);
    let marker = *data.get(at)?;
    let (mut len, mut start) = ((marker & 0x0f) as usize, at + 1);
    if len == 0x0f {
        // The length follows as an integer object: 0x1n, then 2^n bytes.
        let int = *data.get(start)?;
        let bytes = 1usize << (int & 0x0f);
        len = data.get(start + 1..start + 1 + bytes)?.iter().fold(0usize, |n, &b| n << 8 | b as usize);
        start += 1 + bytes;
    }
    match marker >> 4 {
        0x5 => data.get(start..start + len).map(|b| String::from_utf8_lossy(b).into_owned()),
        0x6 => {
            let units: Vec<u16> = data.get(start..start + 2 * len)?.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

pub fn is_package_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PACKAGE_EXTENSIONS.iter().any(|p| p.eq_ignore_ascii_case(e)))
}

#[derive(Clone, Copy, Debug)]
pub struct SortSpec {
    pub column: Column,
    pub ascending: bool,
    /// Keep folders above files regardless of column. Explorer's details
    /// view does this; the exact rule in `CFSFolder::CompareIDs` is not yet
    /// traced. `false` mixes them like Finder (an app setting, not from
    /// Explorer).
    pub folders_first: bool,
    /// `false` gives the `NoStrCmpLogical` plain ordering.
    pub natural: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self { column: Column::Name, ascending: true, folders_first: true, natural: true }
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
            Column::Name => names(&a.name, &b.name),
            Column::Size => a.size.cmp(&b.size),
            Column::Type => names(&a.kind_label(), &b.kind_label()),
            Column::Modified => a.modified.cmp(&b.modified),
            Column::Attributes => a.attributes().cmp(&b.attributes()),
            // Items without a comment sort first, as empty text.
            Column::Comment => names(a.comment.as_deref().unwrap_or(""), b.comment.as_deref().unwrap_or("")),
            Column::Created => a.created.cmp(&b.created),
            Column::Accessed => a.accessed.cmp(&b.accessed),
            Column::Owner => names(&a.owner, &b.owner),
            Column::Group => names(&a.group, &b.group),
            Column::Branch => names(a.branch.as_deref().unwrap_or(""), b.branch.as_deref().unwrap_or("")),
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
    // f_bavail is u32 on macOS and u64 on 64-bit Linux.
    #[allow(clippy::useless_conversion)]
    Some(u64::from(st.f_bavail) * st.f_frsize)
}

/// Formats a byte count the way Explorer's Size column does: whole KB,
/// rounded up, so a 1-byte file shows as "1 KB".
pub fn format_size(bytes: u64) -> String {
    format!("{} KB", group_digits(bytes.div_ceil(1024)))
}

/// The system's thousands and decimal separators, set once at startup by
/// [`set_number_format`]. Windows takes both from the user locale
/// (`AddCommas64` and `StrFormatByteSize` use `GetNumberFormat`).
struct NumberFormat {
    group: String,
    decimal: String,
}

static NUMBER_FORMAT: OnceLock<NumberFormat> = OnceLock::new();

/// Uses these separators for every number shown from now on. Only the first
/// call counts; without one, numbers use "," and ".".
pub fn set_number_format(group: &str, decimal: &str) {
    let _ = NUMBER_FORMAT.set(NumberFormat { group: group.to_owned(), decimal: decimal.to_owned() });
}

fn separators() -> (&'static str, &'static str) {
    NUMBER_FORMAT.get().map_or((",", "."), |f| (f.group.as_str(), f.decimal.as_str()))
}

/// A number with thousands separators, like shell32's `AddCommas64`: "1,624".
pub fn group_digits(n: u64) -> String {
    group_digits_with(n, separators().0)
}

fn group_digits_with(n: u64, separator: &str) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push_str(separator);
        }
        grouped.push(c);
    }
    grouped
}

/// Formats a byte count like Windows' `StrFormatByteSize`, used by the
/// status bar: "512 bytes", "1.00 KB", "12.3 KB", "123 KB", "1.20 MB".
/// Three significant digits, truncated rather than rounded.
pub fn format_byte_size(bytes: u64) -> String {
    format_byte_size_with(bytes, separators().1)
}

fn format_byte_size_with(bytes: u64, decimal: &str) -> String {
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
    let number = format!("{truncated:.prec$}", prec = decimals as usize).replace('.', decimal);
    format!("{number} {}", units[unit])
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
            created: None,
            accessed: None,
            mode: 0o100644,
            owner: String::new(),
            group: String::new(),
            comment: None,
            hidden: false,
            custom_icon: false,
            branch: None,
            link: None,
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
    fn mixed_sorting_interleaves_folders_and_files() {
        let mut v = vec![
            entry("b10.txt", RenameKind::File, Some(1)),
            entry("zeta", RenameKind::Folder, None),
            entry("b2.txt", RenameKind::File, Some(1)),
            entry("Alpha", RenameKind::Folder, None),
        ];
        sort_entries(&mut v, SortSpec { folders_first: false, ..Default::default() });
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "b2.txt", "b10.txt", "zeta"]);
    }

    #[test]
    fn descending_keeps_folders_on_top() {
        let mut v = vec![
            entry("a", RenameKind::Folder, None),
            entry("small", RenameKind::File, Some(1)),
            entry("big", RenameKind::File, Some(9)),
        ];
        sort_entries(&mut v, SortSpec { column: Column::Size, ascending: false, ..Default::default() });
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

    #[test]
    fn git_repo_is_the_top_of_a_working_tree() {
        let dir = std::env::temp_dir().join(format!("macsplorer-git-{}", std::process::id()));
        let plain = dir.join("plain");
        let repo = dir.join("repo");
        let worktree = dir.join("worktree");
        fs::create_dir_all(plain.join("sub")).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(worktree.join(".git"), "gitdir: ../repo/.git/worktrees/w\n").unwrap();
        assert!(!is_git_repo(&plain));
        assert!(is_git_repo(&repo));
        assert!(is_git_repo(&worktree));
        assert!(!is_git_repo(&repo.join("src")));
        let listed = list_dir_with(&dir, true, true).unwrap();
        let flag = |n: &str| listed.iter().find(|e| e.name == n).unwrap().branch.is_some();
        assert!(flag("repo") && flag("worktree") && !flag("plain"));
        // Not asked for, not looked up.
        assert!(list_dir(&dir, true).unwrap().iter().all(|e| e.branch.is_none()));
        fs::remove_dir_all(&dir).unwrap();
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

    #[cfg(target_os = "macos")]
    #[test]
    fn alias_flag_ignored_on_folders() {
        let dir = std::env::temp_dir().join(format!("macsplorer-alias-{}", std::process::id()));
        fs::create_dir_all(dir.join("folder")).unwrap();
        fs::write(dir.join("file"), "x").unwrap();
        // Type 'alis', creator 'MACS', kIsAlias: what Finder writes on an alias.
        let mut info = [0u8; 32];
        info[..8].copy_from_slice(b"alisMACS");
        info[8..10].copy_from_slice(&K_IS_ALIAS.to_be_bytes());
        for name in ["folder", "file"] {
            let c = std::ffi::CString::new(dir.join(name).to_str().unwrap()).unwrap();
            // SAFETY: valid path, name and buffer.
            let rc = unsafe {
                libc::setxattr(c.as_ptr(), c"com.apple.FinderInfo".as_ptr(), info.as_ptr().cast(), info.len(), 0, 0)
            };
            assert_eq!(rc, 0);
        }
        let entries = list_dir(&dir, false).unwrap();
        let link = |name: &str| entries.iter().find(|e| e.name == name).unwrap().link;
        assert_eq!(link("file"), Some(Link::Alias));
        assert_eq!(link("folder"), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mode_strings() {
        assert_eq!(format_mode(0o100644), "-rw-r--r--");
        assert_eq!(format_mode(0o040755), "drwxr-xr-x");
        assert_eq!(format_mode(0o120755), "lrwxr-xr-x");
        assert_eq!(format_mode(0o104755), "-rwsr-xr-x");
        assert_eq!(format_mode(0o102644), "-rw-r-Sr--");
        assert_eq!(format_mode(0o041777), "drwxrwxrwt");
        assert_eq!(format_mode(0o041776), "drwxrwxrwT");
    }

    #[test]
    fn listing_reads_permissions_and_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("macsplorer-perm-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.sh");
        fs::write(&file, "x").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o750)).unwrap();
        let entries = list_dir(&dir, false).unwrap();
        assert_eq!(entries[0].attributes(), "-rwxr-x---");
        // SAFETY: getuid can't fail.
        assert_eq!(entries[0].owner, user_name(unsafe { libc::getuid() }));
        assert!(!entries[0].owner.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listing_marks_links() {
        let dir = std::env::temp_dir().join(format!("macsplorer-links-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("real"), "x").unwrap();
        std::os::unix::fs::symlink("real", dir.join("good")).unwrap();
        std::os::unix::fs::symlink("missing", dir.join("gone")).unwrap();
        let entries = list_dir(&dir, false).unwrap();
        let link = |name: &str| entries.iter().find(|e| e.name == name).unwrap().link;
        assert_eq!(link("real"), None);
        assert_eq!(link("good"), Some(Link::Symlink));
        assert_eq!(link("gone"), Some(Link::Broken));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_ids_show_as_numbers() {
        assert_eq!(user_name(4_000_000_000), "4000000000");
    }

    #[test]
    fn plist_strings() {
        // Hand-built: bplist00, an 'A' string "hi", offset table, trailer.
        let mut ascii = b"bplist00\x52hi".to_vec();
        ascii.push(8);
        let mut trailer = [0u8; 32];
        trailer[6] = 1;
        trailer[7] = 1;
        trailer[15] = 1;
        trailer[31] = 11;
        ascii.extend_from_slice(&trailer);
        assert_eq!(plist_string(&ascii).as_deref(), Some("hi"));
        // A UTF-16 string with the long length form: 0x6F, 0x10 0x02, "äb".
        let mut utf16 = b"bplist00\x6f\x10\x02\x00\xe4\x00b".to_vec();
        utf16.push(8);
        trailer[31] = 15;
        utf16.extend_from_slice(&trailer);
        assert_eq!(plist_string(&utf16).as_deref(), Some("äb"));
        assert_eq!(plist_string(b"nope"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_comment() {
        let dir = std::env::temp_dir().join(format!("macsplorer-comment-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x");
        fs::write(&file, "x").unwrap();
        assert_eq!(comment(&file), None);
        let status = std::process::Command::new("xattr")
            .args(["-wx", "com.apple.metadata:kMDItemFinderComment", "62706C697374303052686908000000000000010100000000000000010000000000000000000000000000000B"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(comment(&file).as_deref(), Some("hi"));
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

    #[test]
    fn locale_separators() {
        assert_eq!(group_digits_with(221_664_650, "."), "221.664.650");
        assert_eq!(group_digits_with(1_624, "\u{a0}"), "1\u{a0}624");
        assert_eq!(group_digits_with(999, "."), "999");
        assert_eq!(format_byte_size_with(1_258_291, ","), "1,19 MB");
        assert_eq!(format_byte_size_with(1_023_999, ","), "999 KB");
    }
}
