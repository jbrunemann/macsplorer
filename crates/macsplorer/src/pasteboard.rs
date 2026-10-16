//! Files on the system pasteboard, so Cut and Copy work between Macsplorer
//! windows (each one is its own process) and with Finder.
//!
//! Finder's Copy puts one pasteboard item per file, each with a
//! `public.file-url` (a file reference URL such as `file:///.file/id=1.2`,
//! which has to be resolved to a path) and its name as plain text. Macsplorer
//! writes the same, so Finder can paste what was copied here. Finder has no Cut
//! (apps like Command X keep it to themselves, nothing on the pasteboard marks
//! it), so a cut is marked here with a private type on the first item. Finder
//! ignores it and pastes a copy, or moves with Option+Cmd+V ("Move Item Here").
//!
//! Only macOS has this for now; elsewhere `SUPPORTED` is false and the app keeps
//! its own in-memory clipboard.

use std::path::PathBuf;

/// Whether this platform shares Cut and Copy through the system pasteboard.
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// What a Cut or Copy left on the pasteboard.
pub struct Files {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

#[cfg(target_os = "macos")]
pub use mac::{Pasteboard, files_on};

#[cfg(not(target_os = "macos"))]
pub use other::Pasteboard;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeString, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSString, NSURL};

    use super::Files;

    /// Marks a cut: a type on the first item, which only Macsplorer knows.
    const CUT_TYPE: &str = "dev.macsplorer.cut";

    pub(super) fn file_url_type() -> &'static NSString {
        // SAFETY: a constant AppKit initializes before any code runs.
        unsafe { NSPasteboardTypeFileURL }
    }

    fn string_type() -> &'static NSString {
        // SAFETY: as above.
        unsafe { NSPasteboardTypeString }
    }

    /// The files on any pasteboard (the general one, or a drag's), in order.
    pub fn files_on(pb: &NSPasteboard) -> Vec<PathBuf> {
        let Some(items) = pb.pasteboardItems() else { return Vec::new() };
        let mut paths = Vec::new();
        for item in items.iter() {
            let Some(url) = item.stringForType(file_url_type()) else { continue };
            // filePathURL turns a file reference URL into the path it stands for.
            let path = NSURL::URLWithString(&url).and_then(|u| u.filePathURL()).and_then(|u| u.path());
            paths.extend(path.map(|p| PathBuf::from(p.to_string())));
        }
        paths
    }

    /// A handle on a pasteboard: the system's general one, or a private one in tests.
    #[derive(Clone)]
    pub struct Pasteboard(pub(super) Retained<NSPasteboard>);

    impl Pasteboard {
        pub fn general() -> Self {
            Self(NSPasteboard::generalPasteboard())
        }

        /// A pasteboard of its own, so tests don't touch the user's clipboard.
        #[cfg(test)]
        pub fn private() -> Self {
            Self(NSPasteboard::pasteboardWithUniqueName())
        }

        /// Changes whenever anything puts something new on the pasteboard.
        /// Reading it doesn't count as accessing the contents.
        pub fn change_count(&self) -> Option<isize> {
            Some(self.0.changeCount())
        }

        /// Whether there are files to paste. This asks only for the types, not the
        /// data, so macOS doesn't treat it as reading the clipboard.
        pub fn has_files(&self) -> bool {
            self.0.canReadItemWithDataConformingToTypes(&NSArray::from_slice(&[file_url_type()]))
        }

        /// Replaces the pasteboard's contents with `paths`, marked as cut or not.
        pub fn write(&self, paths: &[PathBuf], cut: bool) -> bool {
            let mut items = Vec::new();
            for (i, path) in paths.iter().enumerate() {
                let Some(text) = path.to_str() else { continue };
                let Some(url) = NSURL::fileURLWithPath(&NSString::from_str(text)).absoluteString() else { continue };
                let item = NSPasteboardItem::new();
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                item.setString_forType(&url, file_url_type());
                item.setString_forType(&NSString::from_str(&name), string_type());
                if cut && i == 0 {
                    item.setString_forType(&NSString::from_str("1"), &NSString::from_str(CUT_TYPE));
                }
                items.push(ProtocolObject::<dyn NSPasteboardWriting>::from_retained(item));
            }
            if items.is_empty() {
                return false;
            }
            self.0.clearContents();
            self.0.writeObjects(&NSArray::from_retained_slice(&items))
        }

        /// The files on the pasteboard, if any.
        pub fn read(&self) -> Option<Files> {
            let items = self.0.pasteboardItems()?;
            let cut_type = NSString::from_str(CUT_TYPE);
            let cut = items.iter().any(|item| item.types().iter().any(|t| *t == *cut_type));
            let paths = files_on(&self.0);
            (!paths.is_empty()).then_some(Files { paths, cut })
        }

        /// Empties the pasteboard, as a paste of a cut does.
        pub fn clear(&self) {
            self.0.clearContents();
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use std::path::PathBuf;

    use super::Files;

    #[derive(Clone)]
    pub struct Pasteboard;

    impl Pasteboard {
        pub fn general() -> Self {
            Self
        }

        #[cfg(test)]
        pub fn private() -> Self {
            Self
        }

        pub fn change_count(&self) -> Option<isize> {
            None
        }

        pub fn has_files(&self) -> bool {
            false
        }

        pub fn write(&self, _paths: &[PathBuf], _cut: bool) -> bool {
            false
        }

        pub fn read(&self) -> Option<Files> {
            None
        }

        pub fn clear(&self) {}
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn files_round_trip_with_the_cut_mark() {
        let pb = Pasteboard::private();
        assert!(!pb.has_files());
        let paths = vec![PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/c.txt")];
        let before = pb.change_count();
        assert!(pb.write(&paths, true));
        assert_ne!(pb.change_count(), before);
        assert!(pb.has_files());
        let files = pb.read().unwrap();
        assert_eq!(files.paths, paths);
        assert!(files.cut);

        assert!(pb.write(&paths, false));
        assert!(!pb.read().unwrap().cut, "a new copy drops the cut mark");

        pb.clear();
        assert!(!pb.has_files() && pb.read().is_none());
    }

    #[test]
    fn a_file_reference_url_resolves_to_its_path() {
        use objc2_foundation::NSString;
        let dir = std::env::temp_dir().join(format!("macsplorer-pb-ref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ref.txt");
        std::fs::write(&file, "x").unwrap();
        // Finder writes file:///.file/id=... URLs.
        let reference = objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(file.to_str().unwrap())).fileReferenceURL().unwrap();
        let text = reference.absoluteString().unwrap().to_string();
        assert!(text.contains("/.file/id="), "{text}");

        let pb = Pasteboard::private();
        let item = objc2_app_kit::NSPasteboardItem::new();
        item.setString_forType(&NSString::from_str(&text), super::mac::file_url_type());
        pb.0.clearContents();
        assert!(pb.0.writeObjects(&objc2_foundation::NSArray::from_retained_slice(&[objc2::runtime::ProtocolObject::from_retained(item)])));
        let files = pb.read().unwrap();
        assert_eq!(files.paths.len(), 1);
        assert_eq!(files.paths[0].canonicalize().unwrap(), file.canonicalize().unwrap());
        assert!(!files.cut);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
