//! AirLynx in Network Neighborhood: its row, the devices found under it,
//! and everything that talks to `airlynx-core` (the AirLynx project's core,
//! copied in). Nothing here runs until the row is expanded (or a Send To
//! menu asks for the devices), and it knows nothing of LocalSend:
//! [`super::Network`] only routes by device key.
//!
//! Expanding the row starts a [`Scan`], which asks for devices every few
//! seconds while the row stays expanded; collapsing stops it.
//!
//! AirLynx has no question for each transfer. A device takes files from the
//! devices it has added as favorites, so sending to one needs it to have
//! added this computer, and receiving from one needs this computer to have
//! added it. [`AirLynx::toggle_favorite`] is the second half; the first is up
//! to the other device, and the discovery answer says whether it is done
//! (`Peer::mutual`). AirLynx sends files, not folders: a folder in the
//! selection is left out.
//!
//! The receiving half (the listener on port 47800) is the
//! [`super::receiving`] worker's; what it reports arrives through
//! [`AirLynx::event`].

use std::collections::{HashMap, HashSet};
use std::io;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use airlynx_core::receive::ReceiveEvent;
use airlynx_core::send;
use airlynx_core::{DeviceId, DeviceKind, Favorites, Identity, Peer, Profile};
use eframe::egui;
use egui::{RichText, Ui};

use super::{NetAction, Node, RowIcon, Rows, Service, Shared, device_dialog, folder_name, load_png, note_row, now, properties_item, receiving, section_menu, sentence};
use crate::app::{Guide, Status};
use crate::progress_dialog::{Kind, ProgressDialog, Snapshot, Tracked};
use crate::theme;
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

/// An AirLynx send's outcome, tagged with its dialog and the device's name.
type SendDone = (u64, String, Result<Sent, SendError>);

/// When a device was first and last heard from.
struct Seen {
    first: SystemTime,
    last: SystemTime,
}

/// AirLynx as Network Neighborhood has it: this computer's identity and
/// favorites, the devices found, the scan for them and the sends under way.
pub struct AirLynx {
    pub me: Identity,
    pub favorites: Favorites,
    /// The favorites' ids, read when a device answers and when one is
    /// toggled, so drawing a row asks no file.
    favorite_ids: HashSet<DeviceId>,
    peers: Vec<Peer>,
    expanded: bool,
    scan: Option<Scan>,
    /// When the scan started, for "Searching..."
    scan_started: f64,
    /// A scan started by the menu stops then, unless the row is expanded.
    scan_until: f64,
    found_tx: Sender<Peer>,
    found_rx: Receiver<Peer>,
    done_tx: Sender<SendDone>,
    done_rx: Receiver<SendDone>,
    /// Files favorites sent in this session, for the status bar.
    saved: usize,
    /// When each device (by key) was first and last heard from, kept for the
    /// session even when Search Again forgets the device.
    seen: HashMap<String, Seen>,
    /// Tests: never touch the network.
    offline: bool,
    logo: egui::TextureHandle,
    ctx: egui::Context,
}

impl AirLynx {
    /// Loads (or creates) the identity next to `settings.json`; `name` is
    /// what other devices list this computer as. Tests use a fresh one in
    /// memory.
    pub fn new(ctx: egui::Context, name: &str) -> Self {
        let profile = if cfg!(test) { None } else { preferences::config_dir().and_then(|d| Profile::load(&d.join(PROFILE_DIR)).ok()) };
        let (id, favorites) = match profile {
            Some(p) => (p.id, p.favorites),
            None => (DeviceId::random(), Favorites::in_memory()),
        };
        let (found_tx, found_rx) = channel();
        let (done_tx, done_rx) = channel();
        let mut air = Self {
            me: Identity::new(id, name).with_kind(DeviceKind::Desktop),
            favorites,
            favorite_ids: HashSet::new(),
            peers: Vec::new(),
            expanded: false,
            scan: None,
            scan_started: 0.0,
            scan_until: 0.0,
            found_tx,
            found_rx,
            done_tx,
            done_rx,
            saved: 0,
            seen: HashMap::new(),
            offline: false,
            logo: load_png(&ctx, "airlynx-logo", include_bytes!("../../../../assets/airlynx/logo-32.png")),
            ctx,
        };
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
    fn set_favorite(&mut self, peer: &Peer) -> io::Result<bool> {
        let now = !self.is_favorite(peer.id);
        if now {
            self.favorites.add(peer.id, &peer.name, peer.kind)?;
        } else {
            self.favorites.remove(peer.id)?;
        }
        self.refresh();
        Ok(now)
    }

    /// The device `key`'s entry in the favorites menu item: add it, or take
    /// it off, and say so.
    pub fn toggle_favorite(&mut self, key: &str, shared: &mut Shared) {
        let Some(peer) = self.find(key).cloned() else { return };
        shared.say(match self.set_favorite(&peer) {
            Ok(true) if peer.mutual => Status::note(format!("{} is a favorite now: files can go either way", peer.name)),
            Ok(true) => Status::note(format!("{} is a favorite now and can send files here. To send to it, it has to add this computer too.", peer.name)),
            Ok(false) => Status::note(format!("{} is no longer a favorite, so it can't send files here", peer.name)),
            Err(e) => Status::error(format!("Couldn't change the favorites: {e}")),
        });
    }

    fn find(&self, key: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| self::key(p) == key)
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    pub fn logo(&self) -> egui::TextureId {
        self.logo.id()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Tests: shows `peers` as found, without scanning.
    #[cfg(test)]
    pub fn fake(&mut self, peers: impl IntoIterator<Item = Peer>) {
        self.offline = true;
        self.expanded = true;
        for p in peers {
            self.saw(key(&p));
            self.peers.push(p);
        }
    }

    // ---- the search -----------------------------------------------------

    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// No scan runs, and one could.
    pub fn idle(&self) -> bool {
        self.scan.is_none() && !self.offline
    }

    /// A scan the Send To menu asked for stops at `until`, unless the row
    /// is expanded by then.
    pub fn keep_until(&mut self, until: f64) {
        self.scan_until = until;
    }

    /// Starts asking for devices, if that isn't going on.
    pub fn ensure_scan(&mut self) {
        if self.idle() {
            let (tx, ctx) = (self.found_tx.clone(), self.ctx.clone());
            self.scan = Some(Scan::start(self.me.clone(), move |p| {
                if tx.send(p).is_ok() {
                    ctx.request_repaint();
                }
            }));
            self.scan_started = now(&self.ctx);
        }
    }

    /// "Search Again": forgets the devices and asks anew.
    pub fn rescan(&mut self) {
        self.peers.clear();
        self.scan = None;
        self.expanded = true;
        self.ensure_scan();
    }

    pub fn stop(&mut self) {
        self.scan = None;
    }

    /// Whether a scan is young enough that an empty list means "not yet".
    pub fn searching(&self) -> bool {
        self.scan.is_some() && now(&self.ctx) - self.scan_started < super::SEARCH_SECONDS
    }

    /// Notes that the device `key` was just heard from.
    fn saw(&mut self, key: String) {
        let now = SystemTime::now();
        self.seen.entry(key).and_modify(|s| s.last = now).or_insert(Seen { first: now, last: now });
    }

    // ---- what the workers tell ------------------------------------------

    /// Takes in what the workers found and finished, and stops a scan that
    /// is no longer wanted (`visible`: the row can be seen). Call once a frame.
    pub fn poll(&mut self, visible: bool, shared: &mut Shared) {
        let mut found = false;
        while let Ok(peer) = self.found_rx.try_recv() {
            self.saw(key(&peer));
            self.peers.retain(|p| p.id != peer.id);
            self.peers.push(peer);
            found = true;
        }
        if found {
            // A favorite may have been added or removed in another window.
            self.refresh();
        }
        while let Ok((id, name, result)) = self.done_rx.try_recv() {
            shared.finish(id);
            shared.say(match result {
                Ok(Sent { files, failed: 0, skipped: 0 }) => Status::note(format!("Sent {files} file(s) to {name}")),
                Ok(Sent { files, failed, skipped }) => {
                    let mut text = format!("Sent {files} file(s) to {name}");
                    if failed > 0 {
                        text.push_str(&format!("; {failed} arrived damaged and were refused"));
                    }
                    if skipped > 0 {
                        text.push_str(&format!("; {skipped} folder(s) weren't sent, AirLynx sends files only"));
                    }
                    if failed > 0 { Status::error(text) } else { Status::note(text) }
                }
                Err(SendError::Cancelled) => Status::note(format!("Sending to {name} was cancelled")),
                Err(e) => Status::error(format!("Couldn't send to {name}: {e}")),
            });
        }
        if self.scan.is_some() && !visible && now(&self.ctx) > self.scan_until {
            self.scan = None;
        }
        if self.searching() {
            self.ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// The receiver couldn't start, and why.
    pub fn unavailable(&self, why: &str, shared: &mut Shared) {
        shared.say(Status::error(format!("AirLynx devices can't send files here: {why}")));
    }

    /// What a favorite sends in, as it happens: the status bar says which
    /// file is coming and how it ended.
    pub fn event(&mut self, event: ReceiveEvent, shared: &mut Shared) {
        let folder = folder_name(&receiving::downloads());
        let peer_at = |peers: &[Peer], peer: &str| {
            let ip = peer.rsplit_once(':').map_or(peer, |(ip, _)| ip).to_owned();
            peers.iter().find(|p| p.addr.ip().to_string() == ip).cloned()
        };
        let name_at = |peers: &[Peer], peer: &str| peer_at(peers, peer).map_or_else(|| "a device".to_owned(), |p| p.name);
        let peers = &self.peers;
        let status = match event {
            ReceiveEvent::Refused { name, .. } => Status::error(format!(
                "{name} tried to send files, but isn't one of your AirLynx favorites. Add it to the favorites under AirLynx in Network Neighborhood first."
            )),
            ReceiveEvent::Connected { peer, files } => Status::note(format!("Receiving {files} file(s) from {}...", name_at(peers, &peer))),
            ReceiveEvent::FileStarted { peer, name, .. } => Status::note(format!("Receiving '{name}' from {}...", name_at(peers, &peer))),
            ReceiveEvent::FileProgress { peer, name, received, size } => {
                let percent = (received * 100).checked_div(size).unwrap_or(100);
                Status::note(format!("Receiving '{name}' from {}: {percent}%", name_at(peers, &peer)))
            }
            ReceiveEvent::FileSaved { peer, path, .. } => {
                quarantine(&path);
                let sender = peer_at(peers, &peer);
                shared.got(Service::AirLynx, sender.as_ref().map(key), sender.map_or_else(|| "a device".to_owned(), |p| p.name), path.clone());
                self.saved += 1;
                Status::note(format!("Received {} in {folder}", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())))
            }
            ReceiveEvent::FileRejected { peer, name, reason } => Status::error(format!("'{name}' from {} was refused: {reason}", name_at(peers, &peer))),
            ReceiveEvent::SessionEnded { peer, error: Some(e) } => Status::error(format!("Receiving from {} stopped: {e}", name_at(peers, &peer))),
            ReceiveEvent::SessionEnded { peer, error: None } => {
                let n = std::mem::take(&mut self.saved);
                if n == 0 {
                    return;
                }
                Status::note(format!("Received {n} file(s) from {} in {folder}", name_at(peers, &peer)))
            }
        };
        shared.say(status);
    }

    // ---- doing things ---------------------------------------------------

    /// Sends the files among `paths` to the device `key` on a worker thread:
    /// no question to wait for, so the dialog shows the files going at once.
    pub fn send(&mut self, key: &str, paths: Vec<PathBuf>, shared: &mut Shared) {
        let Some(peer) = self.find(key).cloned() else { return };
        let id = shared.next_id();
        let progress = Arc::new(Sending::new(&peer.name, paths[0].clone()));
        shared.dialogs.push(ProgressDialog::new(id, progress.clone(), Kind::Send));
        shared.say(Status::note(format!("Sending to {}...", peer.name)));
        let (me, tx, ctx) = (self.me.clone(), self.done_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = send_to(&peer, &me, &paths, &progress);
            if tx.send((id, peer.name.clone(), result)).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// A listed device's name.
    pub fn alias(&self, key: &str) -> Option<String> {
        self.find(key).map(|p| p.name.clone())
    }

    /// A device's tooltip and status bar text.
    pub fn describe(&self, key: &str) -> Option<String> {
        self.find(key).map(|p| describe(p, self.is_favorite(p.id)))
    }

    /// What a device's Properties window shows: where it is, who it is, how
    /// the favorites stand and when it was seen.
    pub fn view(&self, key: &str) -> Option<device_dialog::View> {
        let p = self.find(key)?;
        let when = |t: Option<SystemTime>| t.map_or_else(String::new, |t| crate::locale::long_date_time(Some(t), true));
        let seen = self.seen.get(key);
        let yes_no = |b: bool| if b { "Yes" } else { "No" };
        let favorite = self.is_favorite(p.id);
        Some(device_dialog::View {
            name: p.name.clone(),
            groups: vec![
                vec![("Type:", sentence(kind_word(p.kind))), ("ID:", p.id.to_string()), ("Short ID:", p.id.short())],
                vec![("Address:", p.addr.ip().to_string()), ("Port:", p.addr.port().to_string()), ("Protocol:", "AirLynx".to_owned())],
                vec![
                    ("Favorite:", format!("{}: {}", yes_no(favorite), if favorite { "it can send files here" } else { "it can't send files here" })),
                    ("Added you:", format!("{}: {}", yes_no(p.mutual), if p.mutual { "this computer can send to it" } else { "this computer can't send to it" })),
                ],
                vec![("First seen:", when(seen.map(|s| s.first))), ("Last seen:", when(seen.map(|s| s.last)))],
            ],
        })
    }

    /// Every device found, by name.
    fn sorted(&self) -> Vec<&Peer> {
        let mut all: Vec<&Peer> = self.peers.iter().collect();
        all.sort_by_cached_key(|p| p.name.to_lowercase());
        all
    }

    // ---- drawing --------------------------------------------------------

    /// The AirLynx row and, expanded, its devices or a note that there are
    /// none. `last`: no row follows it at its level.
    pub fn tree(&self, ui: &mut Ui, rows: &mut Rows, last: bool) {
        let guide = Guide { ancestors: vec![false], has_next: !last, has_prev: true };
        let resp = rows.branch(ui, 1, &guide, RowIcon::Image(self.logo.id()), "AirLynx", Node::Service(Service::AirLynx), self.expanded);
        section_menu(&resp, Service::AirLynx, self.expanded, rows.actions);
        if !self.expanded {
            return;
        }
        let found = self.sorted();
        let ancestors = vec![false, !last];
        if found.is_empty() {
            let text = if self.searching() { "Searching..." } else { "No devices found" };
            note_row(ui, 2, &Guide { ancestors, has_next: false, has_prev: true }, text);
            return;
        }
        for (i, peer) in found.iter().enumerate() {
            let dkey = key(peer);
            let favorite = self.is_favorite(peer.id);
            let guide = Guide { ancestors: ancestors.clone(), has_next: i + 1 < found.len(), has_prev: true };
            let resp = rows.device(ui, 2, &guide, &dkey, &peer.name, self.logo.id(), describe(peer, favorite));
            let can_send = rows.can_send;
            let actions = &mut *rows.actions;
            egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| {
                let send = egui_classic_menu::item(ui, RichText::new("Send Selected Items").font(theme::bold(theme::FONT_SIZE)), None);
                if ui.add_enabled(can_send, send).clicked() {
                    actions.push(NetAction::SendSelection(dkey.clone()));
                }
                let label = if favorite { "Remove from Favorites" } else { "Add to Favorites" };
                if ui.add(egui_classic_menu::item(ui, label, None)).clicked() {
                    actions.push(NetAction::ToggleFavorite(dkey.clone()));
                    ui.close();
                }
                properties_item(ui, &dkey, actions);
            });
        }
    }

    /// The Send To submenu's items: a device each.
    pub fn send_to_items(&self, ui: &mut Ui, actions: &mut Vec<NetAction>) {
        for peer in self.sorted() {
            let button = egui::Button::image_and_text(egui::Image::new((self.logo.id(), egui::vec2(16.0, 16.0))), &peer.name);
            if ui.add(button).on_hover_text(describe(peer, self.is_favorite(peer.id))).clicked() {
                actions.push(NetAction::SendSelection(key(peer)));
            }
        }
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
        let mut air = AirLynx::new(egui::Context::default(), "Mac");
        let p = peer(false);
        assert!(!air.is_favorite(p.id));
        assert!(air.set_favorite(&p).unwrap());
        assert!(air.is_favorite(p.id) && air.favorites.contains(p.id));
        assert!(!air.set_favorite(&p).unwrap());
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
