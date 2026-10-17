//! Finder's folder customization (macOS 26 and later): an emoji or an SF
//! Symbol drawn on the folder, and the folder's color. Not from Explorer.
//! Found by reading folders Finder customized and checking what
//! `NSWorkspace` draws for folders written by hand (macOS 27):
//!
//! - `com.apple.icon.folder#S` holds JSON, `{"emoji":"🚀"}` or
//!   `{"sym":"star.fill"}`. It is only drawn while `kHasCustomIcon` is set in
//!   the Finder flags.
//! - The color is a tag: the *last* tag with a color in
//!   `com.apple.metadata:_kMDItemUserTags`, a binary plist array of
//!   `"Name\nN"` strings (`N` is the color, 1 to 7). Finder copies it into
//!   the label bits of the Finder flags, which don't change the drawing.
//!   Finder draws the color only on a folder that also has an emblem.
//!
//! Macsplorer adds colors of its own: the exact color goes in
//! `dev.macsplorer.folder-color` (`#rrggbb`) and Finder gets the nearest tag
//! color. When the tag no longer matches (the color was changed in Finder),
//! Finder's color wins.
//!
//! Linux has colors only, in `user.macsplorer.folder-color` (`#rrggbb`),
//! which no other program reads.

/// Finder's tag colors, numbered as in the tags attribute and the label bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TagColor {
    Gray = 1,
    Green = 2,
    Purple = 3,
    Blue = 4,
    Yellow = 5,
    Red = 6,
    Orange = 7,
}

impl TagColor {
    /// In the order Finder lists them.
    pub const ALL: [TagColor; 7] = [Self::Red, Self::Orange, Self::Yellow, Self::Green, Self::Blue, Self::Purple, Self::Gray];

    pub fn number(self) -> u8 {
        self as u8
    }

    pub fn from_number(n: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.number() == n)
    }

    /// The name of Finder's standard tag for the color (English).
    pub fn name(self) -> &'static str {
        match self {
            Self::Gray => "Gray",
            Self::Green => "Green",
            Self::Purple => "Purple",
            Self::Blue => "Blue",
            Self::Yellow => "Yellow",
            Self::Red => "Red",
            Self::Orange => "Orange",
        }
    }

    /// The tag string Finder stores for its standard tag of this color.
    fn tag(self) -> String {
        format!("{}\n{}", self.name(), self.number())
    }

    /// The color as sRGB: the system colors Finder shows tags in (light mode).
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::Red => [255, 59, 48],
            Self::Orange => [255, 149, 0],
            Self::Yellow => [255, 204, 0],
            Self::Green => [40, 205, 65],
            Self::Blue => [0, 122, 255],
            Self::Purple => [175, 82, 222],
            Self::Gray => [142, 142, 147],
        }
    }

    /// The tag color that looks most like `rgb`: Gray for grays, black and
    /// white, else the color with the nearest hue in Oklab, which follows how
    /// colors look (a plain distance would call dark colors gray).
    pub fn nearest(rgb: [u8; 3]) -> Self {
        let [_, a, b] = oklab(rgb);
        if a.hypot(b) < 0.03 {
            return Self::Gray;
        }
        let hue = |a: f32, b: f32| b.atan2(a);
        let apart = |c: &TagColor| {
            let [_, ca, cb] = oklab(c.rgb());
            let d = (hue(a, b) - hue(ca, cb)).abs() % std::f32::consts::TAU;
            d.min(std::f32::consts::TAU - d)
        };
        Self::ALL.into_iter().filter(|c| *c != Self::Gray).min_by(|x, y| apart(x).total_cmp(&apart(y))).unwrap_or(Self::Gray)
    }
}

/// A folder color: one of Finder's, or Macsplorer's own exact color, which
/// Finder sees as the nearest of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FolderColor {
    Tag(TagColor),
    Custom([u8; 3]),
}

impl FolderColor {
    /// One of Finder's colors when `rgb` is exactly one, else our own.
    pub fn from_rgb(rgb: [u8; 3]) -> Self {
        TagColor::ALL.into_iter().find(|c| c.rgb() == rgb).map_or(Self::Custom(rgb), Self::Tag)
    }

    /// The tag color Finder shows.
    pub fn tag(self) -> TagColor {
        match self {
            Self::Tag(c) => c,
            Self::Custom(rgb) => TagColor::nearest(rgb),
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::Tag(c) => c.rgb(),
            Self::Custom(rgb) => rgb,
        }
    }
}

/// `#rrggbb`.
pub fn format_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// `#rrggbb` or `rrggbb`, either case.
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let part = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([part(0)?, part(2)?, part(4)?])
}

/// sRGB to Oklab (Björn Ottosson's matrices).
fn oklab(rgb: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(|c| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    });
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// What Finder draws on the folder.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Emblem {
    Emoji(String),
    /// An SF Symbol, by name.
    Symbol(String),
}

impl Emblem {
    /// What the user typed: empty is none, a name made of ASCII letters,
    /// digits and dots (`star.fill`) is an SF Symbol, anything else an emoji.
    pub fn from_input(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            None
        } else if text.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
            Some(Self::Symbol(text.to_owned()))
        } else {
            Some(Self::Emoji(text.to_owned()))
        }
    }

    /// The text to put back in the input box.
    pub fn text(&self) -> &str {
        match self {
            Self::Emoji(s) | Self::Symbol(s) => s,
        }
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn to_json(&self) -> String {
        let (key, value) = match self {
            Self::Emoji(s) => ("emoji", s),
            Self::Symbol(s) => ("sym", s),
        };
        serde_json::json!({ key: value }).to_string()
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn from_json(data: &[u8]) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(data).ok()?;
        let text = |key: &str| value.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
        text("emoji").map(Self::Emoji).or_else(|| text("sym").map(Self::Symbol))
    }
}

/// A folder's customization.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FolderStyle {
    pub color: Option<FolderColor>,
    pub emblem: Option<Emblem>,
}

impl FolderStyle {
    pub fn is_customized(&self) -> bool {
        self.color.is_some() || self.emblem.is_some()
    }
}

/// The color of a tag string (`"Red\n6"`), if it has one.
fn tag_color(tag: &str) -> Option<TagColor> {
    let (_, n) = tag.rsplit_once('\n')?;
    TagColor::from_number(n.parse().ok()?)
}

/// The color Finder gives a folder with `tags`: the last one with a color.
pub fn color_of(tags: &[String]) -> Option<TagColor> {
    tags.iter().rev().find_map(|t| tag_color(t))
}

/// `tags` with the folder color changed: Finder's standard color tags
/// (named Red, Orange, ...) are taken off and the new one goes last, where
/// it wins. Other tags stay, so a colored tag of the user's own still colors
/// the folder after "no color" (inferred: what Finder does then isn't checked).
pub fn with_color(tags: &[String], color: Option<TagColor>) -> Vec<String> {
    let standard = |tag: &String| {
        let name = tag.split('\n').next().unwrap_or_default();
        TagColor::ALL.iter().any(|c| c.name().eq_ignore_ascii_case(name))
    };
    let mut out: Vec<String> = tags.iter().filter(|t| !standard(t)).cloned().collect();
    out.extend(color.map(TagColor::tag));
    out
}

/// `kHasCustomIcon` in the Finder flags.
const K_HAS_CUSTOM_ICON: u16 = 0x0400;
/// The label color, bits 1 to 3 of the Finder flags.
const K_COLOR: u16 = 0x000e;

/// A `com.apple.FinderInfo` value with the custom icon flag and the label
/// color set; the flags are bytes 8 and 9, big endian.
pub fn with_flags(mut info: [u8; 32], custom_icon: bool, color: Option<TagColor>) -> [u8; 32] {
    let mut flags = u16::from_be_bytes([info[8], info[9]]) & !(K_HAS_CUSTOM_ICON | K_COLOR);
    if custom_icon {
        flags |= K_HAS_CUSTOM_ICON;
    }
    flags |= u16::from(color.map_or(0, TagColor::number)) << 1;
    info[8..10].copy_from_slice(&flags.to_be_bytes());
    info
}

/// Whether folders can be customized here at all.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "linux"));
/// Whether folders can have an emblem here (Linux has colors only).
pub const EMBLEMS: bool = cfg!(target_os = "macos");

#[cfg(target_os = "macos")]
pub use self::mac::{may_be_customized, read, write};

#[cfg(target_os = "linux")]
pub use self::linux::{may_be_customized, read, write};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub use self::unsupported::{may_be_customized, read, write};

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::CStr;
    use std::io;
    use std::path::Path;

    use super::*;
    use crate::{bplist, xattr};

    const EMBLEM: &CStr = c"com.apple.icon.folder#S";
    const TAGS: &CStr = c"com.apple.metadata:_kMDItemUserTags";
    const FINDER_INFO: &CStr = c"com.apple.FinderInfo";
    const CUSTOM_COLOR: &CStr = c"dev.macsplorer.folder-color";

    /// Whether a folder with these Finder flags can have a style: Finder
    /// sets a custom icon or a label color along with one, so the listing
    /// skips reading the others for most folders.
    pub fn may_be_customized(finder_flags: u16) -> bool {
        finder_flags & (K_HAS_CUSTOM_ICON | K_COLOR) != 0
    }

    /// The customization of the folder at `path`.
    pub fn read(path: &Path) -> FolderStyle {
        let flags = xattr::read(path, FINDER_INFO).filter(|i| i.len() >= 10).map_or(0, |i| u16::from_be_bytes([i[8], i[9]]));
        let emblem = xattr::read(path, EMBLEM).filter(|_| flags & K_HAS_CUSTOM_ICON != 0).and_then(|d| Emblem::from_json(&d));
        let tag = color_of(&tags(path));
        let custom = xattr::read(path, CUSTOM_COLOR).and_then(|d| parse_hex(&String::from_utf8_lossy(&d)));
        // Our own color only counts while Finder's still matches it.
        let color = match (tag, custom) {
            (Some(tag), Some(rgb)) if TagColor::nearest(rgb) == tag => Some(FolderColor::Custom(rgb)),
            (tag, _) => tag.map(FolderColor::Tag),
        };
        FolderStyle { color, emblem }
    }

    /// Writes `style` the way Finder does, so Finder shows it too.
    pub fn write(path: &Path, style: &FolderStyle) -> io::Result<()> {
        match &style.emblem {
            Some(emblem) => xattr::set(path, EMBLEM, emblem.to_json().as_bytes())?,
            None => xattr::remove(path, EMBLEM)?,
        }
        match style.color {
            Some(FolderColor::Custom(rgb)) => xattr::set(path, CUSTOM_COLOR, format_hex(rgb).as_bytes())?,
            _ => xattr::remove(path, CUSTOM_COLOR)?,
        }
        let old_tags = tags(path);
        let new_tags = with_color(&old_tags, style.color.map(FolderColor::tag));
        if new_tags != old_tags {
            if new_tags.is_empty() { xattr::remove(path, TAGS)? } else { xattr::set(path, TAGS, &bplist::encode_strings(&new_tags))? }
        }
        let mut info = [0u8; 32];
        if let Some(old) = xattr::read(path, FINDER_INFO) {
            let n = old.len().min(32);
            info[..n].copy_from_slice(&old[..n]);
        }
        // A pasted custom icon (the `Icon\r` file) keeps the flag.
        let custom_icon = style.emblem.is_some() || path.join("Icon\r").exists();
        let new_info = with_flags(info, custom_icon, color_of(&new_tags));
        if new_info != info {
            if new_info == [0; 32] { xattr::remove(path, FINDER_INFO)? } else { xattr::set(path, FINDER_INFO, &new_info)? }
        }
        Ok(())
    }

    fn tags(path: &Path) -> Vec<String> {
        xattr::read(path, TAGS).and_then(|d| bplist::strings(&d)).unwrap_or_default()
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::CStr;
    use std::io;
    use std::path::Path;

    use super::*;
    use crate::xattr;

    const COLOR: &CStr = c"user.macsplorer.folder-color";

    /// No Finder flags here: every folder is read (one attribute).
    pub fn may_be_customized(_finder_flags: u16) -> bool {
        true
    }

    pub fn read(path: &Path) -> FolderStyle {
        let color = xattr::read(path, COLOR).and_then(|d| parse_hex(&String::from_utf8_lossy(&d)));
        FolderStyle { color: color.map(FolderColor::from_rgb), emblem: None }
    }

    /// Writes the color; an emblem isn't stored (see [`EMBLEMS`]).
    pub fn write(path: &Path, style: &FolderStyle) -> io::Result<()> {
        match style.color {
            Some(color) => xattr::set(path, COLOR, format_hex(color.rgb()).as_bytes()),
            None => xattr::remove(path, COLOR),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod unsupported {
    use std::io;
    use std::path::Path;

    use super::FolderStyle;

    pub fn may_be_customized(_finder_flags: u16) -> bool {
        false
    }

    pub fn read(_path: &Path) -> FolderStyle {
        FolderStyle::default()
    }

    pub fn write(_path: &Path, _style: &FolderStyle) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn input_is_an_emoji_or_a_symbol_name() {
        assert_eq!(Emblem::from_input("  "), None);
        assert_eq!(Emblem::from_input(" star.fill "), Some(Emblem::Symbol("star.fill".into())));
        assert_eq!(Emblem::from_input("figure.outdoor.cycle"), Some(Emblem::Symbol("figure.outdoor.cycle".into())));
        assert_eq!(Emblem::from_input("👨🏻‍🎓"), Some(Emblem::Emoji("👨🏻‍🎓".into())));
        assert_eq!(Emblem::from_input("❤️"), Some(Emblem::Emoji("❤️".into())));
    }

    #[test]
    fn json_matches_finder() {
        assert_eq!(Emblem::Emoji("👨🏻‍🎓".into()).to_json(), r#"{"emoji":"👨🏻‍🎓"}"#);
        assert_eq!(Emblem::Symbol("house.fill".into()).to_json(), r#"{"sym":"house.fill"}"#);
        assert_eq!(Emblem::from_json(r#"{"sym":"shower.fill"}"#.as_bytes()), Some(Emblem::Symbol("shower.fill".into())));
        assert_eq!(Emblem::from_json(r#"{"emoji":"🚀"}"#.as_bytes()), Some(Emblem::Emoji("🚀".into())));
        assert_eq!(Emblem::from_json(br#"{"emoji":""}"#), None);
        assert_eq!(Emblem::from_json(b"junk"), None);
    }

    #[test]
    fn the_last_colored_tag_wins() {
        assert_eq!(color_of(&tags(&["Work", "Red\n6", "Green\n2"])), Some(TagColor::Green));
        assert_eq!(color_of(&tags(&["Green\n2", "Red\n6", "Work"])), Some(TagColor::Red));
        assert_eq!(color_of(&tags(&["Work", "Home\n0"])), None);
        assert_eq!(color_of(&[]), None);
    }

    #[test]
    fn changing_the_color_replaces_the_standard_tag() {
        let before = tags(&["Red\n6", "Work", "Urgent\n4"]);
        assert_eq!(with_color(&before, Some(TagColor::Yellow)), tags(&["Work", "Urgent\n4", "Yellow\n5"]));
        assert_eq!(color_of(&with_color(&before, Some(TagColor::Yellow))), Some(TagColor::Yellow));
        // The user's own colored tag stays, and colors the folder.
        assert_eq!(with_color(&before, None), tags(&["Work", "Urgent\n4"]));
        assert_eq!(with_color(&tags(&["red"]), None), Vec::<String>::new());
    }

    #[test]
    fn flags_keep_the_other_bits() {
        let mut info = [0u8; 32];
        info[8] = 0x40; // kIsInvisible
        info[0] = 7;
        let set = with_flags(info, true, Some(TagColor::Red));
        assert_eq!((set[0], set[8], set[9]), (7, 0x44, 0x0c));
        let cleared = with_flags(set, false, None);
        assert_eq!((cleared[8], cleared[9]), (0x40, 0));
        // icm, as Finder left it: custom icon and Red.
        assert_eq!(&with_flags([0; 32], true, Some(TagColor::Red))[8..10], &[0x04, 0x0c]);
    }

    #[test]
    fn nearest_tag_colors() {
        for c in TagColor::ALL {
            assert_eq!(TagColor::nearest(c.rgb()), c);
        }
        assert_eq!(TagColor::nearest([128, 0, 0]), TagColor::Red);
        assert_eq!(TagColor::nearest([0, 0, 128]), TagColor::Blue);
        assert_eq!(TagColor::nearest([0, 128, 128]), TagColor::Green);
        assert_eq!(TagColor::nearest([255, 128, 192]), TagColor::Purple);
        assert_eq!(TagColor::nearest([128, 128, 64]), TagColor::Yellow);
        assert_eq!(TagColor::nearest([128, 128, 128]), TagColor::Gray);
        assert_eq!(TagColor::nearest([128, 64, 0]), TagColor::Orange);
        assert_eq!(TagColor::nearest([255, 255, 255]), TagColor::Gray);
        assert_eq!(TagColor::nearest([0, 0, 0]), TagColor::Gray);
    }

    #[test]
    fn finders_own_colors_stay_tag_colors() {
        assert_eq!(FolderColor::from_rgb(TagColor::Red.rgb()), FolderColor::Tag(TagColor::Red));
        assert_eq!(FolderColor::from_rgb([0, 128, 128]), FolderColor::Custom([0, 128, 128]));
    }

    /// Linux keeps the color only. Skips where the temporary folder has no
    /// user attributes.
    #[cfg(target_os = "linux")]
    #[test]
    fn writes_and_reads_a_color() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("macsplorer-folder-color-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(read(&dir), FolderStyle::default());
        let teal = FolderStyle { color: Some(FolderColor::Custom([0, 128, 128])), emblem: None };
        if write(&dir, &teal).is_ok() {
            assert_eq!(read(&dir), teal);
            let red = FolderStyle { color: Some(FolderColor::Tag(TagColor::Red)), emblem: None };
            write(&dir, &red).unwrap();
            assert_eq!(read(&dir), red);
            write(&dir, &FolderStyle::default()).unwrap();
            assert_eq!(read(&dir), FolderStyle::default());
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hex_colors() {
        assert_eq!(format_hex([0, 200, 160]), "#00c8a0");
        assert_eq!(parse_hex(" #00C8A0 "), Some([0, 200, 160]));
        assert_eq!(parse_hex("00c8a0"), Some([0, 200, 160]));
        assert_eq!(parse_hex("#00c8a"), None);
        assert_eq!(parse_hex("#00c8ag"), None);
        assert_eq!(parse_hex("#ééé"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn writes_and_reads_a_folder() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("macsplorer-folder-style-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(read(&dir), FolderStyle::default());

        let style = FolderStyle { color: Some(FolderColor::Tag(TagColor::Purple)), emblem: Some(Emblem::Symbol("house.fill".into())) };
        write(&dir, &style).unwrap();
        assert_eq!(read(&dir), style);
        let attr = |name: &str| {
            let out = std::process::Command::new("xattr").args(["-px", name]).arg(&dir).output().unwrap();
            String::from_utf8(out.stdout).unwrap().split_whitespace().collect::<String>().to_lowercase()
        };
        assert_eq!(&attr("com.apple.FinderInfo")[16..20], "0406");
        assert_eq!(attr("com.apple.icon.folder#S"), "7b2273796d223a22686f7573652e66696c6c227d");

        let emoji = FolderStyle { color: None, emblem: Some(Emblem::Emoji("🚀".into())) };
        write(&dir, &emoji).unwrap();
        assert_eq!(read(&dir), emoji);
        assert_eq!(&attr("com.apple.FinderInfo")[16..20], "0400");
        assert_eq!(attr("com.apple.metadata:_kMDItemUserTags"), "");

        // Our own color: Finder gets the nearest tag, we read the exact one back.
        let teal = FolderStyle { color: Some(FolderColor::Custom([0, 128, 128])), emblem: None };
        write(&dir, &teal).unwrap();
        assert_eq!(read(&dir), teal);
        assert_eq!(attr("com.apple.metadata:_kMDItemUserTags"), crate::bplist::encode_strings(&["Green\n2".into()]).iter().map(|b| format!("{b:02x}")).collect::<String>());
        // Changed to Blue in Finder: Finder's color wins.
        let blue = crate::bplist::encode_strings(&["Blue\n4".into()]).iter().map(|b| format!("{b:02x}")).collect::<String>();
        std::process::Command::new("xattr").args(["-wx", "com.apple.metadata:_kMDItemUserTags", &blue]).arg(&dir).status().unwrap();
        assert_eq!(read(&dir).color, Some(FolderColor::Tag(TagColor::Blue)));

        write(&dir, &FolderStyle::default()).unwrap();
        assert_eq!(read(&dir), FolderStyle::default());
        let names = std::process::Command::new("xattr").arg(&dir).output().unwrap().stdout;
        let names = String::from_utf8(names).unwrap();
        assert!(!names.contains("FinderInfo") && !names.contains("icon.folder") && !names.contains("UserTags") && !names.contains("macsplorer"), "{names}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
