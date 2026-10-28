//! Device identity and the persisted favorites list.
//!
//! Every install has a random `DeviceId`. A device only accepts files from
//! devices in its favorites, so two devices can exchange files in both
//! directions once each has added the other.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::MAX_DEVICE_NAME_LEN;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId([u8; 16]);

impl DeviceId {
    pub fn random() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let mut bytes = [0u8; 16];
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            // RandomState is seeded from the OS, which avoids a dependency just for this.
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u128(nanos);
            hasher.write_usize(i);
            chunk.copy_from_slice(&hasher.finish().to_be_bytes());
        }
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// First eight hex digits, enough to tell devices apart in a list.
    pub fn short(&self) -> String {
        self.to_string()[..8].to_string()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({self})")
    }
}

impl FromStr for DeviceId {
    type Err = io::Error;

    fn from_str(s: &str) -> io::Result<Self> {
        let bad = || io::Error::new(io::ErrorKind::InvalidData, format!("invalid device id {s:?}"));
        if s.len() != 32 || !s.is_ascii() {
            return Err(bad());
        }
        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
        }
        Ok(Self(bytes))
    }
}

/// What sort of device this is, shown as an icon next to its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceKind {
    #[default]
    Unknown,
    Desktop,
    Mobile,
}

impl DeviceKind {
    /// One letter on the wire (discovery replies).
    pub fn code(self) -> char {
        match self {
            DeviceKind::Unknown => '?',
            DeviceKind::Desktop => 'd',
            DeviceKind::Mobile => 'm',
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "d" => DeviceKind::Desktop,
            "m" => DeviceKind::Mobile,
            _ => DeviceKind::Unknown,
        }
    }

    fn word(self) -> &'static str {
        match self {
            DeviceKind::Unknown => "unknown",
            DeviceKind::Desktop => "desktop",
            DeviceKind::Mobile => "mobile",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        [DeviceKind::Unknown, DeviceKind::Desktop, DeviceKind::Mobile].into_iter().find(|k| k.word() == word)
    }
}

/// Who we are on the network: the id plus the display name.
#[derive(Debug, Clone)]
pub struct Identity {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
}

impl Identity {
    pub fn new(id: DeviceId, name: &str) -> Self {
        Self { id, name: clean_name(name), kind: DeviceKind::Unknown }
    }

    pub fn with_kind(mut self, kind: DeviceKind) -> Self {
        self.kind = kind;
        self
    }
}

/// Makes a peer-supplied name safe to show and store: no control characters,
/// trimmed, at most `MAX_DEVICE_NAME_LEN` bytes, never empty.
pub fn clean_name(raw: &str) -> String {
    let mut name = String::new();
    for c in raw.chars().filter(|c| !c.is_control()) {
        if name.len() + c.len_utf8() > MAX_DEVICE_NAME_LEN {
            break;
        }
        name.push(c);
    }
    let name = name.trim();
    if name.is_empty() { "unknown".into() } else { name.into() }
}

#[derive(Debug, Clone)]
pub struct Favorite {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
}

/// The set of trusted devices. Cloning shares the same list, so the receiver
/// sees changes the UI makes immediately. Changes are written to disk.
#[derive(Debug, Clone)]
pub struct Favorites {
    inner: Arc<Mutex<FavoritesInner>>,
}

#[derive(Debug)]
struct FavoritesInner {
    path: Option<PathBuf>,
    list: Vec<Favorite>,
    /// Modification time of the file as last read or written, to notice edits by other processes.
    mtime: Option<SystemTime>,
}

impl Favorites {
    /// Loads the list from `path`; a missing file is an empty list. The list reloads
    /// itself when another process (the CLI, say) changes the file.
    pub fn load(path: PathBuf) -> io::Result<Self> {
        let mut inner = FavoritesInner { path: Some(path), list: Vec::new(), mtime: None };
        inner.reload()?;
        Ok(Self { inner: Arc::new(Mutex::new(inner)) })
    }

    /// A list that is never saved.
    pub fn in_memory() -> Self {
        let inner = FavoritesInner { path: None, list: Vec::new(), mtime: None };
        Self { inner: Arc::new(Mutex::new(inner)) }
    }

    pub fn contains(&self, id: DeviceId) -> bool {
        let mut inner = self.inner.lock().unwrap();
        inner.refresh();
        inner.list.iter().any(|f| f.id == id)
    }

    pub fn list(&self) -> Vec<Favorite> {
        let mut inner = self.inner.lock().unwrap();
        inner.refresh();
        inner.list.clone()
    }

    /// Adds a device, or updates its stored name and kind if it is already a favorite.
    /// An `Unknown` kind never replaces a known one. The change stays in memory even if saving fails.
    pub fn add(&self, id: DeviceId, name: &str, kind: DeviceKind) -> io::Result<()> {
        let name = clean_name(name);
        let mut inner = self.inner.lock().unwrap();
        inner.refresh();
        match inner.list.iter_mut().find(|f| f.id == id) {
            Some(existing) => {
                let kind = if kind == DeviceKind::Unknown { existing.kind } else { kind };
                if existing.name == name && existing.kind == kind {
                    return Ok(());
                }
                existing.name = name;
                existing.kind = kind;
            }
            None => inner.list.push(Favorite { id, name, kind }),
        }
        inner.save()
    }

    pub fn remove(&self, id: DeviceId) -> io::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.refresh();
        inner.list.retain(|f| f.id != id);
        inner.save()
    }
}

impl FavoritesInner {
    fn file_mtime(&self) -> Option<SystemTime> {
        fs::metadata(self.path.as_ref()?).and_then(|m| m.modified()).ok()
    }

    fn reload(&mut self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        self.list = match fs::read_to_string(path) {
            Ok(text) => text
                .lines()
                .filter_map(|line| {
                    let (id, rest) = line.split_once(' ')?;
                    // Lines are `<id> kind:<kind> <name>`; older files have `<id> <name>`.
                    let (kind, name) = match rest.split_once(' ') {
                        Some((word, name)) => match word.strip_prefix("kind:").and_then(DeviceKind::from_word) {
                            Some(kind) => (kind, name),
                            None => (DeviceKind::Unknown, rest),
                        },
                        None => (DeviceKind::Unknown, rest),
                    };
                    Some(Favorite { id: id.parse().ok()?, name: clean_name(name), kind })
                })
                .collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        self.mtime = self.file_mtime();
        Ok(())
    }

    /// Picks up changes made to the file by someone else. Keeps the old list if it cannot be read.
    fn refresh(&mut self) {
        if self.path.is_some() && self.file_mtime() != self.mtime {
            let _ = self.reload();
        }
    }

    fn save(&mut self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let text: String = self.list.iter().map(|f| format!("{} kind:{} {}\n", f.id, f.kind.word(), f.name)).collect();
        write_atomic(path, &text)?;
        self.mtime = self.file_mtime();
        Ok(())
    }
}

/// This device's id and favorites, stored in one directory.
pub struct Profile {
    pub id: DeviceId,
    pub favorites: Favorites,
}

impl Profile {
    /// Loads the profile from `dir`, creating the directory and a new device id on first use.
    pub fn load(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let id_path = dir.join("device-id");
        let id = match fs::read_to_string(&id_path).ok().and_then(|s| s.trim().parse().ok()) {
            Some(id) => id,
            None => {
                let id = DeviceId::random();
                write_atomic(&id_path, &format!("{id}\n"))?;
                id
            }
        };
        Ok(Self { id, favorites: Favorites::load(dir.join("favorites"))? })
    }
}

fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_hex_round_trip() {
        let id = DeviceId::random();
        assert_eq!(id.to_string().len(), 32);
        assert_eq!(id.to_string().parse::<DeviceId>().unwrap(), id);
        assert!("xyz".parse::<DeviceId>().is_err());
        assert!("g".repeat(32).parse::<DeviceId>().is_err());
        assert_ne!(DeviceId::random(), DeviceId::random());
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Pixel\n3a "), "Pixel3a");
        assert_eq!(clean_name(""), "unknown");
        assert_eq!(clean_name(&"a".repeat(200)).len(), MAX_DEVICE_NAME_LEN);
        assert!(clean_name(&"\u{e9}".repeat(100)).len() <= MAX_DEVICE_NAME_LEN);
    }

    #[test]
    fn edits_from_another_handle_are_seen() {
        let dir = std::env::temp_dir().join(format!("airlynx-test-{}", DeviceId::random()));
        let a = Profile::load(&dir).unwrap().favorites;
        let b = Profile::load(&dir).unwrap().favorites;
        let friend = DeviceId::random();
        assert!(!a.contains(friend));
        b.add(friend, "Phone", DeviceKind::Mobile).unwrap();
        // Same-second writes can share a modification time on some filesystems.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(a.contains(friend));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn profile_persists() {
        let dir = std::env::temp_dir().join(format!("airlynx-test-{}", DeviceId::random()));
        let first = Profile::load(&dir).unwrap();
        let friend = DeviceId::random();
        first.favorites.add(friend, "Mac mini", DeviceKind::Desktop).unwrap();
        first.favorites.add(friend, "Mac mini 2", DeviceKind::Unknown).unwrap();

        let second = Profile::load(&dir).unwrap();
        assert_eq!(second.id, first.id);
        let list = second.favorites.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Mac mini 2");
        // An unknown kind does not erase a known one.
        assert_eq!(list[0].kind, DeviceKind::Desktop);

        second.favorites.remove(friend).unwrap();
        assert!(!Profile::load(&dir).unwrap().favorites.contains(friend));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn old_favorites_files_still_load() {
        let dir = std::env::temp_dir().join(format!("airlynx-test-{}", DeviceId::random()));
        fs::create_dir_all(&dir).unwrap();
        let (a, b) = (DeviceId::random(), DeviceId::random());
        fs::write(dir.join("favorites"), format!("{a} Old Mac\n{b} kind:mobile Pixel 3a\n")).unwrap();
        let list = Profile::load(&dir).unwrap().favorites.list();
        assert_eq!((list[0].name.as_str(), list[0].kind), ("Old Mac", DeviceKind::Unknown));
        assert_eq!((list[1].name.as_str(), list[1].kind), ("Pixel 3a", DeviceKind::Mobile));
        let _ = fs::remove_dir_all(dir);
    }
}
