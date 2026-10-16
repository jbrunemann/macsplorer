//! Which app opens a kind of file when you double-click it, chosen with
//! "Always use this program to open these files" in the Open With dialog.
//! Windows 2000 keeps this per extension in the registry (`HKCR\.ext`); not
//! traced from build 10240. macsplorer keys it by the file's type instead
//! (a UTI such as `public.json` on macOS), so `.jpg` and `.jpeg` share one
//! choice, and stores it in `settings.json` (see `preferences`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// A type key (what the platform calls the file's type) mapped to the app
/// the user chose for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Associations(BTreeMap<String, PathBuf>);

impl Associations {
    /// The app chosen for `type_key`, if any.
    pub fn app_for(&self, type_key: &str) -> Option<&Path> {
        self.0.get(type_key).map(PathBuf::as_path)
    }

    /// Makes `app` the choice for `type_key`, replacing the earlier one.
    pub fn set(&mut self, type_key: &str, app: &Path) {
        self.0.insert(type_key.to_owned(), app.to_path_buf());
    }

    /// Forgets the choice for `type_key`.
    pub fn clear(&mut self, type_key: &str) {
        self.0.remove(type_key);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn to_json(&self) -> Value {
        Value::Object(self.0.iter().map(|(k, app)| (k.clone(), Value::from(app.to_string_lossy().as_ref()))).collect())
    }

    /// Entries that aren't a non-empty string path are skipped, so one bad
    /// entry doesn't lose the others.
    pub fn from_json(value: &Value) -> Self {
        let Some(map) = value.as_object() else { return Self::default() };
        Self(read_entries(map))
    }
}

fn read_entries(map: &Map<String, Value>) -> BTreeMap<String, PathBuf> {
    map.iter()
        .filter_map(|(key, app)| {
            let app = app.as_str()?.trim();
            (!key.is_empty() && !app.is_empty()).then(|| (key.clone(), PathBuf::from(app)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_is_found_replaced_and_forgotten() {
        let mut a = Associations::default();
        assert_eq!(a.app_for("public.json"), None);
        a.set("public.json", Path::new("/Applications/Zed.app"));
        assert_eq!(a.app_for("public.json"), Some(Path::new("/Applications/Zed.app")));
        a.set("public.json", Path::new("/Applications/VSCodium.app"));
        assert_eq!(a.app_for("public.json"), Some(Path::new("/Applications/VSCodium.app")));
        a.clear("public.json");
        assert!(a.is_empty());
    }

    #[test]
    fn json_round_trips() {
        let mut a = Associations::default();
        a.set("public.json", Path::new("/Applications/Zed.app"));
        a.set("public.jpeg", Path::new("/Applications/Preview.app"));
        assert_eq!(Associations::from_json(&a.to_json()), a);
    }

    #[test]
    fn bad_entries_are_skipped() {
        let value = serde_json::json!({ "public.json": "/Applications/Zed.app", "public.png": 3, "public.gif": " ", "": "/x.app" });
        let a = Associations::from_json(&value);
        assert_eq!(a.app_for("public.json"), Some(Path::new("/Applications/Zed.app")));
        assert_eq!(a.to_json().as_object().map(Map::len), Some(1));
        assert!(Associations::from_json(&Value::Bool(true)).is_empty());
    }
}
