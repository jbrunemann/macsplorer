//! The applications macOS offers for opening a file, for the "Open With"
//! submenu. Windows builds that submenu in shell32's
//! `COpenWithMenu::QueryContextMenu` / `InitMenuPopup` (not traced yet).

use std::path::{Path, PathBuf};

/// Apps that can open `path`: the default app first, then the rest by name.
/// Copies of the same app in several places show up once.
#[cfg(target_os = "macos")]
pub fn apps_for(path: &Path) -> Vec<PathBuf> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSURL;

    let Some(url) = NSURL::from_file_path(path) else {
        return Vec::new();
    };
    let ws = NSWorkspace::sharedWorkspace();
    let default = ws.URLForApplicationToOpenURL(&url).and_then(|u| u.to_file_path());
    let mut others: Vec<PathBuf> =
        ws.URLsForApplicationsToOpenURL(&url).iter().filter_map(|u| u.to_file_path()).collect();
    others.sort_by(|a, b| macsplorer_core::natural::natural_cmp(&app_name(a), &app_name(b)));

    let mut apps: Vec<PathBuf> = Vec::new();
    for app in default.into_iter().chain(others) {
        if !apps.iter().any(|a| app_name(a) == app_name(&app)) {
            apps.push(app);
        }
    }
    apps
}

#[cfg(not(target_os = "macos"))]
pub fn apps_for(_path: &Path) -> Vec<PathBuf> {
    Vec::new()
}

/// The name an app shows in the menu: its bundle name without `.app`.
pub fn app_name(app: &Path) -> String {
    app.file_stem().map_or_else(|| app.display().to_string(), |s| s.to_string_lossy().into_owned())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn text_files_open_with_textedit() {
        let file = std::env::temp_dir().join("macsplorer-open-with.txt");
        std::fs::write(&file, "hi").unwrap();
        let names: Vec<String> = apps_for(&file).iter().map(|a| app_name(a)).collect();
        std::fs::remove_file(&file).ok();
        assert!(names.iter().any(|n| n == "TextEdit"), "{names:?}");
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }
}
