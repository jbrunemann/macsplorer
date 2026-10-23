//! The settings file that survives between runs: `settings.json` in the
//! config directory. Not from Explorer, which keeps these in the registry.
//!
//! Every window is its own process, so a change is written by reading the
//! file again, changing one value and replacing the file (`Preferences::update`),
//! and keys this version doesn't know are kept as they are.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::associations::Associations;
use crate::columns::{Column, ColumnSet};
use crate::folder_style::{format_hex, parse_hex};
use crate::text_size;

const FILE: &str = "settings.json";
/// Where "Check for Updates" asks by default: GitHub's latest release.
pub const DEFAULT_UPDATE_URL: &str = "https://api.github.com/repos/jbrunemann/macsplorer/releases/latest";
/// The empty file older versions used to mean "dark mode is on".
const LEGACY_DARK_MARKER: &str = "dark-mode";

/// The item view's mode. Windows 2000's View menu also had Small Icons and
/// Thumbnails.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    LargeIcons,
    List,
    #[default]
    Details,
}

impl ViewMode {
    fn key(self) -> &'static str {
        match self {
            ViewMode::LargeIcons => "large_icons",
            ViewMode::List => "list",
            ViewMode::Details => "details",
        }
    }

    fn from_key(key: &str) -> Option<ViewMode> {
        [ViewMode::LargeIcons, ViewMode::List, ViewMode::Details].into_iter().find(|v| v.key() == key)
    }
}

/// The column the item view is sorted by, and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortOrder {
    pub column: Column,
    pub ascending: bool,
}

/// The smallest window size that is stored; anything smaller is ignored.
pub const MIN_WINDOW: [f32; 2] = [520.0, 320.0];
/// Larger stored sizes are cut down to this.
const MAX_WINDOW: f32 = 16384.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub dark: bool,
    /// The details view's columns; `None` until one was stored.
    pub columns: Option<ColumnSet>,
    /// View > Text Size, one of `text_size::SIZES`.
    pub zoom: f32,
    /// The terminal "Open in Terminal" starts: an app name on macOS ("iTerm",
    /// "Ghostty"), a command on Linux. Only set by editing the file; `None`
    /// means the system's.
    pub terminal: Option<String>,
    /// Where "Check for Updates" asks for the newest release (the `update-check`
    /// crate says what it may answer). Always written, so there is something
    /// to edit if the default's host goes away; empty or missing reads as
    /// [`DEFAULT_UPDATE_URL`].
    pub update_url: String,
    /// Where git was found the last time, so the next launch doesn't have to
    /// search for it. Written by the app, and dropped when git is gone.
    pub git: Option<PathBuf>,
    /// The item view's mode the last time it was changed.
    pub view: ViewMode,
    /// The sort the last time it was changed; `None` until then.
    pub sort: Option<SortOrder>,
    /// The window's inner size (in macOS points, before Text Size zoom)
    /// when it was last resized; `None` until then.
    pub window: Option<[f32; 2]>,
    /// The apps chosen with "Always use this program", by file type.
    pub associations: Associations,
    /// The Color dialog's 16 custom color boxes, empty ones `None`.
    pub custom_colors: [Option<[u8; 3]>; 16],
    /// Extract All's "Show extracted files when complete", on until turned
    /// off (Windows keeps it as `ExtractionWizard\ShowFiles`).
    pub extract_show: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self { dark: false, columns: None, zoom: text_size::NORMAL, terminal: None, update_url: DEFAULT_UPDATE_URL.to_owned(), git: None, view: ViewMode::default(), sort: None, window: None, associations: Associations::default(), custom_colors: [None; 16], extract_show: true }
    }
}

impl Preferences {
    /// The stored preferences, or the defaults when there are none or the
    /// file can't be read.
    pub fn load() -> Self {
        config_dir().map(|dir| Self::load_from(&dir)).unwrap_or_default()
    }

    /// The settings file, created empty (`{}`) when there is none yet so
    /// there is something to open in an editor.
    pub fn file_path() -> io::Result<PathBuf> {
        let dir = config_dir().ok_or_else(|| io::Error::other("no config folder"))?;
        let file = dir.join(FILE);
        if !file.exists() {
            fs::create_dir_all(&dir)?;
            fs::write(&file, "{}\n")?;
        }
        Ok(file)
    }

    /// Reads the file again, lets `change` edit it and writes it back.
    pub fn update(change: impl FnOnce(&mut Preferences)) {
        let Some(dir) = config_dir() else { return };
        let mut prefs = Self::load_from(&dir);
        change(&mut prefs);
        let _ = prefs.save_to(&dir);
    }

    fn load_from(dir: &Path) -> Self {
        let file = dir.join(FILE);
        let marker = dir.join(LEGACY_DARK_MARKER);
        let mut prefs = match fs::read_to_string(&file) {
            Ok(text) => Self::from_map(&read_map(&text)),
            Err(_) => Self::default(),
        };
        // Move the old marker file into the settings file.
        if marker.exists() {
            if !file.exists() {
                prefs.dark = true;
                let _ = prefs.save_to(dir);
            }
            let _ = fs::remove_file(&marker);
        }
        prefs
    }

    fn save_to(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        let file = dir.join(FILE);
        let mut map = fs::read_to_string(&file).map(|t| read_map(&t)).unwrap_or_default();
        map.insert("dark".into(), Value::Bool(self.dark));
        map.insert("zoom".into(), Value::from(self.zoom));
        if let Some(terminal) = &self.terminal {
            map.insert("terminal".into(), Value::from(terminal.as_str()));
        }
        map.insert("update_url".into(), Value::from(self.update_url.as_str()));
        match &self.git {
            Some(git) => map.insert("git".into(), Value::from(git.to_string_lossy().as_ref())),
            None => map.remove("git"),
        };
        if let Some(columns) = &self.columns {
            map.insert("columns".into(), columns.to_json());
        }
        map.insert("view".into(), Value::from(self.view.key()));
        if let Some(sort) = self.sort {
            map.insert("sort".into(), serde_json::json!({ "column": sort.column.key(), "ascending": sort.ascending }));
        }
        if let Some([w, h]) = self.window {
            map.insert("window".into(), serde_json::json!({ "width": w.round(), "height": h.round() }));
        }
        if self.associations.is_empty() {
            map.remove("associations");
        } else {
            map.insert("associations".into(), self.associations.to_json());
        }
        if self.custom_colors.iter().all(Option::is_none) {
            map.remove("custom_colors");
        } else {
            let boxes = self.custom_colors.iter().map(|c| c.map_or(Value::Null, |rgb| Value::from(format_hex(rgb)))).collect();
            map.insert("custom_colors".into(), Value::Array(boxes));
        }
        map.insert("extract_show".into(), Value::Bool(self.extract_show));
        let text = serde_json::to_string_pretty(&map).map_err(io::Error::other)?;
        // Replace the file in one step, so another window reading it never
        // sees half of it.
        let temp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
        fs::write(&temp, text + "\n")?;
        fs::rename(&temp, &file).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    fn from_map(map: &Map<String, Value>) -> Self {
        Self {
            dark: map.get("dark").and_then(Value::as_bool).unwrap_or(false),
            columns: map.get("columns").and_then(ColumnSet::from_json),
            zoom: map.get("zoom").and_then(Value::as_f64).map_or(text_size::NORMAL, |z| text_size::snap(z as f32)),
            terminal: map.get("terminal").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(String::from),
            update_url: map.get("update_url").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()).unwrap_or(DEFAULT_UPDATE_URL).to_owned(),
            git: map.get("git").and_then(Value::as_str).map(str::trim).filter(|g| !g.is_empty()).map(PathBuf::from),
            view: map.get("view").and_then(Value::as_str).and_then(ViewMode::from_key).unwrap_or_default(),
            sort: map.get("sort").and_then(read_sort),
            window: map.get("window").and_then(read_window),
            associations: map.get("associations").map(Associations::from_json).unwrap_or_default(),
            custom_colors: read_custom_colors(map.get("custom_colors")),
            extract_show: map.get("extract_show").and_then(Value::as_bool).unwrap_or(true),
        }
    }
}

/// Up to 16 `"#rrggbb"` strings or nulls; anything else is an empty box.
fn read_custom_colors(value: Option<&Value>) -> [Option<[u8; 3]>; 16] {
    let mut boxes = [None; 16];
    for (slot, v) in boxes.iter_mut().zip(value.and_then(Value::as_array).into_iter().flatten()) {
        *slot = v.as_str().and_then(parse_hex);
    }
    boxes
}

fn read_sort(value: &Value) -> Option<SortOrder> {
    let column = value.get("column")?.as_str().and_then(Column::from_key)?;
    Some(SortOrder { column, ascending: value.get("ascending").and_then(Value::as_bool).unwrap_or(true) })
}

/// A stored size below [`MIN_WINDOW`] or not a number is ignored.
fn read_window(value: &Value) -> Option<[f32; 2]> {
    let side = |key: &str, min: f32| value.get(key)?.as_f64().map(|v| v as f32).filter(|v| v.is_finite() && *v >= min).map(|v| v.min(MAX_WINDOW));
    Some([side("width", MIN_WINDOW[0])?, side("height", MIN_WINDOW[1])?])
}

/// A damaged or unexpected file reads as empty.
fn read_map(text: &str) -> Map<String, Value> {
    match serde_json::from_str(text) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("macsplorer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("macsplorer-prefs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn nothing_stored_gives_defaults() {
        assert_eq!(Preferences::load_from(&temp("none")), Preferences::default());
    }

    #[test]
    fn saved_values_come_back() {
        let dir = temp("roundtrip");
        Preferences { dark: true, ..Default::default() }.save_to(&dir).unwrap();
        assert!(Preferences::load_from(&dir).dark);
        Preferences { dark: false, ..Default::default() }.save_to(&dir).unwrap();
        assert!(!Preferences::load_from(&dir).dark);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn custom_colors_come_back() {
        let dir = temp("custom-colors");
        let mut custom_colors = [None; 16];
        custom_colors[0] = Some([0, 200, 160]);
        custom_colors[3] = Some([255, 255, 255]);
        Preferences { custom_colors, ..Default::default() }.save_to(&dir).unwrap();
        assert!(fs::read_to_string(dir.join(FILE)).unwrap().contains("#00c8a0"));
        assert_eq!(Preferences::load_from(&dir).custom_colors, custom_colors);
        fs::write(dir.join(FILE), r##"{"custom_colors": ["#ff0000", 7, "nope"]}"##).unwrap();
        assert_eq!(Preferences::load_from(&dir).custom_colors[..3], [Some([255, 0, 0]), None, None]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn columns_are_stored_next_to_the_other_settings() {
        let dir = temp("columns");
        let mut columns = ColumnSet::default();
        columns.set_width(crate::columns::Column::Name, 222.0);
        Preferences { dark: true, columns: Some(columns.clone()), ..Default::default() }.save_to(&dir).unwrap();
        let back = Preferences::load_from(&dir);
        assert!(back.dark);
        assert_eq!(back.columns, Some(columns));
        // Saving without columns leaves the stored ones alone.
        Preferences { dark: false, columns: None, ..Default::default() }.save_to(&dir).unwrap();
        assert!(Preferences::load_from(&dir).columns.is_some());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn associations_are_stored_and_can_be_emptied() {
        let dir = temp("associations");
        let mut prefs = Preferences::default();
        prefs.associations.set("public.json", Path::new("/Applications/Zed.app"));
        prefs.save_to(&dir).unwrap();
        let mut back = Preferences::load_from(&dir);
        assert_eq!(back.associations.app_for("public.json"), Some(Path::new("/Applications/Zed.app")));
        back.associations.clear("public.json");
        back.save_to(&dir).unwrap();
        assert!(Preferences::load_from(&dir).associations.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_update_address_defaults_and_can_be_changed() {
        let dir = temp("update-url");
        assert_eq!(Preferences::load_from(&dir).update_url, DEFAULT_UPDATE_URL);
        // Saving writes the default out, where it can be edited.
        Preferences::default().save_to(&dir).unwrap();
        let text = fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains(DEFAULT_UPDATE_URL), "{text}");
        fs::write(dir.join(FILE), r#"{"update_url": " https://example.org/macsplorer.json "}"#).unwrap();
        let mut prefs = Preferences::load_from(&dir);
        assert_eq!(prefs.update_url, "https://example.org/macsplorer.json");
        // Another setting changing doesn't put the default back.
        prefs.dark = true;
        prefs.save_to(&dir).unwrap();
        assert_eq!(Preferences::load_from(&dir).update_url, "https://example.org/macsplorer.json");
        fs::write(dir.join(FILE), r#"{"update_url": ""}"#).unwrap();
        assert_eq!(Preferences::load_from(&dir).update_url, DEFAULT_UPDATE_URL);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_terminal_is_read_and_kept() {
        let dir = temp("terminal");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), r#"{"terminal": " Ghostty "}"#).unwrap();
        let prefs = Preferences::load_from(&dir);
        assert_eq!(prefs.terminal.as_deref(), Some("Ghostty"));
        // Another setting changing doesn't lose it.
        Preferences { dark: true, ..prefs }.save_to(&dir).unwrap();
        assert_eq!(Preferences::load_from(&dir).terminal.as_deref(), Some("Ghostty"));
        fs::write(dir.join(FILE), r#"{"terminal": ""}"#).unwrap();
        assert_eq!(Preferences::load_from(&dir).terminal, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_git_location_is_stored_and_dropped() {
        let dir = temp("git");
        fs::create_dir_all(&dir).unwrap();
        let mut prefs = Preferences::load_from(&dir);
        assert_eq!(prefs.git, None);
        prefs.git = Some("/opt/homebrew/bin/git".into());
        prefs.save_to(&dir).unwrap();
        assert_eq!(Preferences::load_from(&dir).git, Some(PathBuf::from("/opt/homebrew/bin/git")));
        // Another setting changing doesn't lose it.
        Preferences { dark: true, ..Preferences::load_from(&dir) }.save_to(&dir).unwrap();
        assert_eq!(Preferences::load_from(&dir).git, Some(PathBuf::from("/opt/homebrew/bin/git")));
        // Git being gone removes it from the file.
        Preferences { git: None, ..Preferences::load_from(&dir) }.save_to(&dir).unwrap();
        assert_eq!(Preferences::load_from(&dir).git, None);
        assert!(!fs::read_to_string(dir.join(FILE)).unwrap().contains("\"git\""));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_text_size_is_stored() {
        let dir = temp("zoom");
        assert_eq!(Preferences::load_from(&dir).zoom, 1.0);
        Preferences { zoom: 1.5, ..Default::default() }.save_to(&dir).unwrap();
        assert_eq!(Preferences::load_from(&dir).zoom, 1.5);
        // A value edited by hand snaps to a menu choice.
        fs::write(dir.join(FILE), r#"{"zoom": 1.9}"#).unwrap();
        assert_eq!(Preferences::load_from(&dir).zoom, 2.0);
        fs::write(dir.join(FILE), r#"{"zoom": "big"}"#).unwrap();
        assert_eq!(Preferences::load_from(&dir).zoom, 1.0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn view_sort_and_window_are_stored() {
        let dir = temp("view");
        let prefs = Preferences::load_from(&dir);
        assert_eq!((prefs.view, prefs.sort, prefs.window), (ViewMode::Details, None, None));
        let sort = SortOrder { column: Column::Modified, ascending: false };
        Preferences { view: ViewMode::List, sort: Some(sort), window: Some([900.4, 600.0]), ..Default::default() }.save_to(&dir).unwrap();
        let back = Preferences::load_from(&dir);
        assert_eq!((back.view, back.sort, back.window), (ViewMode::List, Some(sort), Some([900.0, 600.0])));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_view_sort_and_window_entries_are_ignored() {
        let dir = temp("badview");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), r#"{"view": "tiles", "sort": {"column": "colour"}, "window": {"width": 100, "height": 500}}"#).unwrap();
        let prefs = Preferences::load_from(&dir);
        assert_eq!((prefs.view, prefs.sort, prefs.window), (ViewMode::Details, None, None));
        fs::write(dir.join(FILE), r#"{"sort": {"column": "size"}, "window": {"width": 99999, "height": 700}}"#).unwrap();
        let prefs = Preferences::load_from(&dir);
        assert_eq!(prefs.sort, Some(SortOrder { column: Column::Size, ascending: true }));
        assert_eq!(prefs.window, Some([MAX_WINDOW, 700.0]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_columns_entry_reads_as_none() {
        let dir = temp("badcolumns");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), r#"{"dark": true, "columns": 5}"#).unwrap();
        let prefs = Preferences::load_from(&dir);
        assert!(prefs.dark);
        assert_eq!(prefs.columns, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_keys_are_kept() {
        let dir = temp("unknown");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), r#"{"future": [1, 2], "dark": false}"#).unwrap();
        Preferences { dark: true, ..Default::default() }.save_to(&dir).unwrap();
        let map = read_map(&fs::read_to_string(dir.join(FILE)).unwrap());
        assert_eq!(map["future"], serde_json::json!([1, 2]));
        assert_eq!(map["dark"], Value::Bool(true));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_file_reads_as_defaults() {
        let dir = temp("damaged");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), "{not json").unwrap();
        assert_eq!(Preferences::load_from(&dir), Preferences::default());
        // And the next save replaces it.
        Preferences { dark: true, ..Default::default() }.save_to(&dir).unwrap();
        assert!(Preferences::load_from(&dir).dark);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_old_marker_file_is_migrated() {
        let dir = temp("legacy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(LEGACY_DARK_MARKER), "").unwrap();
        assert!(Preferences::load_from(&dir).dark);
        assert!(!dir.join(LEGACY_DARK_MARKER).exists());
        // It now comes from the settings file.
        assert!(Preferences::load_from(&dir).dark);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stale_marker_does_not_override_the_settings_file() {
        let dir = temp("stale");
        Preferences { dark: false, ..Default::default() }.save_to(&dir).unwrap();
        fs::write(dir.join(LEGACY_DARK_MARKER), "").unwrap();
        assert!(!Preferences::load_from(&dir).dark);
        assert!(!dir.join(LEGACY_DARK_MARKER).exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
