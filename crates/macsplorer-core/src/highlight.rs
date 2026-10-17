//! Colored names for special files, after the Ubuntu terminal's `ls`
//! (`dircolors` defaults): executables green, links cyan, archives red.
//!
//! Not an Explorer behavior. The closest Windows 2000 feature is Folder
//! Options' "Display compressed files and folders with alternate color",
//! which drew NTFS-compressed items in blue.

use crate::listing::{Entry, Link};
use crate::rename::RenameKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Highlight {
    /// A regular file with an execute bit (`ex`, green).
    Executable,
    /// A symlink or Finder alias (`ln`, cyan).
    Link,
    /// A symlink whose target is gone (`or`, red).
    BrokenLink,
    /// A file with an archive extension (red).
    Archive,
}

/// The archive and compressed-file extensions of Ubuntu's `dircolors`, plus
/// macOS disk images, installer packages and Xcode archives.
const ARCHIVE_EXTENSIONS: &[&str] = &[
    "tar", "tgz", "arc", "arj", "taz", "lha", "lz4", "lzh", "lzma", "tlz", "txz", "tzo", "t7z", "zip", "z", "dz", "gz",
    "lrz", "lz", "lzo", "xz", "zst", "tzst", "bz2", "bz", "tbz", "tbz2", "tz", "deb", "rpm", "jar", "war", "ear", "sar",
    "rar", "alz", "ace", "zoo", "cpio", "7z", "rz", "cab", "wim", "swm", "dwm", "esd", "dmg", "xip", "pkg",
];

/// How `entry`'s name is colored, if at all. A link wins over what it points
/// to, and an archive over an execute bit (zips from FAT volumes often have
/// one).
pub fn classify(entry: &Entry) -> Option<Highlight> {
    match entry.link {
        Some(Link::Broken) => return Some(Highlight::BrokenLink),
        Some(Link::Symlink | Link::Alias) => return Some(Highlight::Link),
        None => {}
    }
    if entry.kind != RenameKind::File {
        return None;
    }
    if entry.extension().is_some_and(|e| ARCHIVE_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e))) {
        return Some(Highlight::Archive);
    }
    let regular = entry.mode & 0o170000 == 0o100000;
    (regular && entry.mode & 0o111 != 0).then_some(Highlight::Executable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, kind: RenameKind, mode: u32, link: Option<Link>) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from(name),
            kind,
            size: None,
            modified: None,
            created: None,
            accessed: None,
            mode,
            owner: String::new(),
            group: String::new(),
            comment: None,
            hidden: false,
            custom_icon: false,
            folder_style: None,
            branch: None,
            link,
        }
    }

    #[test]
    fn plain_items_stay_uncolored() {
        assert_eq!(classify(&entry("notes.txt", RenameKind::File, 0o100644, None)), None);
        assert_eq!(classify(&entry("bin", RenameKind::Folder, 0o040755, None)), None);
        assert_eq!(classify(&entry("Safari.app", RenameKind::Package, 0o040755, None)), None);
    }

    #[test]
    fn executables() {
        assert_eq!(classify(&entry("build.sh", RenameKind::File, 0o100755, None)), Some(Highlight::Executable));
        assert_eq!(classify(&entry("tool", RenameKind::File, 0o100700, None)), Some(Highlight::Executable));
        assert_eq!(classify(&entry("others", RenameKind::File, 0o100601, None)), Some(Highlight::Executable));
    }

    #[test]
    fn archives_by_extension() {
        for name in ["a.zip", "a.tar.gz", "B.TGZ", "x.7z", "Installer.dmg", "Setup.pkg", "a.zst"] {
            assert_eq!(classify(&entry(name, RenameKind::File, 0o100644, None)), Some(Highlight::Archive), "{name}");
        }
        // An execute bit doesn't make an archive green.
        assert_eq!(classify(&entry("a.zip", RenameKind::File, 0o100755, None)), Some(Highlight::Archive));
        // A folder or bundle package named like an archive isn't one.
        assert_eq!(classify(&entry("old.zip", RenameKind::Folder, 0o040755, None)), None);
        assert_eq!(classify(&entry("Thing.pkg", RenameKind::Package, 0o040755, None)), None);
    }

    #[test]
    fn links_win() {
        assert_eq!(classify(&entry("Calc.app", RenameKind::Package, 0o120755, Some(Link::Symlink))), Some(Highlight::Link));
        assert_eq!(classify(&entry("a.zip", RenameKind::File, 0o120755, Some(Link::Symlink))), Some(Highlight::Link));
        assert_eq!(classify(&entry("Docs alias", RenameKind::File, 0o100644, Some(Link::Alias))), Some(Highlight::Link));
        assert_eq!(classify(&entry("gone", RenameKind::File, 0o120755, Some(Link::Broken))), Some(Highlight::BrokenLink));
    }
}
