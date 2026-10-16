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

/// The font with its TrueType hinting program hidden, so egui's rasterizer
/// (skrifa) grid-fits it with its autohinter instead. DejaVu's own instructions
/// leave its outlines untouched at UI sizes under skrifa's interpreter (checked
/// at 12 px: hinted and unhinted outlines are identical), which renders soft,
/// gray text on a 1x screen; the autohinter snaps the x-height and stems to
/// whole pixels. skrifa takes the interpreter whenever `fpgm` or `prep` exist
/// or `maxp` allows glyph instructions (FreeType's rule), so this renames the
/// two tables (keeping the directory sorted) and zeroes
/// `maxp.maxSizeOfInstructions`. Bytes that aren't a TrueType font come back
/// unchanged.
pub fn prefer_autohinter(font: &[u8]) -> Vec<u8> {
    let mut out = font.to_vec();
    let Some(count) = font.get(4..6).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize) else { return out };
    for i in 0..count {
        let record = 12 + 16 * i;
        let Some(entry) = font.get(record..record + 16) else { return out };
        let offset = u32::from_be_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        match &entry[..4] {
            // '_' sorts below every lowercase letter, so "fpg_" stays between "cvt " and
            // "gasp" and "pre_" after "post".
            b"fpgm" | b"prep" => out[record + 3] = b'_',
            // maxSizeOfInstructions, in the version 1.0 table only.
            b"maxp" if font.get(offset..offset + 4) == Some(&[0, 1, 0, 0]) && out.len() >= offset + 28 => {
                out[offset + 26..offset + 28].fill(0);
            }
            _ => {}
        }
    }
    out
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

    fn tables(font: &[u8]) -> Vec<(String, usize)> {
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        (0..count)
            .map(|i| {
                let r = &font[12 + 16 * i..28 + 16 * i];
                (String::from_utf8_lossy(&r[..4]).into_owned(), u32::from_be_bytes([r[8], r[9], r[10], r[11]]) as usize)
            })
            .collect()
    }

    #[test]
    fn autohinter_font_has_no_hinting_program() {
        let font = prefer_autohinter(include_bytes!("../../../assets/fonts/DejaVuSansCondensed.ttf"));
        let tables = tables(&font);
        let tags: Vec<&str> = tables.iter().map(|(t, _)| t.as_str()).collect();
        assert!(!tags.contains(&"fpgm") && !tags.contains(&"prep"));
        assert!(tags.contains(&"fpg_") && tags.contains(&"pre_"));
        assert!(tags.windows(2).all(|w| w[0] < w[1]), "directory stays sorted: {tags:?}");
        let maxp = tables.iter().find(|(t, _)| t == "maxp").unwrap().1;
        assert_eq!(&font[maxp + 26..maxp + 28], [0, 0]);
    }

    #[test]
    fn other_bytes_come_back_unchanged() {
        assert_eq!(prefer_autohinter(b"abc"), b"abc");
        assert!(prefer_autohinter(&[]).is_empty());
    }
}
