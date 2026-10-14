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

const FILE: &str = "settings.json";
/// The empty file older versions used to mean "dark mode is on".
const LEGACY_DARK_MARKER: &str = "dark-mode";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub dark: bool,
}

impl Preferences {
    /// The stored preferences, or the defaults when there are none or the
    /// file can't be read.
    pub fn load() -> Self {
        config_dir().map(|dir| Self::load_from(&dir)).unwrap_or_default()
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
        Self { dark: map.get("dark").and_then(Value::as_bool).unwrap_or(false) }
    }
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
        Preferences { dark: true }.save_to(&dir).unwrap();
        assert!(Preferences::load_from(&dir).dark);
        Preferences { dark: false }.save_to(&dir).unwrap();
        assert!(!Preferences::load_from(&dir).dark);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_keys_are_kept() {
        let dir = temp("unknown");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), r#"{"future": [1, 2], "dark": false}"#).unwrap();
        Preferences { dark: true }.save_to(&dir).unwrap();
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
        Preferences { dark: true }.save_to(&dir).unwrap();
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
        Preferences { dark: false }.save_to(&dir).unwrap();
        fs::write(dir.join(LEGACY_DARK_MARKER), "").unwrap();
        assert!(!Preferences::load_from(&dir).dark);
        assert!(!dir.join(LEGACY_DARK_MARKER).exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
