//! Drag and drop of items: when a drag starts, and what a drop does.
//!
//! The rules are shell32's `CFSDropTarget` (build 10240), the drop target of
//! a file system folder (`docs/file-browser-findings.md` §2.14):
//!
//! - Keys force an effect (`CFSDropTarget::_GetDefaultEffect`): Shift moves,
//!   Ctrl copies, Ctrl+Shift or Alt makes a shortcut. On the Mac AppKit
//!   decides this for every drag, Option copying, Command moving and both
//!   making an alias, and the app uses the same keys for its own drags
//!   ([`mac_keys`]).
//! - Without keys (`_FilesystemAdjustedDefaultEffect`): a drive's root makes
//!   a shortcut, an item on the target's drive moves, anything else copies.
//!   When the source allows only one of copy and move, that one is used.
//! - An effect the source doesn't allow is not used: forced, the drop is
//!   refused; by default, the first allowed of copy, move and shortcut is
//!   (`_DetermineEffects`).
//!
//! Not traced: shell32 also asks a per-item property before copying across
//! drives, and a second check on the source that also makes a shortcut;
//! neither applies to plain files and folders as far as known. A folder's
//! `desktop.ini` `DefaultDropEffect` has no Mac counterpart and is left out.
//!
//! Not from Windows: refusing to move items into the folder they're already
//! in (Explorer repositions the icons there instead, `CDefView::IsDropOnSource`)
//! and dropping a folder into itself (Explorer only finds out when the copy
//! engine starts).

use std::path::{Path, PathBuf};

/// What a drop does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Copy,
    Move,
    /// A shortcut: a symbolic link here.
    Link,
}

impl Effect {
    fn bit(self) -> u8 {
        match self {
            Effect::Copy => 1,
            Effect::Move => 2,
            Effect::Link => 4,
        }
    }
}

/// The effects a drag's source allows (`DROPEFFECT_COPY | MOVE | LINK`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowed(u8);

impl Allowed {
    pub const ALL: Allowed = Allowed(7);
    pub const NONE: Allowed = Allowed(0);

    pub fn of(effects: &[Effect]) -> Self {
        Allowed(effects.iter().fold(0, |bits, e| bits | e.bit()))
    }

    pub fn contains(self, effect: Effect) -> bool {
        self.0 & effect.bit() != 0
    }
}

/// Pixels the pointer must move from where the button went down before a
/// press on an item becomes a drag: comctl32's `CheckForDragBeginEx` waits
/// for it to leave a box of `SM_CXDRAG` by `SM_CYDRAG` (4 by default, never
/// less) on each side. The app gives this to egui as its click distance,
/// which egui measures as a circle rather than a box.
pub const DRAG_DISTANCE: f32 = 4.0;

/// Seconds the pointer rests on a collapsed folder in the Folders pane
/// before it expands. Guess: not traced, about what Explorer feels like.
pub const HOVER_EXPAND: f64 = 1.0;

/// The effect Windows' keys force (`_GetDefaultEffect`): Shift moves, Ctrl
/// copies, Ctrl+Shift or Alt (with or without the others) links.
pub fn windows_keys(shift: bool, ctrl: bool, alt: bool) -> Option<Effect> {
    match (shift, ctrl, alt) {
        (_, _, true) | (true, true, _) => Some(Effect::Link),
        (true, false, false) => Some(Effect::Move),
        (false, true, false) => Some(Effect::Copy),
        (false, false, false) => None,
    }
}

/// The effect AppKit's keys force: Option copies, Command moves, both make
/// an alias.
pub fn mac_keys(option: bool, command: bool) -> Option<Effect> {
    match (option, command) {
        (true, true) => Some(Effect::Link),
        (true, false) => Some(Effect::Copy),
        (false, true) => Some(Effect::Move),
        (false, false) => None,
    }
}

/// Whether `path` is the root of its volume: `/`, or a mount point such as
/// `/Volumes/USB`.
pub fn is_volume_root(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }
    mount_point(path)
}

/// Asks the mount table, since a device number change also happens at the
/// firmlinks into the Data volume (`/Users`, `/Applications`).
#[cfg(target_os = "macos")]
fn mount_point(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut stat) } != 0 {
        return false;
    }
    let on = unsafe { std::ffi::CStr::from_ptr(stat.f_mntonname.as_ptr()) };
    on.to_bytes() == path.as_os_str().as_bytes()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn mount_point(path: &Path) -> bool {
    path.parent().is_some_and(|parent| !crate::fileops::same_volume(path, parent))
}

#[cfg(not(unix))]
fn mount_point(_: &Path) -> bool {
    false
}

/// What dropping `sources` into the folder `target` does without keys
/// (`_FilesystemAdjustedDefaultEffect`), judged by the first item.
pub fn default_effect(sources: &[PathBuf], target: &Path) -> Effect {
    match sources.first() {
        None => Effect::Copy,
        Some(src) if is_volume_root(src) => Effect::Link,
        Some(src) if crate::fileops::same_volume(src, target) => Effect::Move,
        Some(_) => Effect::Copy,
    }
}

/// What dropping `sources` into the folder `target` does, with the effect
/// the keys force and the effects the source allows. `None` refuses the
/// drop.
pub fn drop_effect(sources: &[PathBuf], target: &Path, forced: Option<Effect>, allowed: Allowed) -> Option<Effect> {
    if sources.is_empty() || sources.iter().any(|src| target.starts_with(src)) {
        return None;
    }
    let effect = match forced {
        Some(effect) => allowed.contains(effect).then_some(effect)?,
        None => {
            let copy_or_move = (allowed.contains(Effect::Copy), allowed.contains(Effect::Move));
            match (default_effect(sources, target), copy_or_move) {
                (Effect::Move, (true, false)) => Effect::Copy,
                (Effect::Copy, (false, true)) => Effect::Move,
                (effect, _) if allowed.contains(effect) => effect,
                _ => [Effect::Copy, Effect::Move, Effect::Link].into_iter().find(|&e| allowed.contains(e))?,
            }
        }
    };
    let already_there = sources.iter().all(|src| src.parent() == Some(target));
    (effect != Effect::Move || !already_there).then_some(effect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn keys() {
        assert_eq!(windows_keys(false, false, false), None);
        assert_eq!(windows_keys(true, false, false), Some(Effect::Move));
        assert_eq!(windows_keys(false, true, false), Some(Effect::Copy));
        assert_eq!(windows_keys(true, true, false), Some(Effect::Link));
        assert_eq!(windows_keys(false, false, true), Some(Effect::Link));
        assert_eq!(windows_keys(false, true, true), Some(Effect::Link));
        assert_eq!(mac_keys(true, false), Some(Effect::Copy));
        assert_eq!(mac_keys(false, true), Some(Effect::Move));
        assert_eq!(mac_keys(true, true), Some(Effect::Link));
        assert_eq!(mac_keys(false, false), None);
    }

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("macsplorer-dragdrop-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/inner")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("a/file.txt"), "x").unwrap();
        dir
    }

    #[test]
    fn same_volume_moves_and_keys_override() {
        let dir = fixture("same");
        let src = vec![dir.join("a/file.txt")];
        let b = dir.join("b");
        assert_eq!(drop_effect(&src, &b, None, Allowed::ALL), Some(Effect::Move));
        assert_eq!(drop_effect(&src, &b, Some(Effect::Copy), Allowed::ALL), Some(Effect::Copy));
        assert_eq!(drop_effect(&src, &b, Some(Effect::Link), Allowed::ALL), Some(Effect::Link));
        // A source that only copies: the default follows, a forced move is refused.
        let copy_only = Allowed::of(&[Effect::Copy]);
        assert_eq!(drop_effect(&src, &b, None, copy_only), Some(Effect::Copy));
        assert_eq!(drop_effect(&src, &b, Some(Effect::Move), copy_only), None);
        assert_eq!(drop_effect(&src, &b, None, Allowed::of(&[Effect::Link])), Some(Effect::Link));
        assert_eq!(drop_effect(&src, &b, None, Allowed::NONE), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_drops_onto_themselves() {
        let dir = fixture("self");
        let a = dir.join("a");
        // Into itself or a folder inside it.
        assert_eq!(drop_effect(std::slice::from_ref(&a), &a, None, Allowed::ALL), None);
        assert_eq!(drop_effect(std::slice::from_ref(&a), &a.join("inner"), Some(Effect::Copy), Allowed::ALL), None);
        // Moving into the folder it's in does nothing; copying makes "- Copy".
        let file = vec![a.join("file.txt")];
        assert_eq!(drop_effect(&file, &a, None, Allowed::ALL), None);
        assert_eq!(drop_effect(&file, &a, Some(Effect::Copy), Allowed::ALL), Some(Effect::Copy));
        assert_eq!(drop_effect(&[], &a, None, Allowed::ALL), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn volume_roots_link() {
        assert!(is_volume_root(Path::new("/")));
        assert!(!is_volume_root(&std::env::temp_dir()));
        assert_eq!(default_effect(&[PathBuf::from("/")], &std::env::temp_dir()), Effect::Link);
    }
}
