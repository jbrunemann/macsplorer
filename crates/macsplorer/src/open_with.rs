//! The applications macOS offers for opening a file, for the "Open With"
//! submenu. Windows builds that submenu in shell32's
//! `COpenWithMenu::QueryContextMenu` / `InitMenuPopup` (not traced yet).

use std::path::{Path, PathBuf};

use macsplorer_core::preferences::Preferences;

/// Whether this platform can list apps for a file. There is no Linux
/// implementation yet (it would read `.desktop` files and `mimeapps.list`),
/// so the menu is hidden there.
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// Apps that can open `path`: the one the user chose for this type of file,
/// or else the system's default, first, then the rest by name.
/// Copies of the same app in several places show up once.
#[cfg(target_os = "macos")]
pub fn apps_for(path: &Path) -> Vec<PathBuf> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSURL;

    let Some(url) = NSURL::from_file_path(path) else {
        return Vec::new();
    };
    let ws = NSWorkspace::sharedWorkspace();
    let default = chosen_app(path).or_else(|| ws.URLForApplicationToOpenURL(&url).and_then(|u| u.to_file_path()));
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

/// The file's type as the system knows it (a UTI such as `public.json`),
/// the key the "always use this program" choices are stored under. Dynamic
/// types for unknown extensions (`dyn.…`) work too: they are stable per
/// extension.
#[cfg(target_os = "macos")]
pub fn type_key(path: &Path) -> Option<String> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2_foundation::{NSString, NSURL, NSURLContentTypeKey};

    let url = NSURL::from_file_path(path)?;
    let mut value = None;
    // SAFETY: the content type resource is a UTType.
    unsafe { url.getResourceValue_forKey_error(&mut value, NSURLContentTypeKey) }.ok()?;
    let content_type = value?;
    // SAFETY: UTType has an `identifier` property, an NSString.
    let identifier: Retained<NSString> = unsafe { msg_send![&*content_type, identifier] };
    Some(identifier.to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn type_key(_path: &Path) -> Option<String> {
    None
}

/// The app the user chose for this kind of file, if it is still installed.
/// Reads the settings file, which is small, so a choice made in another
/// window counts at once.
pub fn chosen_app(path: &Path) -> Option<PathBuf> {
    if !SUPPORTED {
        return None;
    }
    let key = type_key(path)?;
    let app = Preferences::load().associations.app_for(&key)?.to_path_buf();
    app.exists().then_some(app)
}

/// Stores `app` as the one that opens files of `path`'s type, or forgets the
/// choice when `app` is `None`.
pub fn remember(path: &Path, app: Option<&Path>) {
    let Some(key) = type_key(path) else { return };
    Preferences::update(|prefs| match app {
        Some(app) => prefs.associations.set(&key, app),
        None => prefs.associations.clear(&key),
    });
}

/// The name an app shows in the menu: its bundle name without `.app`.
pub fn app_name(app: &Path) -> String {
    app.file_stem().map_or_else(|| app.display().to_string(), |s| s.to_string_lossy().into_owned())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn files_are_keyed_by_their_type() {
        let dir = std::env::temp_dir().join(format!("macsplorer-type-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (jpg, jpeg) = (dir.join("a.jpg"), dir.join("b.jpeg"));
        std::fs::write(&jpg, "x").unwrap();
        std::fs::write(&jpeg, "x").unwrap();
        let (a, b) = (type_key(&jpg), type_key(&jpeg));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(a.as_deref(), Some("public.jpeg"));
        assert_eq!(a, b);
        assert_eq!(type_key(Path::new("/no/such/file.jpg")), None);
    }

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
