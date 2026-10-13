//! Sidebar locations: the macOS stand-ins for Explorer's known folders.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
}

/// User folders that exist on this machine, in Explorer's Quick access order.
pub fn favorites() -> Vec<Place> {
    let known = [
        ("Desktop", dirs::desktop_dir()),
        ("Downloads", dirs::download_dir()),
        ("Documents", dirs::document_dir()),
        ("Pictures", dirs::picture_dir()),
        ("Music", dirs::audio_dir()),
        ("Movies", dirs::video_dir()),
        ("Home", dirs::home_dir()),
        ("Applications", Some(PathBuf::from("/Applications"))),
    ];
    known
        .into_iter()
        .filter_map(|(label, path)| path.filter(|p| p.is_dir()).map(|path| Place { label: label.into(), path }))
        .collect()
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
}
