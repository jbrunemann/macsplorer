//! AirLynx devices in Network Neighborhood, beside LocalSend's. The protocol
//! is the `airlynx-core` crate (the AirLynx project's core, copied in); this
//! is the glue: this computer's identity and favorites, the scan, the send
//! worker and what the progress dialog shows of it.
//!
//! AirLynx has no question for each transfer. A device takes files from the
//! devices it has added as favorites, so sending to one needs it to have
//! added this computer, and receiving from one needs this computer to have
//! added it. [`Air::toggle_favorite`] is the second half; the first is up to
//! the other device, and the discovery answer says whether it is done
//! (`Peer::mutual`). AirLynx sends files, not folders: a folder in the
//! selection is left out.

use std::collections::HashSet;
use std::io;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use airlynx_core::send;
use airlynx_core::{DeviceId, DeviceKind, Favorites, Identity, Peer, Profile};

use crate::progress_dialog::{Snapshot, Tracked};
use macsplorer_core::preferences;

/// Starts the key of an AirLynx device, so it can't be taken for a LocalSend
/// one (those are fingerprints or addresses).
pub const KEY_PREFIX: &str = "airlynx:";

/// How long one round of asking waits for answers, and the pause before the
/// next round (the AirLynx app asks every six seconds too).
const ASK_WAIT: Duration = Duration::from_millis(1500);
const ASK_PAUSE: Duration = Duration::from_millis(4500);
/// Folder inside the config folder with AirLynx's `device-id` and `favorites`.
const PROFILE_DIR: &str = "airlynx";

/// This computer as AirLynx devices know it, and who it trusts.
pub struct Air {
    pub me: Identity,
    pub favorites: Favorites,
    /// The favorites' ids, read when a device answers and when one is
    /// toggled, so drawing a row asks no file.
    favorite_ids: HashSet<DeviceId>,
}

impl Air {
    /// Loads (or creates) the identity next to `settings.json`; `name` is
    /// what other devices list this computer as. Tests use a fresh one in
    /// memory.
    pub fn load(name: &str) -> Self {
        let profile = if cfg!(test) { None } else { preferences::config_dir().and_then(|d| Profile::load(&d.join(PROFILE_DIR)).ok()) };
        let (id, favorites) = match profile {
            Some(p) => (p.id, p.favorites),
            None => (DeviceId::random(), Favorites::in_memory()),
        };
        let mut air = Self { me: Identity::new(id, name).with_kind(DeviceKind::Desktop), favorites, favorite_ids: HashSet::new() };
        air.refresh();
        air
    }

    /// Reads the favorites again (another window may have changed them).
    pub fn refresh(&mut self) {
        self.favorite_ids = self.favorites.list().into_iter().map(|f| f.id).collect();
    }

    pub fn is_favorite(&self, id: DeviceId) -> bool {
        self.favorite_ids.contains(&id)
    }

    /// Adds `peer` to the favorites, or takes it off; returns whether it is
    /// one now.
    pub fn toggle_favorite(&mut self, peer: &Peer) -> io::Result<bool> {
        let now = !self.is_favorite(peer.id);
        if now {
            self.favorites.add(peer.id, &peer.name, peer.kind)?;
        } else {
            self.favorites.remove(peer.id)?;
        }
        self.refresh();
        Ok(now)
    }
}

/// The key of a device in the rows, menus and drops.
pub fn key(peer: &Peer) -> String {
    format!("{KEY_PREFIX}{}", peer.id)
}

/// Keeps asking for AirLynx devices until dropped, and tells `found` about
/// each answer.
pub struct Scan {
    stop: Arc<AtomicBool>,
}

impl Scan {
    pub fn start(me: Identity, found: impl Fn(Peer) + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                for peer in airlynx_core::discover(&me, ASK_WAIT).unwrap_or_default() {
                    found(peer);
                }
                // A scan that is dropped stops within a tenth of a second.
                for _ in 0..ASK_PAUSE.as_millis() / 100 {
                    if flag.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        });
        Self { stop }
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// How a send went.
#[derive(Debug, PartialEq, Eq)]
pub struct Sent {
    pub files: usize,
    /// Files the other side said arrived damaged.
    pub failed: usize,
    /// Folders and other things that aren't files.
    pub skipped: usize,
}

/// Why a send didn't happen.
#[derive(Debug)]
pub enum SendError {
    Cancelled,
    /// Nothing but folders was selected.
    NoFiles,
    /// The other device hasn't added this computer to its favorites.
    NotFavorite,
    Io(io::Error),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Cancelled => f.write_str("cancelled"),
            SendError::NoFiles => f.write_str("AirLynx sends files, not folders"),
            SendError::NotFavorite => f.write_str("it hasn't added this computer to its favorites yet"),
            SendError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A send as the progress dialog sees it.
pub struct Sending {
    alias: String,
    from: PathBuf,
    state: Mutex<Snap>,
    cancelled: AtomicBool,
}

#[derive(Default)]
struct Snap {
    name: String,
    done: u64,
    total: u64,
}

impl Sending {
    pub fn new(alias: &str, from: PathBuf) -> Self {
        Self { alias: alias.to_owned(), from, state: Mutex::new(Snap::default()), cancelled: AtomicBool::new(false) }
    }
}

impl Tracked for Sending {
    fn snapshot(&self) -> Snapshot {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.name.is_empty() {
            return Snapshot { lines: ["Preparing...".into(), String::new()], ..Snapshot::default() };
        }
        let folder = self.from.parent().and_then(|p| p.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        Snapshot { done: s.done, total: s.total, lines: [s.name.clone(), format!("From '{folder}' to '{}'", self.alias)] }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Sends the files among `paths` to `peer`, blocking: run it on a worker.
pub fn send_to(peer: &Peer, me: &Identity, paths: &[PathBuf], shown: &Sending) -> Result<Sent, SendError> {
    let (files, others): (Vec<_>, Vec<_>) = paths.iter().cloned().partition(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()));
    if files.is_empty() {
        return Err(SendError::NoFiles);
    }
    let entries = send::prepare(files).map_err(SendError::Io)?;
    let total = entries.iter().map(|e| e.size).sum();
    let report = send::send_files(peer.addr, me, &entries, |p| {
        let mut s = shown.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.name != p.file_name {
            s.name.clone_from(&p.file_name);
        }
        (s.done, s.total) = (p.total_sent, total);
        if shown.is_cancelled() { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
    });
    match report {
        Ok(r) => Ok(Sent { files: r.sent.len(), failed: r.failed.len(), skipped: others.len() }),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Err(SendError::Cancelled),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Err(SendError::NotFavorite),
        Err(e) => Err(SendError::Io(e)),
    }
}

/// What a device is, for tooltips: "computer", "phone or tablet".
pub fn kind_word(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Desktop => "computer",
        DeviceKind::Mobile => "phone or tablet",
        DeviceKind::Unknown => "device",
    }
}

/// A device's tooltip and status bar text: what it is, where, and which of
/// the two favorites (the one each side must add) are there.
pub fn describe(peer: &Peer, favorite: bool) -> String {
    let trust = match (favorite, peer.mutual) {
        (true, true) => "favorites on both sides: files can go either way",
        (true, false) => "it can send files here, but hasn't added this computer, so this one can't send to it",
        (false, true) => "this computer can send to it, but it can't send here until you add it to your favorites",
        (false, false) => "not a favorite on either side yet",
    };
    format!("{}: {} at {}, AirLynx; {trust}", peer.name, kind_word(peer.kind), peer.addr.ip())
}

/// Marks a file that arrived as downloaded (`com.apple.quarantine`), so
/// Gatekeeper checks it, as LocalSend's receiver does.
pub fn quarantine(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        // Flags 0081: downloaded, not yet opened (what Safari writes).
        let value = format!("0081;{secs:08x};Macsplorer;");
        let _ = macsplorer_core::xattr::set(path, c"com.apple.quarantine", value.as_bytes());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(mutual: bool) -> Peer {
        Peer { addr: "192.168.1.9:47800".parse().unwrap(), id: DeviceId::random(), name: "Pixel".into(), mutual, kind: DeviceKind::Mobile }
    }

    #[test]
    fn favorites_toggle() {
        let mut air = Air::load("Mac");
        let p = peer(false);
        assert!(!air.is_favorite(p.id));
        assert!(air.toggle_favorite(&p).unwrap());
        assert!(air.is_favorite(p.id) && air.favorites.contains(p.id));
        assert!(!air.toggle_favorite(&p).unwrap());
        assert!(!air.is_favorite(p.id));
    }

    #[test]
    fn keys_are_told_apart_from_localsends() {
        let p = peer(true);
        assert!(key(&p).starts_with(KEY_PREFIX) && key(&p).ends_with(&p.id.to_string()));
    }

    #[test]
    fn the_tooltip_says_what_is_missing() {
        let p = peer(false);
        assert!(describe(&p, true).contains("can't send to it"));
        assert!(describe(&peer(true), false).contains("add it to your favorites"));
        assert!(describe(&p, false).contains("neither side") || describe(&p, false).contains("either side"));
        assert!(describe(&peer(true), true).contains("both sides"));
    }

    /// A real receiver on this computer: a favorite's file arrives, a
    /// folder in the selection is left out, and a stranger is turned away.
    /// Skips quietly where the AirLynx ports are taken (the AirLynx app).
    #[test]
    fn files_go_to_a_favorite_and_folders_stay() {
        let dir = std::env::temp_dir().join(format!("macsplorer-airlynx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        std::fs::write(dir.join("hello.txt"), b"hello").unwrap();
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let (me, receiver) = (Identity::new(DeviceId::random(), "Mac"), Identity::new(DeviceId::random(), "Receiver"));
        let favorites = Favorites::in_memory();
        let config = airlynx_core::receive::ReceiverConfig {
            dir: dir.join("in"),
            port: 0,
            identity: receiver,
            favorites: favorites.clone(),
            poll_interval: Duration::from_millis(20),
        };
        let Ok(handle) = airlynx_core::receive::start(config, |_| {}) else { return };
        let mut target = peer(true);
        target.addr = format!("127.0.0.1:{}", handle.port()).parse().unwrap();
        let paths = [dir.join("hello.txt"), dir.join("folder")];

        let shown = Sending::new("Receiver", dir.clone());
        assert!(matches!(send_to(&target, &me, &paths, &shown), Err(SendError::NotFavorite)));

        favorites.add(me.id, "Mac", DeviceKind::Desktop).unwrap();
        let shown = Sending::new("Receiver", dir.clone());
        assert_eq!(send_to(&target, &me, &paths, &shown).unwrap(), Sent { files: 1, failed: 0, skipped: 1 });
        assert_eq!(shown.snapshot().lines[0], "hello.txt");
        assert_eq!((shown.snapshot().done, shown.snapshot().total), (5, 5));
        assert_eq!(std::fs::read(dir.join("in/hello.txt")).unwrap(), b"hello");

        shown.cancel();
        assert!(matches!(send_to(&target, &me, &paths, &shown), Err(SendError::Cancelled)));
        drop(handle);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_alone_is_not_sent() {
        let dir = std::env::temp_dir();
        let shown = Sending::new("Pixel", dir.clone());
        let me = Identity::new(DeviceId::random(), "Mac");
        assert!(matches!(send_to(&peer(true), &me, &[dir], &shown), Err(SendError::NoFiles)));
    }
}
