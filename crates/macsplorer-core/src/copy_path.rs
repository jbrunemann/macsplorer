//! "Copy as path": the text Explorer puts on the clipboard for a selection.
//!
//! Explorer-derived (shell32 `CCopyAsPathMenu::Invoke`): each item's full
//! path is wrapped in double quotes and the items are joined one per line;
//! the result goes on the clipboard as plain Unicode text. The quoting is
//! what makes the text safe to paste into a shell or a dialog when the path
//! has spaces. Embedded quotes are not escaped (Explorer never needs to, as
//! Windows forbids `"` in names; unverified what a macOS name with one
//! should do, so it is left as is). The line separator is a guess: Explorer
//! joins with a separator string that was not decoded, `\n` is used here.

use std::path::PathBuf;

/// The clipboard text for `paths`: `"/a/b"` per line.
pub fn as_path_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| format!("\"{}\"", p.display())).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_path_is_quoted() {
        assert_eq!(as_path_text(&[PathBuf::from("/Users/me/My File.txt")]), "\"/Users/me/My File.txt\"");
    }

    #[test]
    fn several_paths_go_one_per_line() {
        let paths = [PathBuf::from("/a"), PathBuf::from("/b c")];
        assert_eq!(as_path_text(&paths), "\"/a\"\n\"/b c\"");
    }

    #[test]
    fn nothing_is_empty() {
        assert_eq!(as_path_text(&[]), "");
    }
}
