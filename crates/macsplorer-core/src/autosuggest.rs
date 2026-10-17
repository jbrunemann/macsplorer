//! Predicting the folder typed in the address bar, like the Fish shell's
//! autosuggestion: the rest of the name shows dimmed after the cursor and
//! Right (or End) takes it.
//!
//! Not from Explorer, which drops down a list instead (`CAutoComplete`):
//! this follows Fish, see `docs/file-browser-findings.md` §3.

use icu_normalizer::DecomposingNormalizerBorrowed;

use crate::listing::Entry;
use crate::natural::{fold, natural_cmp};

/// A folder in the directory the typed path is inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    pub hidden: bool,
}

impl Folder {
    /// The folders among `entries`.
    pub fn from_entries(entries: Vec<Entry>) -> Vec<Folder> {
        entries.into_iter().filter(Entry::is_folder).map(|e| Folder { name: e.name, hidden: e.hidden }).collect()
    }
}

/// What to show and what accepting it gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// The dimmed text after the typed one, ending in `/`.
    pub ghost: String,
    /// The whole address once accepted. A name that matched in another case
    /// replaces what was typed, so the path stays right.
    pub completed: String,
}

/// Splits a typed address at its last `/` into the folder to look in and the
/// name being typed. Only absolute paths and `~/` ones are predicted: a
/// relative one means nothing here, and `~name` isn't resolved.
pub fn split(input: &str) -> Option<(&str, &str)> {
    if !(input.starts_with('/') || input.starts_with("~/")) {
        return None;
    }
    let cut = input.rfind('/')? + 1;
    Some(input.split_at(cut))
}

/// The folder `input` is most likely heading for, from the `folders` of the
/// directory [`split`] names. Nothing is suggested before a first letter is
/// typed. Hidden folders only show for a name that starts with a dot.
///
/// A name with the typed letters in the same case beats one that matches in
/// another case, as in Fish, and that beats one that only matches with its
/// accents left out (`A` finds `Äpfel`, `e` finds `éclair`); among equals the
/// first in natural order wins.
pub fn suggest(input: &str, folders: &[Folder]) -> Option<Suggestion> {
    let (parent, partial) = split(input)?;
    if partial.is_empty() {
        return None;
    }
    let dots = partial.starts_with('.');
    let candidates = folders.iter().filter(|f| dots || !f.hidden);
    let first = |matching: &dyn Fn(&Folder) -> bool| candidates.clone().filter(|f| matching(f)).min_by(|a, b| natural_cmp(&a.name, &b.name));
    let wanted = fold(partial);
    let plain = base_letters(partial);
    let found = first(&|f| f.name.starts_with(partial))
        .or_else(|| first(&|f| fold(&f.name).starts_with(&wanted)))
        // Accents typed have to fit; only left-out ones are forgiven.
        .or_else(|| if has_accents(partial) { None } else { first(&|f| base_letters(&f.name).starts_with(&plain)) })?;
    // The shortest start of the name that says what was typed, with the
    // accents that go on its last letter.
    let name = found.name.as_str();
    let start = name.char_indices().map(|(i, _)| i).chain(Some(name.len())).find(|&i| base_letters(&name[..i]) == plain)?;
    let typed_end = start + name[start..].chars().take_while(|&c| is_accent(c)).map(char::len_utf8).sum::<usize>();
    Some(Suggestion { ghost: format!("{}/", &found.name[typed_end..]), completed: format!("{parent}{}/", found.name) })
}

/// `s` in lower case without accents: decomposed, then the combining marks
/// dropped, so `Äpfel` becomes `apfel`.
fn base_letters(s: &str) -> String {
    let decomposed = DecomposingNormalizerBorrowed::new_nfd().normalize(s);
    decomposed.chars().filter(|&c| !is_accent(c)).collect::<String>().to_lowercase()
}

fn has_accents(s: &str) -> bool {
    DecomposingNormalizerBorrowed::new_nfd().normalize(s).chars().any(is_accent)
}

/// A combining diacritical mark, as decomposing a letter leaves behind.
fn is_accent(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(names: &[&str]) -> Vec<Folder> {
        names.iter().map(|n| Folder { name: (*n).into(), hidden: n.starts_with('.') }).collect()
    }

    fn ghost(input: &str, names: &[&str]) -> Option<String> {
        suggest(input, &folders(names)).map(|s| s.ghost)
    }

    #[test]
    fn splits_at_the_last_slash() {
        assert_eq!(split("/Users/me/Doc"), Some(("/Users/me/", "Doc")));
        assert_eq!(split("/Us"), Some(("/", "Us")));
        assert_eq!(split("~/D"), Some(("~/", "D")));
        assert_eq!(split("/Users/"), Some(("/Users/", "")));
        for relative in ["", "Doc", "~", "~me/D", "./x", "'/Users/me"] {
            assert_eq!(split(relative), None, "{relative}");
        }
    }

    #[test]
    fn completes_the_name_being_typed() {
        let names = ["Desktop", "Documents", "Downloads"];
        assert_eq!(ghost("/Users/me/Doc", &names).as_deref(), Some("uments/"));
        assert_eq!(ghost("~/Dow", &names).as_deref(), Some("nloads/"));
        let s = suggest("/Users/me/Doc", &folders(&names)).unwrap();
        assert_eq!(s.completed, "/Users/me/Documents/");
    }

    #[test]
    fn nothing_before_a_first_letter() {
        assert_eq!(ghost("/Users/me/", &["Desktop"]), None);
        assert_eq!(ghost("/", &["Users"]), None);
    }

    #[test]
    fn nothing_when_nothing_matches() {
        assert_eq!(ghost("/Users/me/x", &["Desktop"]), None);
        assert_eq!(ghost("relative/D", &["Desktop"]), None);
    }

    #[test]
    fn a_complete_name_suggests_the_slash() {
        assert_eq!(ghost("/Users", &["Users"]).as_deref(), Some("/"));
    }

    #[test]
    fn the_first_in_natural_order_wins() {
        assert_eq!(ghost("/a/f", &["file10", "file2", "fold"]).as_deref(), Some("ile2/"));
    }

    #[test]
    fn matching_case_beats_other_case() {
        assert_eq!(ghost("/a/d", &["Desktop", "dev"]).as_deref(), Some("ev/"));
        // With no same-case match the name is taken in its own case.
        let s = suggest("/a/des", &folders(&["Desktop"])).unwrap();
        assert_eq!((s.ghost.as_str(), s.completed.as_str()), ("ktop/", "/a/Desktop/"));
    }

    #[test]
    fn hidden_folders_need_a_dot() {
        let names = [".config", "code"];
        assert_eq!(ghost("/h/c", &names).as_deref(), Some("ode/"));
        assert_eq!(ghost("/h/.c", &names).as_deref(), Some("onfig/"));
        assert_eq!(ghost("/h/.", &names).as_deref(), Some("config/"));
    }

    #[test]
    fn accents_need_not_be_typed() {
        let s = suggest("/a/A", &folders(&["\u{c4}pfel", "Banane"])).unwrap();
        assert_eq!((s.ghost.as_str(), s.completed.as_str()), ("pfel/", "/a/\u{c4}pfel/"));
        assert_eq!(ghost("/a/ecl", &["\u{e9}clair"]).as_deref(), Some("air/"));
        // Typed accents still have to fit: `ä` doesn't find `Apfel`.
        assert_eq!(ghost("/a/\u{e4}", &["Apfel"]), None);
        // A match that keeps the accents is the better one.
        assert_eq!(ghost("/a/a", &["\u{e4}pfel", "apfel"]).as_deref(), Some("pfel/"));
        let s = suggest("/a/a", &folders(&["\u{e4}pfel", "apfel"])).unwrap();
        assert_eq!(s.completed, "/a/apfel/");
    }

    #[test]
    fn decomposed_names_match_composed_typing() {
        // "ä" typed as one character, stored as "a" + combining diaeresis.
        let name = "Ba\u{308}ren";
        let s = suggest("/a/B\u{e4}", &folders(&[name])).unwrap();
        assert_eq!(s.ghost, "ren/");
        assert_eq!(s.completed, format!("/a/{name}/"));
    }
}
