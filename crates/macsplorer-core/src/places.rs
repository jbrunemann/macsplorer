//! Sidebar locations: the macOS (and Linux) stand-ins for Explorer's known folders.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
}

/// The user folders Finder gives their own sidebar glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KnownFolder {
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Movies,
    Home,
    Applications,
    /// Explorer's Recycle Bin.
    Trash,
}

impl KnownFolder {
    /// In Explorer's Quick access order, with the home folder first (not from
    /// Explorer).
    const ALL: [KnownFolder; 9] = [
        KnownFolder::Home,
        KnownFolder::Desktop,
        KnownFolder::Downloads,
        KnownFolder::Documents,
        KnownFolder::Pictures,
        KnownFolder::Music,
        KnownFolder::Movies,
        KnownFolder::Applications,
        KnownFolder::Trash,
    ];

    pub fn label(self) -> &'static str {
        match self {
            KnownFolder::Desktop => "Desktop",
            KnownFolder::Downloads => "Downloads",
            KnownFolder::Documents => "Documents",
            KnownFolder::Pictures => "Pictures",
            KnownFolder::Music => "Music",
            KnownFolder::Movies => if cfg!(target_os = "macos") { "Movies" } else { "Videos" },
            KnownFolder::Home => "Home",
            KnownFolder::Applications => "Applications",
            KnownFolder::Trash => "Trash",
        }
    }

    pub fn path(self) -> Option<PathBuf> {
        match self {
            KnownFolder::Desktop => dirs::desktop_dir(),
            KnownFolder::Downloads => dirs::download_dir(),
            KnownFolder::Documents => dirs::document_dir(),
            KnownFolder::Pictures => dirs::picture_dir(),
            KnownFolder::Music => dirs::audio_dir(),
            KnownFolder::Movies => dirs::video_dir(),
            KnownFolder::Home => dirs::home_dir(),
            // Linux has no single Applications folder.
            KnownFolder::Applications => cfg!(target_os = "macos").then(|| PathBuf::from("/Applications")),
            // macOS keeps the Trash in the home folder; Linux follows the freedesktop spec.
            KnownFolder::Trash if cfg!(target_os = "macos") => dirs::home_dir().map(|h| h.join(".Trash")),
            KnownFolder::Trash => dirs::data_dir().map(|d| d.join("Trash/files")),
        }
    }

    /// The SF Symbol Finder's sidebar shows for this folder.
    pub fn symbol(self) -> &'static str {
        match self {
            KnownFolder::Desktop => "menubar.dock.rectangle",
            KnownFolder::Downloads => "arrow.down.circle",
            KnownFolder::Documents => "doc",
            KnownFolder::Pictures => "photo",
            KnownFolder::Music => "music.note",
            KnownFolder::Movies => "film",
            KnownFolder::Home => "house",
            KnownFolder::Applications => "app.badge",
            KnownFolder::Trash => "trash",
        }
    }
}

/// Which known folder `path` is, if any.
pub fn known_folder(path: &Path) -> Option<KnownFolder> {
    KnownFolder::ALL.into_iter().find(|k| k.path().is_some_and(|p| p == path))
}

/// User folders that exist on this machine, in Explorer's Quick access order
/// after the home folder,
/// with the symbolic links to folders directly in the home folder (the
/// shortcuts people keep there) before the Trash. Not from Explorer.
pub fn favorites() -> Vec<Place> {
    let mut out: Vec<Place> = KnownFolder::ALL
        .into_iter()
        .filter_map(|k| k.path().filter(|p| p.is_dir()).map(|path| Place { label: k.label().into(), path }))
        .collect();
    if let Some(home) = dirs::home_dir() {
        let links = home_folder_links(&home, &out);
        let at = out.iter().position(|p| Some(&p.path) == KnownFolder::Trash.path().as_ref()).unwrap_or(out.len());
        out.splice(at..at, links);
    }
    out
}

/// The symbolic links directly in `home` that lead to folders, by name. Hidden
/// ones and those already in `known` are left out.
fn home_folder_links(home: &Path, known: &[Place]) -> Vec<Place> {
    let Ok(dir) = fs::read_dir(home) else { return Vec::new() };
    let mut links: Vec<Place> = dir
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_symlink()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        // `is_dir` follows the link, so a dangling one or one to a file drops out.
        .filter(|p| p.is_dir() && !known.iter().any(|k| &k.path == p))
        .map(|path| Place { label: path.file_name().unwrap_or_default().to_string_lossy().into_owned(), path })
        .collect();
    links.sort_by(|a, b| crate::natural::natural_cmp(&a.label, &b.label));
    links
}

/// The machine's name for the status bar's zone pane ("My Computer" in
/// Explorer): the host name without its domain, e.g. "pc192" for
/// "pc192.example.com". `None` if it can't be read.
#[cfg(unix)]
pub fn computer_name() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname NUL-terminates
    // within it unless the name was truncated, which the last byte guards.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) } != 0 {
        return None;
    }
    let len = buf.iter().position(|&b| b == 0)?;
    let name = String::from_utf8_lossy(&buf[..len]);
    let short = name.split('.').next().unwrap_or_default().trim();
    (!short.is_empty()).then(|| short.to_string())
}

/// The label of the root of the file system.
const ROOT_LABEL: &str = if cfg!(target_os = "macos") { "Macintosh HD" } else { "File System" };

/// Mounted volumes, Explorer's "This PC" drives.
pub fn volumes() -> Vec<Place> {
    #[allow(unused_mut)]
    let mut out = vec![Place { label: ROOT_LABEL.into(), path: PathBuf::from("/") }];
    #[cfg(unix)]
    {
        let mut vols: Vec<Place> = mounted_volumes()
            .into_iter()
            .map(|path| Place { label: path.file_name().unwrap_or_default().to_string_lossy().into_owned(), path })
            .collect();
        vols.sort_by(|a, b| crate::natural::natural_cmp(&a.label, &b.label));
        out.extend(vols);
    }
    out
}

/// Volumes other than the boot volume that Finder would show: the entries of
/// `/Volumes` that are real, browsable mount points.
#[cfg(target_os = "macos")]
fn mounted_volumes() -> Vec<PathBuf> {
    let Ok(dir) = fs::read_dir("/Volumes") else { return Vec::new() };
    dir.flatten().map(|e| e.path()).filter(|p| is_browsable_mount(p)).collect()
}

/// Removable media and manual mounts, the places desktop file managers list:
/// `/media/<user>/x`, `/run/media/<user>/x` (udisks) and `/mnt/x`.
#[cfg(target_os = "linux")]
fn mounted_volumes() -> Vec<PathBuf> {
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    parse_mounts(&mounts).into_iter().filter(|p| is_user_mount(p)).collect()
}

/// What sort of drive a volume is, which picks its icon in the Folders pane.
/// My Computer in Windows 2000 sorted its drives the same way, by the drive
/// type Windows reports (`GetDriveType`: fixed, removable, CD-ROM, remote).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DriveKind {
    /// A disk built into the computer.
    #[default]
    Fixed,
    /// A USB stick, an external disk, a memory card or a mounted disk image.
    Removable,
    /// A CD, DVD or similar (ISO 9660, UDF).
    Optical,
    /// A share on another computer.
    Network,
}

/// What the system says about a mounted volume.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
struct VolumeFacts {
    /// The file system's name as the kernel gives it (`apfs`, `cd9660`, `smbfs`).
    fs_type: String,
    /// On this computer, not a share.
    local: bool,
    /// Outside the computer's case: a USB drive, a card, a disk image.
    external: bool,
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn classify(facts: &VolumeFacts) -> DriveKind {
    const OPTICAL: [&str; 4] = ["cd9660", "udf", "cddafs", "iso9660"];
    const NETWORK: [&str; 10] = ["smbfs", "nfs", "nfs4", "afpfs", "webdav", "cifs", "smb3", "davfs", "9p", "fuse.sshfs"];
    if !facts.local || NETWORK.contains(&facts.fs_type.as_str()) {
        DriveKind::Network
    } else if OPTICAL.contains(&facts.fs_type.as_str()) {
        DriveKind::Optical
    } else if facts.external {
        DriveKind::Removable
    } else {
        DriveKind::Fixed
    }
}

/// The kind of drive mounted at `path`. The boot volume and anything the
/// system can't tell about count as a fixed disk.
pub fn drive_kind(path: &Path) -> DriveKind {
    #[cfg(target_os = "macos")]
    {
        mac_facts(path).map_or(DriveKind::Fixed, |f| classify(&f))
    }
    #[cfg(target_os = "linux")]
    {
        let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        parse_mount_table(&mounts)
            .into_iter()
            .rev()
            .find(|(mount, _)| mount == path)
            .map_or(DriveKind::Fixed, |(mount, fs_type)| {
                let external = mount.starts_with("/media") || mount.starts_with("/run/media");
                classify(&VolumeFacts { fs_type, local: true, external })
            })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        DriveKind::Fixed
    }
}

/// File system type from `statfs`, and Finder's own view of whether the
/// volume is local and internal (`NSURLVolumeIs…Key`).
#[cfg(target_os = "macos")]
fn mac_facts(path: &Path) -> Option<VolumeFacts> {
    use objc2_foundation::{NSArray, NSNumber, NSString, NSURLVolumeIsEjectableKey, NSURLVolumeIsInternalKey, NSURLVolumeIsLocalKey, NSURLVolumeIsRemovableKey, NSURL};
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;

    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statfs is plain old data, so all-zero is a valid value.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid NUL-terminated path and a statfs buffer to fill.
    if unsafe { libc::statfs(cpath.as_ptr(), &mut st) } != 0 {
        return None;
    }
    // SAFETY: the kernel NUL-terminates f_fstypename.
    let fs_type = unsafe { CStr::from_ptr(st.f_fstypename.as_ptr()) }.to_string_lossy().into_owned();

    let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
    // SAFETY: these are Foundation's own key constants.
    let keys = unsafe { NSArray::from_slice(&[NSURLVolumeIsLocalKey, NSURLVolumeIsInternalKey, NSURLVolumeIsRemovableKey, NSURLVolumeIsEjectableKey]) };
    let values = url.resourceValuesForKeys_error(&keys).ok()?;
    // SAFETY: as above.
    let flag = |key: &objc2_foundation::NSURLResourceKey| values.objectForKey(key).and_then(|v| v.downcast::<NSNumber>().ok()).map(|n| n.boolValue());
    let (local, internal, removable, ejectable) = unsafe { (flag(NSURLVolumeIsLocalKey), flag(NSURLVolumeIsInternalKey), flag(NSURLVolumeIsRemovableKey), flag(NSURLVolumeIsEjectableKey)) };
    Some(VolumeFacts {
        fs_type,
        local: local.unwrap_or(true),
        // A disk image reports itself as external too.
        external: removable.unwrap_or(false) || (!internal.unwrap_or(true) && ejectable.unwrap_or(true)),
    })
}

/// True if `path` is a mounted volume that can be ejected (disk images, USB
/// drives, network shares), never the boot volume.
#[cfg(unix)]
pub fn is_ejectable(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        path.parent() == Some(Path::new("/Volumes")) && is_browsable_mount(path)
    }
    #[cfg(target_os = "linux")]
    {
        is_user_mount(path) && mounted_volumes().iter().any(|v| v == path)
    }
}

/// Mount points from the text of `/proc/self/mounts`, in file order. Spaces
/// and other odd bytes in paths are written as octal escapes (`\040`).
#[cfg(any(target_os = "linux", test))]
fn parse_mounts(text: &str) -> Vec<PathBuf> {
    parse_mount_table(text).into_iter().map(|(path, _)| path).collect()
}

/// Mount points with their file system types, in file order.
#[cfg(any(target_os = "linux", test))]
fn parse_mount_table(text: &str) -> Vec<(PathBuf, String)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let path = PathBuf::from(unescape_mount(fields.nth(1)?));
            Some((path, fields.next()?.to_owned()))
        })
        .collect()
}

#[cfg(any(target_os = "linux", test))]
fn unescape_mount(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = bytes
            .get(i + 1..i + 4)
            .filter(|d| bytes[i] == b'\\' && d.iter().all(|c| (b'0'..=b'7').contains(c)))
            .and_then(|d| u8::try_from(d.iter().fold(0u32, |n, c| n * 8 + u32::from(c - b'0'))).ok());
        if let Some(byte) = escaped {
            out.push(byte);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// True if `path` lies where Linux desktops mount user-visible volumes:
/// one level under `/media/<user>` or `/run/media/<user>`, or directly under
/// `/mnt` or `/media`.
#[cfg(any(target_os = "linux", test))]
fn is_user_mount(path: &Path) -> bool {
    let depth_under = |base: &str| path.strip_prefix(base).map(|rest| rest.components().count()).ok();
    matches!(depth_under("/mnt"), Some(1))
        || matches!(depth_under("/media"), Some(1 | 2))
        || matches!(depth_under("/run/media"), Some(2))
}

/// True if `path` is itself a mount point that Finder would show. Skips
/// `nobrowse` mounts (e.g. a Recovery volume a macOS update left mounted),
/// leftover plain folders in /Volumes, and the boot volume's symlink to "/".
#[cfg(target_os = "macos")]
pub fn is_browsable_mount(path: &Path) -> bool {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: statfs is plain old data, so all-zero is a valid value.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid NUL-terminated path and a statfs buffer to fill.
    if unsafe { libc::statfs(cpath.as_ptr(), &mut st) } != 0 {
        return false;
    }
    // SAFETY: the kernel NUL-terminates f_mntonname.
    let mount_point = unsafe { CStr::from_ptr(st.f_mntonname.as_ptr()) };
    mount_point.to_bytes() == path.as_os_str().as_bytes() && st.f_flags & libc::MNT_DONTBROWSE as u32 == 0
}

#[cfg(test)]
mod mount_tests {
    use super::*;

    #[test]
    fn parses_mount_points() {
        let text = "/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /run/media/me/My\\040Disk vfat rw 0 0\nproc /proc proc rw 0 0\n";
        let mounts = parse_mounts(text);
        assert_eq!(mounts, [PathBuf::from("/"), PathBuf::from("/run/media/me/My Disk"), PathBuf::from("/proc")]);
    }

    #[test]
    fn mount_table_has_file_system_types() {
        let text = "/dev/sdb1 /run/media/me/My\\040Disk vfat rw 0 0\n/dev/sr0 /media/me/DISC iso9660 ro 0 0\n";
        let table = parse_mount_table(text);
        assert_eq!(table[0], (PathBuf::from("/run/media/me/My Disk"), "vfat".to_owned()));
        assert_eq!(table[1].1, "iso9660");
    }

    #[test]
    fn drive_kinds() {
        let facts = |fs: &str, local, external| VolumeFacts { fs_type: fs.into(), local, external };
        assert_eq!(classify(&facts("apfs", true, false)), DriveKind::Fixed);
        assert_eq!(classify(&facts("exfat", true, true)), DriveKind::Removable);
        assert_eq!(classify(&facts("cd9660", true, true)), DriveKind::Optical);
        assert_eq!(classify(&facts("udf", true, false)), DriveKind::Optical);
        assert_eq!(classify(&facts("smbfs", true, false)), DriveKind::Network);
        assert_eq!(classify(&facts("apfs", false, true)), DriveKind::Network);
        assert_eq!(classify(&facts("fuse.sshfs", true, false)), DriveKind::Network);
    }

    #[test]
    fn octal_escapes_stay_in_range() {
        assert_eq!(unescape_mount("a\\040b\\134c"), "a b\\c");
        // Not an escape: too short, not octal, or beyond one byte.
        assert_eq!(unescape_mount("a\\04"), "a\\04");
        assert_eq!(unescape_mount("a\\089"), "a\\089");
        assert_eq!(unescape_mount("a\\777"), "a\\777");
    }

    #[test]
    fn user_mounts_are_removable_media_and_mnt() {
        for yes in ["/media/me/USB", "/run/media/me/USB", "/mnt/data", "/media/cdrom"] {
            assert!(is_user_mount(Path::new(yes)), "{yes}");
        }
        for no in ["/", "/proc", "/boot/efi", "/run/media/me", "/mnt", "/mnt/a/b", "/home"] {
            assert!(!is_user_mount(Path::new(no)), "{no}");
        }
    }
}

#[cfg(all(test, unix))]
mod link_tests {
    use super::*;

    #[test]
    fn home_links_are_visible_symlinks_to_folders() {
        let home = std::env::temp_dir().join(format!("macsplorer-home-links-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("Projects/real")).unwrap();
        fs::create_dir_all(home.join("Documents")).unwrap();
        fs::write(home.join("note.txt"), "").unwrap();
        let link = |target: &str, name: &str| std::os::unix::fs::symlink(home.join(target), home.join(name)).unwrap();
        link("Projects/real", "work");
        link("Projects", "Archive");
        link("note.txt", "to-file");
        link("gone", "dangling");
        link("Projects/real", ".hidden-link");
        link("Documents", "docs-alias");
        let known = [Place { label: "Documents".into(), path: home.join("Documents") }];
        let labels: Vec<String> = home_folder_links(&home, &known).into_iter().map(|p| p.label).collect();
        fs::remove_dir_all(&home).ok();
        // docs-alias is a different path than Documents, so it stays.
        assert_eq!(labels, ["Archive", "docs-alias", "work"]);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn ejectable_volumes() {
        assert!(!is_ejectable(Path::new("/")));
        assert!(!is_ejectable(Path::new("/Volumes")));
        assert!(!is_ejectable(&std::env::temp_dir()));
        // Every other mounted volume under /Volumes can be ejected.
        for v in volumes().iter().skip(1) {
            assert!(is_ejectable(&v.path) || v.path.parent() != Some(Path::new("/Volumes")));
        }
    }

    #[test]
    fn the_boot_volume_is_a_fixed_disk() {
        assert_eq!(drive_kind(Path::new("/")), DriveKind::Fixed);
    }

    #[test]
    fn browsable_mounts() {
        assert!(is_browsable_mount(Path::new("/")));
        // The data volume is always mounted nobrowse.
        assert!(!is_browsable_mount(Path::new("/System/Volumes/Data")));
        // A plain folder isn't a mount point.
        assert!(!is_browsable_mount(&std::env::temp_dir()));
        // The boot volume's symlink resolves to "/", a different mount point.
        let link = std::env::temp_dir().join("macsplorer-root-link");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink("/", &link).unwrap();
        let shown = is_browsable_mount(&link);
        fs::remove_file(&link).ok();
        assert!(!shown);
    }

    #[test]
    fn volumes_skip_hidden_mounts() {
        for v in volumes().iter().skip(1) {
            assert!(is_browsable_mount(&v.path), "{}", v.path.display());
        }
    }

    #[test]
    fn computer_name_has_no_domain() {
        let name = computer_name().expect("host name");
        assert!(!name.is_empty() && !name.contains('.'));
    }

    #[test]
    fn known_folders() {
        assert_eq!(known_folder(Path::new("/Applications")), Some(KnownFolder::Applications));
        assert_eq!(known_folder(Path::new("/definitely/not/known")), None);
        if let Some(music) = dirs::audio_dir() {
            assert_eq!(known_folder(&music), Some(KnownFolder::Music));
        }
    }
}
