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
            // The boot volume also appears here as a symlink to "/".
            .filter(|e| fs::read_link(e.path()).map_or(true, |t| t != Path::new("/")))
            .map(|e| Place { label: e.file_name().to_string_lossy().into_owned(), path: e.path() })
            .collect();
        vols.sort_by(|a, b| crate::natural::natural_cmp(&a.label, &b.label));
        out.extend(vols);
    }
    out
}
