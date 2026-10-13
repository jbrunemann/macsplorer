//! Natural ("logical") string ordering.
//!
//! Explorer orders names with `StrCmpLogicalW` unless the `NoStrCmpLogical`
//! policy is set, in which case it falls back to a plain case-insensitive
//! compare (see `docs/file-browser-findings.md` §2.2). This module provides
//! both so the app can offer the same switch.

use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

/// Case-insensitive comparison where runs of ASCII digits compare by numeric
/// value, so `file2` sorts before `file10`.
///
/// Names that compare equal ignoring case and leading zeros are ordered
/// ordinally as a last resort, which keeps the order total and stable.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ia = a.chars().peekable();
    let mut ib = b.chars().peekable();
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let ord = cmp_digit_runs(&take_digits(&mut ia), &take_digits(&mut ib));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                let ord = ca.to_lowercase().cmp(cb.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ia.next();
                ib.next();
            }
        }
    }
}

/// Plain case-insensitive comparison, the `NoStrCmpLogical` fallback.
pub fn plain_cmp(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
        .then_with(|| a.cmp(b))
}

fn take_digits(it: &mut Peekable<Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        run.push(c);
        it.next();
    }
    run
}

/// Compares two digit runs by value without parsing, so arbitrarily long
/// numbers cannot overflow.
fn cmp_digit_runs(a: &str, b: &str) -> Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
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
}
