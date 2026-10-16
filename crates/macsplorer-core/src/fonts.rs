//! Which fonts the Linux port uses. macOS ships Tahoma; Linux doesn't, so the
//! UI font is an installed Tahoma (`UI_FAMILIES`), else the DejaVu Sans
//! Condensed built into the Linux binary (`assets/fonts`). Not from Explorer:
//! Tahoma is a narrower Verdana, and DejaVu Sans Condensed (a Verdana relative)
//! is about as wide as Tahoma and the closest free look-alike (judged from a
//! screenshot next to Tahoma).

/// Installed families that beat the bundled font, best first.
pub const UI_FAMILIES: &[&str] = &["Tahoma", "Wine Tahoma"];

/// Fonts for the scripts and symbols the UI font lacks (file names can hold
/// any of them), each added when installed. The bundled DejaVu Sans covers
/// Greek, Cyrillic, Arabic, Hebrew and many symbols already.
pub const FALLBACK_FAMILIES: &[&str] = &[
    "Noto Sans Arabic",
    "Noto Sans Hebrew",
    "Noto Sans Devanagari",
    "Noto Sans Thai",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Math",
];

/// Han characters look different per region, so the CJK fallback follows the
/// language (`LANG`, e.g. `ja_JP.UTF-8`). Families are the ones Noto's CJK
/// collection and the distro `Noto Sans <Region>` packages register.
pub fn cjk_families(lang: &str) -> Vec<&'static str> {
    let lang = lang.to_ascii_lowercase();
    let first = if lang.starts_with("ja") {
        ["Noto Sans CJK JP", "Noto Sans JP"]
    } else if lang.starts_with("ko") {
        ["Noto Sans CJK KR", "Noto Sans KR"]
    } else if lang.starts_with("zh_tw") || lang.starts_with("zh_hk") || lang.starts_with("zh_mo") {
        ["Noto Sans CJK TC", "Noto Sans TC"]
    } else {
        ["Noto Sans CJK SC", "Noto Sans SC"]
    };
    // Any region beats a missing glyph.
    first.into_iter().chain(["Noto Sans CJK JP", "Noto Sans CJK SC", "WenQuanYi Micro Hei", "Droid Sans Fallback"]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tahoma_comes_first() {
        assert_eq!(UI_FAMILIES[0], "Tahoma");
    }

    #[test]
    fn cjk_follows_the_language() {
        assert_eq!(cjk_families("ja_JP.UTF-8")[0], "Noto Sans CJK JP");
        assert_eq!(cjk_families("ko_KR.UTF-8")[0], "Noto Sans CJK KR");
        assert_eq!(cjk_families("zh_TW.UTF-8")[0], "Noto Sans CJK TC");
        assert_eq!(cjk_families("zh_CN.UTF-8")[0], "Noto Sans CJK SC");
        assert_eq!(cjk_families("")[0], "Noto Sans CJK SC");
    }
}
