//! Finds fonts on a Linux system. `fontdb` scans the font folders the way
//! fontconfig does (system, `~/.local/share/fonts`, `~/.fonts`, the folders
//! listed in `fonts.conf`), so Fedora, Ubuntu and Arch need no path lists.
//! Which families to look for is `macsplorer_core::fonts`.

use fontdb::{Database, Family, Query, Source, Stretch, Style, Weight};
use macsplorer_core::fonts;

/// A font file's bytes and the face inside it (collections such as Noto's CJK `.ttc`).
pub struct Found {
    pub family: String,
    /// The font file, when the face comes from one.
    pub path: Option<String>,
    pub data: Vec<u8>,
    pub index: u32,
}

pub struct SystemFonts {
    db: Database,
}

impl SystemFonts {
    /// Scans the font folders: do it once.
    pub fn scan() -> Self {
        let mut db = Database::new();
        db.load_system_fonts();
        Self { db }
    }

    /// The UI font in the first installed family of `fonts::UI_FAMILIES`, if any.
    /// A family with no bold cut gives its regular one.
    pub fn ui_font(&self, bold: bool) -> Option<Found> {
        fonts::UI_FAMILIES.iter().find_map(|name| self.family(name, bold))
    }

    /// Faces that cover scripts the UI font lacks, one per installed family:
    /// `fonts::FALLBACK_FAMILIES`, then the CJK face for `lang`.
    pub fn fallbacks(&self, lang: &str) -> Vec<Found> {
        let mut found: Vec<Found> = fonts::FALLBACK_FAMILIES.iter().filter_map(|name| self.family(name, false)).collect();
        found.extend(fonts::cjk_families(lang).into_iter().find_map(|name| self.family(name, false)));
        found
    }

    fn family(&self, name: &str, bold: bool) -> Option<Found> {
        let query = Query {
            families: &[Family::Name(name)],
            weight: if bold { Weight::BOLD } else { Weight::NORMAL },
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let id = self.db.query(&query)?;
        let path = match self.db.face_source(id)?.0 {
            Source::File(path) | Source::SharedFile(path, _) => Some(path.display().to_string()),
            Source::Binary(_) => None,
        };
        let (index, data) = self.db.with_face_data(id, |data, index| (index, data.to_vec()))?;
        Some(Found { family: name.to_string(), path, data, index })
    }
}
