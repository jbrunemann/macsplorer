//! Natural ("logical") string ordering.
//!
//! Explorer orders names with `StrCmpLogicalW` unless the `NoStrCmpLogical`
//! policy is set, in which case it falls back to a plain case-insensitive
//! compare (see `docs/file-browser-findings.md` §2.2). This module provides
//! both so the app can offer the same switch.
//!
//! Both compare letters by the user locale's collation, as `CompareStringW`
//! does under them: "Äpfel" sorts with the A's in German and after Z in
//! Swedish. The collation comes from ICU4X (CLDR data), so the order is
//! the same on every OS; the app picks the locale with
//! [`set_collation_locale`], root collation (most languages, English
//! included) until then.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::OnceLock;

use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{CollatorBorrowed, CollatorPreferences};
use icu_locale_core::Locale;
use icu_normalizer::ComposingNormalizerBorrowed;

static COLLATORS: OnceLock<Collators> = OnceLock::new();

struct Collators {
    natural: CollatorBorrowed<'static>,
    plain: CollatorBorrowed<'static>,
}

/// Sorts names by the collation of `tag`, a BCP 47 language tag ("sv-SE";
/// "de_DE.UTF-8" from a POSIX locale works too). Call once at startup,
/// before anything sorts; later calls and unknown tags are ignored.
pub fn set_collation_locale(tag: &str) {
    if let Some(collators) = collators_for(tag) {
        let _ = COLLATORS.set(collators);
    }
}

/// Ignores case like `NORM_IGNORECASE` (secondary strength) but keeps
/// accents apart: "e" and "é" differ, "E" and "e" don't. The natural one
/// reads digit runs as numbers (CLDR's `kn`), which is what Finder's
/// `localizedStandardCompare:` does too.
fn collators_for(tag: &str) -> Option<Collators> {
    // POSIX names: "de_DE.UTF-8@euro" -> "de-DE"; "C" and "POSIX" mean root.
    let tag = tag.split(['.', '@']).next().unwrap_or_default().replace('_', "-");
    let locale = match tag.as_str() {
        "" | "C" | "POSIX" => Locale::UNKNOWN,
        _ => Locale::try_from_str(&tag).ok()?,
    };
    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Secondary);
    let mut prefs = CollatorPreferences::from(&locale);
    let plain = CollatorBorrowed::try_new(prefs, options).ok()?;
    prefs.numeric_ordering = Some(CollationNumericOrdering::True);
    Some(Collators { natural: CollatorBorrowed::try_new(prefs, options).ok()?, plain })
}

fn collators() -> &'static Collators {
    COLLATORS.get_or_init(|| collators_for("").expect("root collation is compiled in"))
}

/// The form names are matched in: NFC, lowercase. Names from HFS+, SMB
/// shares and zip files often arrive decomposed ("a" + combining
/// diaeresis) while typed text is composed, so both sides go through this
/// before a type-ahead or search match.
pub fn fold(s: &str) -> String {
    let composed: Cow<str> = ComposingNormalizerBorrowed::new_nfc().normalize(s);
    composed.to_lowercase()
}

/// Case-insensitive comparison where runs of digits compare by numeric
/// value, so `file2` sorts before `file10`.
///
/// Names that compare equal ignoring case and leading zeros are ordered
/// ordinally as a last resort, which keeps the order total and stable.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    collators().natural.compare(a, b).then_with(|| a.cmp(b))
}

/// Plain case-insensitive comparison, the `NoStrCmpLogical` fallback.
pub fn plain_cmp(a: &str, b: &str) -> Ordering {
    collators().plain.compare(a, b).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<&str>, f: fn(&str, &str) -> Ordering) -> Vec<&str> {
        v.sort_by(|a, b| f(a, b));
        v
    }

    #[test]
    fn numbers_compare_by_value() {
        assert_eq!(
            sorted(vec!["file10", "file2", "file1", "File3"], natural_cmp),
            ["file1", "file2", "File3", "file10"]
        );
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(natural_cmp("abc", "ABD"), Ordering::Less);
        assert!(natural_cmp("Report", "report").is_ne());
    }

    #[test]
    fn leading_zeros_tie_break_ordinally() {
        assert_eq!(
            sorted(vec!["a1", "a01", "a001"], natural_cmp),
            ["a001", "a01", "a1"]
        );
    }

    #[test]
    fn huge_numbers_do_not_overflow() {
        assert_eq!(
            natural_cmp("x99999999999999999999999", "x100000000000000000000000"),
            Ordering::Less
        );
    }

    #[test]
    fn plain_sort_is_lexicographic() {
        assert_eq!(
            sorted(vec!["file10", "file2", "file1"], plain_cmp),
            ["file1", "file10", "file2"]
        );
    }

    fn sorted_in(tag: &str, mut v: Vec<&'static str>) -> Vec<&'static str> {
        let collators = collators_for(tag).unwrap();
        v.sort_by(|a, b| collators.natural.compare(a, b).then_with(|| a.cmp(b)));
        v
    }

    #[test]
    fn letters_follow_the_locale() {
        let names = vec!["Zebra", "Äpfel", "Apfel", "Ost", "Öl"];
        assert_eq!(sorted_in("de-DE", names.clone()), ["Apfel", "Äpfel", "Öl", "Ost", "Zebra"]);
        assert_eq!(sorted_in("sv_SE.UTF-8", names.clone()), ["Apfel", "Ost", "Zebra", "Äpfel", "Öl"]);
        assert_eq!(sorted_in("en-GB", vec!["Zoo", "École", "Eagle"]), ["Eagle", "École", "Zoo"]);
    }

    #[test]
    fn root_until_a_locale_is_set() {
        assert_eq!(sorted(vec!["Zoo", "École", "eagle"], natural_cmp), ["eagle", "École", "Zoo"]);
        assert_eq!(sorted(vec!["Zoo", "École", "eagle"], plain_cmp), ["eagle", "École", "Zoo"]);
        assert!(collators_for("C").is_some() && collators_for("POSIX").is_some());
    }

    #[test]
    fn punctuation_digits_letters() {
        assert_eq!(sorted(vec!["b", "a1", "_a", "a_1", "1"], natural_cmp), ["_a", "1", "a_1", "a1", "b"]);
    }

    #[test]
    fn decomposed_names_sort_with_composed_ones() {
        let (composed, decomposed) = ("caf\u{e9}", "cafe\u{301}");
        assert_eq!(sorted(vec!["cafz", decomposed, "cafa"], natural_cmp), ["cafa", decomposed, "cafz"]);
        assert_ne!(natural_cmp(composed, decomposed), Ordering::Equal);
        assert_eq!(fold("Cafe\u{301}"), fold("CAF\u{c9}"));
    }
}
