//! File-type icons from VS Code's Seti icon theme (VSCodium's default).
//!
//! The theme maps file names, extensions and VS Code language ids to a glyph
//! in the Seti font plus a color. Lookup follows VS Code's precedence:
//!
//! 1. exact file name (`readme.md`, `license`),
//! 2. extension, most specific suffix first (`spec.ts` before `ts`),
//! 3. language id, found from the file name or extension using the table
//!    VS Code's built-in language extensions contribute (`rs` → `rust`),
//! 4. the default file icon.
//!
//! Matching is case-insensitive. The light-theme associations override the
//! base ones, which suits macsplorer's white list background.
//!
//! Assets come from `scripts/update-seti.py`; see `assets/seti/`.

use std::collections::HashMap;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// A private-use code point in the Seti font.
    pub ch: char,
    pub rgb: [u8; 3],
}

#[derive(Debug, Default)]
struct Associations {
    file: Option<String>,
    names: HashMap<String, String>,
    extensions: HashMap<String, String>,
    languages: HashMap<String, String>,
}

impl Associations {
    fn parse(v: &Value) -> Self {
        let map = |key: &str| -> HashMap<String, String> {
            v.get(key)
                .and_then(Value::as_object)
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.to_lowercase(), v.as_str()?.to_owned()))).collect())
                .unwrap_or_default()
        };
        Self {
            file: v.get("file").and_then(Value::as_str).map(str::to_owned),
            names: map("fileNames"),
            extensions: map("fileExtensions"),
            languages: map("languageIds"),
        }
    }
}

#[derive(Debug)]
pub struct SetiTheme {
    definitions: HashMap<String, Glyph>,
    base: Associations,
    light: Associations,
    language_by_extension: HashMap<String, String>,
    language_by_name: HashMap<String, String>,
}

impl SetiTheme {
    /// Parses `vs-seti-icon-theme.json` and `languages.json`.
    pub fn parse(theme_json: &str, languages_json: &str) -> Result<Self, String> {
        let theme: Value = serde_json::from_str(theme_json).map_err(|e| format!("icon theme: {e}"))?;
        let langs: Value = serde_json::from_str(languages_json).map_err(|e| format!("languages: {e}"))?;

        let definitions = theme
            .get("iconDefinitions")
            .and_then(Value::as_object)
            .ok_or("icon theme has no iconDefinitions")?
            .iter()
            .filter_map(|(id, def)| {
                let ch = parse_font_character(def.get("fontCharacter")?.as_str()?)?;
                let rgb = parse_color(def.get("fontColor")?.as_str()?)?;
                Some((id.clone(), Glyph { ch, rgb }))
            })
            .collect();
        let table = |key: &str| -> HashMap<String, String> {
            langs
                .get(key)
                .and_then(Value::as_object)
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.to_lowercase(), v.as_str()?.to_owned()))).collect())
                .unwrap_or_default()
        };
        Ok(Self {
            definitions,
            base: Associations::parse(&theme),
            light: theme.get("light").map(Associations::parse).unwrap_or_default(),
            language_by_extension: table("extensions"),
            language_by_name: table("fileNames"),
        })
    }

    /// The glyph for a file name. `light` selects the light-theme colors.
    pub fn icon_for(&self, file_name: &str, light: bool) -> Option<Glyph> {
        self.resolve(file_name, light, true)
    }

    /// Like [`icon_for`](Self::icon_for), but `None` when only the generic
    /// default icon would apply, i.e. the type isn't recognized.
    pub fn specific_icon_for(&self, file_name: &str, light: bool) -> Option<Glyph> {
        self.resolve(file_name, light, false)
    }

    fn resolve(&self, file_name: &str, light: bool, fallback: bool) -> Option<Glyph> {
        let name = file_name.to_lowercase();
        let pick = |get: &dyn Fn(&Associations) -> Option<&String>| -> Option<&String> {
            light.then(|| get(&self.light)).flatten().or_else(|| get(&self.base))
        };

        let id = pick(&|a| a.names.get(&name))
            .or_else(|| suffixes(&name).find_map(|ext| pick(&|a| a.extensions.get(ext))))
            .or_else(|| {
                let lang = self
                    .language_by_name
                    .get(&name)
                    .or_else(|| suffixes(&name).find_map(|ext| self.language_by_extension.get(ext)))?;
                pick(&|a| a.languages.get(lang))
            })
            .or_else(|| if fallback { pick(&|a| a.file.as_ref()) } else { None })?;
        self.definitions.get(id).copied()
    }
}

/// Dotted suffixes of a name, longest first: `a.spec.ts` → `spec.ts`, `ts`.
/// A leading dot belongs to the name (`.gitignore` has no extension), as
/// in VS Code.
fn suffixes(name: &str) -> impl Iterator<Item = &str> {
    let body = name.strip_prefix('.').unwrap_or(name);
    let offset = name.len() - body.len();
    body.match_indices('.').map(move |(i, _)| &name[offset + i + 1..]).filter(|s| !s.is_empty())
}

/// `"\\E082"` (as written in the JSON) → U+E082.
fn parse_font_character(s: &str) -> Option<char> {
    let hex = s.trim_start_matches('\\');
    char::from_u32(u32::from_str_radix(hex, 16).ok()?)
}

/// `"#6d8086"` or `"#fff"` → RGB.
fn parse_color(s: &str) -> Option<[u8; 3]> {
    let hex = s.strip_prefix('#')?;
    let hex = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => hex.to_owned(),
        _ => return None,
    };
    let v = u32::from_str_radix(&hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> SetiTheme {
        SetiTheme::parse(
            include_str!("../../../assets/seti/vs-seti-icon-theme.json"),
            include_str!("../../../assets/seti/languages.json"),
        )
        .unwrap()
    }

    fn id_of(t: &SetiTheme, g: Glyph) -> Vec<&str> {
        let mut ids: Vec<_> = t.definitions.iter().filter(|(_, d)| **d == g).map(|(k, _)| k.as_str()).collect();
        ids.sort();
        ids
    }

    #[test]
    fn suffixes_longest_first() {
        assert_eq!(suffixes("a.spec.ts").collect::<Vec<_>>(), ["spec.ts", "ts"]);
        assert_eq!(suffixes(".gitignore").count(), 0);
        assert_eq!(suffixes(".eslintrc.json").collect::<Vec<_>>(), ["json"]);
        assert_eq!(suffixes("makefile").count(), 0);
    }

    #[test]
    fn parses_definitions() {
        assert_eq!(parse_font_character("\\E082"), Some('\u{E082}'));
        assert_eq!(parse_color("#6d8086"), Some([0x6d, 0x80, 0x86]));
        assert_eq!(parse_color("#fff"), Some([255, 255, 255]));
    }

    #[test]
    fn resolves_in_vs_code_order() {
        let t = theme();
        let dark = |n| id_of(&t, t.icon_for(n, false).unwrap());
        // Language id via VS Code's extension table.
        assert!(dark("main.rs").contains(&"_rust"));
        assert!(dark("app.py").contains(&"_python"));
        // Exact file name beats extension.
        assert!(dark("README.md").contains(&"_info"));
        assert!(dark("notes.md").contains(&"_markdown"));
        // Most specific extension wins.
        assert!(dark("x.spec.ts").contains(&"_typescript_1"));
        assert!(dark("x.ts").contains(&"_typescript"));
        // File name → language (Dockerfile has no extension).
        assert!(dark("Dockerfile").contains(&"_docker"));
        // Unknown types get the default icon, or nothing when asking for a
        // specific one.
        assert!(dark("thing.qwertyuiop").contains(&"_default"));
        assert_eq!(t.specific_icon_for("thing.qwertyuiop", true), None);
        assert!(t.specific_icon_for("main.rs", true).is_some());
    }

    #[test]
    fn light_theme_overrides_colors() {
        let t = theme();
        let dark = t.icon_for("main.rs", false).unwrap();
        let light = t.icon_for("main.rs", true).unwrap();
        assert_eq!(dark.ch, light.ch);
        assert_ne!(dark.rgb, light.rgb);
    }
}
