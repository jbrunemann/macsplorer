//! The applications macOS offers for opening a file, for the "Open With"
//! submenu. Windows builds that submenu in shell32's
//! `COpenWithMenu::QueryContextMenu` / `InitMenuPopup` (not traced yet).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use macsplorer_core::columns::Column;
use macsplorer_core::listing::SortSpec;
use macsplorer_core::preferences::Preferences;

/// Whether this platform can list apps for a file. There is no Linux
/// implementation yet (it would read `.desktop` files and `mimeapps.list`),
/// so the menu is hidden there.
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// The apps that can open a file, and whether the first is the one it opens with.
#[derive(Clone, Default)]
pub struct Apps {
    pub list: Vec<PathBuf>,
    /// The first app in `list` is the file's default: the app the user chose
    /// for its type, or else the system's.
    pub has_default: bool,
}

impl Apps {
    /// Whether `list[index]` is the app the file opens with.
    pub fn is_default(&self, index: usize) -> bool {
        self.has_default && index == 0
    }
}

/// Apps that can open `path`: the one the user chose for this type of file,
/// or else the system's default, first, then the rest by name.
/// Copies of the same app in several places show up once.
#[cfg(target_os = "macos")]
pub fn apps_for(path: &Path) -> Apps {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSURL;

    let Some(url) = NSURL::from_file_path(path) else {
        return Apps::default();
    };
    let ws = NSWorkspace::sharedWorkspace();
    let default = chosen_app(path).or_else(|| default_app(path));
    let mut others: Vec<PathBuf> =
        ws.URLsForApplicationsToOpenURL(&url).iter().filter_map(|u| u.to_file_path()).collect();
    others.sort_by(|a, b| macsplorer_core::natural::natural_cmp(&app_name(a), &app_name(b)));

    let has_default = default.is_some();
    let mut apps: Vec<PathBuf> = Vec::new();
    for app in default.into_iter().chain(others) {
        if !apps.iter().any(|a| app_name(a) == app_name(&app)) {
            apps.push(app);
        }
    }
    Apps { list: apps, has_default }
}

/// The app macOS opens `path` with when the user has not chosen one here.
#[cfg(target_os = "macos")]
pub fn default_app(path: &Path) -> Option<PathBuf> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSURL;

    let url = NSURL::from_file_path(path)?;
    NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&url).and_then(|u| u.to_file_path())
}

#[cfg(not(target_os = "macos"))]
pub fn default_app(_path: &Path) -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn apps_for(_path: &Path) -> Apps {
    Apps::default()
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

/// Reaview's bundle identifier. Reaview steps through the images of a folder
/// with ← / →, in the listing it is told with `--sort` and friends (see `open_args`).
#[cfg(target_os = "macos")]
const REAVIEW_ID: &str = "dev.reaview.ai";

/// Whether the app bundle at `app` is Reaview.
#[cfg(target_os = "macos")]
pub fn is_reaview(app: &Path) -> bool {
    use objc2_foundation::{NSBundle, NSURL};

    NSURL::from_directory_path(app)
        .and_then(|url| NSBundle::bundleWithURL(&url))
        .and_then(|bundle| bundle.bundleIdentifier())
        .is_some_and(|id| id.to_string() == REAVIEW_ID)
}

#[cfg(not(target_os = "macos"))]
pub fn is_reaview(_app: &Path) -> bool {
    false
}

/// The `open` arguments that open `path` with `app`. Reaview gets the listing
/// this window shows, so its ← / → match it (see `reaview_args`). It reads
/// them at launch only, so it is started as a new process each time.
pub fn open_args(app: &Path, path: &Path, sort: SortSpec, show_hidden: bool) -> Vec<OsString> {
    let reaview = is_reaview(app);
    let mut args: Vec<OsString> = Vec::new();
    if reaview {
        args.push("-n".into());
    }
    args.extend(["-a".into(), app.into(), path.into()]);
    if reaview {
        args.extend(reaview_args(sort, show_hidden).into_iter().map(OsString::from));
    }
    args
}

/// Reaview's options for the folder's listing, after `--args`; empty when
/// none apply. Name, Size and Modified have a counterpart there, ascending or
/// descending; the other columns do not, so Reaview keeps its Finder name
/// order. `--plain` is the `NoStrCmpLogical` name order (no natural numbers),
/// `--hidden` lists dot files like "Hidden files and folders" does.
fn reaview_args(sort: SortSpec, show_hidden: bool) -> Vec<&'static str> {
    let mut args = Vec::new();
    let key = match sort.column {
        Column::Name => Some("name"),
        Column::Size => Some("size"),
        Column::Modified => Some("date"),
        _ => None,
    };
    if let Some(key) = key {
        args.extend(["--sort", key, if sort.ascending { "--asc" } else { "--desc" }]);
    }
    if !sort.natural {
        args.push("--plain");
    }
    if show_hidden {
        args.push("--hidden");
    }
    if !args.is_empty() {
        args.insert(0, "--args");
    }
    args
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

    fn sort(column: Column, ascending: bool) -> SortSpec {
        SortSpec { column, ascending, ..SortSpec::default() }
    }

    /// An app bundle with just enough for `NSBundle` to read its identifier.
    fn app_bundle(name: &str, id: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("macsplorer-open-args-{}-{name}", std::process::id()));
        let app = dir.join(format!("{name}.app"));
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
                 <key>CFBundleIdentifier</key><string>{id}</string></dict></plist>"
            ),
        )
        .unwrap();
        (dir, app)
    }

    #[test]
    fn reaview_is_told_the_listing_in_a_new_process() {
        let (dir, app) = app_bundle("Reaview", REAVIEW_ID);
        let file = Path::new("/photos/a.jpg");
        let args = |sort, hidden| {
            open_args(&app, file, sort, hidden).iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>()
        };
        let a = app.to_string_lossy().into_owned();
        assert_eq!(
            args(sort(Column::Modified, false), false),
            ["-n", "-a", &a, "/photos/a.jpg", "--args", "--sort", "date", "--desc"]
        );
        assert_eq!(
            args(sort(Column::Size, true), false),
            ["-n", "-a", &a, "/photos/a.jpg", "--args", "--sort", "size", "--asc"]
        );
        assert_eq!(args(sort(Column::Name, true), false)[5..], ["--sort", "name", "--asc"]);
        // Plain name order and hidden files ride along.
        let plain = SortSpec { natural: false, ..sort(Column::Name, true) };
        assert_eq!(args(plain, true)[4..], ["--args", "--sort", "name", "--asc", "--plain", "--hidden"]);
        // A column Reaview can't sort by: still a new process, and nothing else to say by default.
        assert_eq!(args(sort(Column::Type, true), false), ["-n", "-a", &a, "/photos/a.jpg"]);
        assert_eq!(args(sort(Column::Type, true), true)[4..], ["--args", "--hidden"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn other_apps_get_only_the_file() {
        let (dir, app) = app_bundle("Other", "com.example.other");
        let args = open_args(&app, Path::new("/photos/a.jpg"), SortSpec { natural: false, ..sort(Column::Size, false) }, true);
        assert_eq!(args, [OsString::from("-a"), app.clone().into(), "/photos/a.jpg".into()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn text_files_open_with_textedit() {
        let file = std::env::temp_dir().join("macsplorer-open-with.txt");
        std::fs::write(&file, "hi").unwrap();
        let names: Vec<String> = apps_for(&file).list.iter().map(|a| app_name(a)).collect();
        std::fs::remove_file(&file).ok();
        assert!(names.iter().any(|n| n == "TextEdit"), "{names:?}");
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }
}
