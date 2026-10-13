//! Sidebar locations: the macOS stand-ins for Explorer's known folders.

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
}

impl KnownFolder {
    /// In Explorer's Quick access order.
    const ALL: [KnownFolder; 8] = [
        KnownFolder::Desktop,
        KnownFolder::Downloads,
        KnownFolder::Documents,
        KnownFolder::Pictures,
        KnownFolder::Music,
        KnownFolder::Movies,
        KnownFolder::Home,
        KnownFolder::Applications,
    ];

    pub fn label(self) -> &'static str {
        match self {
            KnownFolder::Desktop => "Desktop",
            KnownFolder::Downloads => "Downloads",
            KnownFolder::Documents => "Documents",
            KnownFolder::Pictures => "Pictures",
            KnownFolder::Music => "Music",
            KnownFolder::Movies => "Movies",
            KnownFolder::Home => "Home",
            KnownFolder::Applications => "Applications",
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
            KnownFolder::Applications => Some(PathBuf::from("/Applications")),
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
        }
    }
}

/// Which known folder `path` is, if any.
pub fn known_folder(path: &Path) -> Option<KnownFolder> {
    KnownFolder::ALL.into_iter().find(|k| k.path().is_some_and(|p| p == path))
}

/// User folders that exist on this machine, in Explorer's Quick access order.
pub fn favorites() -> Vec<Place> {
    KnownFolder::ALL
        .into_iter()
        .filter_map(|k| k.path().filter(|p| p.is_dir()).map(|path| Place { label: k.label().into(), path }))
        .collect()
}

/// The machine's name for the status bar's zone pane ("My Computer" in
/// Explorer): the host name without its domain, e.g. "pc192" for
/// "pc192.example.com". `None` if it can't be read.
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

/// Mounted volumes, Explorer's "This PC" drives.
pub fn volumes() -> Vec<Place> {
    let mut out = vec![Place { label: "Macintosh HD".into(), path: PathBuf::from("/") }];
    if let Ok(dir) = fs::read_dir("/Volumes") {
        let mut vols: Vec<Place> = dir
            .flatten()
            .filter(|e| is_browsable_mount(&e.path()))
            .map(|e| Place { label: e.file_name().to_string_lossy().into_owned(), path: e.path() })
            .collect();
        vols.sort_by(|a, b| crate::natural::natural_cmp(&a.label, &b.label));
        out.extend(vols);
    }
    out
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

#[cfg(not(target_os = "macos"))]
pub fn is_browsable_mount(path: &Path) -> bool {
    fs::read_link(path).is_err()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

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
        assert!(favorites().iter().all(|f| known_folder(&f.path).is_some()));
    }
}
