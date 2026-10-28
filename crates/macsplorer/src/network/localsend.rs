//! LocalSend in Network Neighborhood: its row, the devices found under it,
//! and everything that talks to the `localsend` crate. Nothing here runs
//! until the row is expanded (or a Send To menu asks for the devices), and
//! it knows nothing of AirLynx: [`super::Network`] only routes by device
//! key.
//!
//! Expanding the row starts a scan ([`Scan`]), which goes on listening while
//! it stays expanded; collapsing stops it. The search asks the addresses
//! remembered from before and their /24s, then our /24 and the multicast
//! group, as LocalSend does; only when that finds no device does it go
//! through the whole network. "Search Again" starts over and goes through
//! the whole network at once. Items dropped on a device, or sent with Send
//! To, go there: the device asks its user to accept, and the dialog waits
//! until they do.
//!
//! The receiving half (the server that other devices send to, and the
//! question it asks) is the [`super::receiving`] worker's; what it reports
//! arrives through [`LocalSend::event`].

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use ::localsend::discovery::{Options, Scan, Sweep, SweepProgress, local_ipv4};
use ::localsend::receive::{self, Closed, End, Event, Offer, Received};
use ::localsend::send::{self, Progress, SendError, Sent, Stage};
use ::localsend::{Device, DeviceType, Devices, Me};
use eframe::egui;
use egui::{RichText, Ui};

use super::offer_dialog::{self, View as OfferView};
use super::{NetAction, Node, RowIcon, Rows, Service, Shared, device_dialog, folder_name, load_png, note_row, now, properties_item, receiving, section_menu, sentence};
use crate::app::{Guide, Status};
use crate::progress_dialog::{Kind, ProgressDialog, Snapshot, Tracked};
use crate::theme;
use macsplorer_core::preferences::Preferences;

/// The identity kept between runs, so other devices know this computer
/// again: next to `settings.json`.
#[cfg(all(feature = "localsend-tls", not(test)))]
const IDENTITY_FILE: &str = "localsend-identity.pem";

/// A send's outcome, tagged with its dialog.
type SendDone = (u64, String, Result<Sent, SendError>);

/// The outcome of telling a device about this computer, with its name.
type Introduced = (String, Result<(), SendError>);

pub struct LocalSend {
    me: Me,
    devices: Devices,
    expanded: bool,
    scan: Option<Scan>,
    /// When the scan started, for "Searching..."
    scan_started: f64,
    /// A scan started by the menu stops then, unless the row is expanded.
    scan_until: f64,
    found_tx: Sender<Device>,
    found_rx: Receiver<Device>,
    /// Where devices were found before, from the settings; the next scan
    /// asks there first.
    remembered: Vec<Ipv4Addr>,
    done_tx: Sender<SendDone>,
    done_rx: Receiver<SendDone>,
    introduced_tx: Sender<Introduced>,
    introduced_rx: Receiver<Introduced>,
    /// The offer being asked, with its dialog's id and what it shows.
    offer: Option<(u64, Offer, OfferView)>,
    /// The fingerprint of the device whose accepted files are coming in (one
    /// transfer at a time), for telling whose files they were.
    sender: Option<String>,
    /// Tests: never touch the network.
    offline: bool,
    logo: egui::TextureHandle,
    ctx: egui::Context,
}

impl LocalSend {
    /// `alias` is the name other devices list this computer under.
    pub fn new(ctx: egui::Context, alias: &str) -> Self {
        let (found_tx, found_rx) = channel();
        let (done_tx, done_rx) = channel();
        let (introduced_tx, introduced_rx) = channel();
        Self {
            me: Me::with_identity(alias, Some("Macsplorer".into()), identity()),
            devices: Devices::default(),
            expanded: false,
            scan: None,
            scan_started: 0.0,
            scan_until: 0.0,
            found_tx,
            found_rx,
            remembered: if cfg!(test) { Vec::new() } else { Preferences::load().devices },
            done_tx,
            done_rx,
            introduced_tx,
            introduced_rx,
            offer: None,
            sender: None,
            offline: false,
            logo: load_png(&ctx, "localsend-logo", include_bytes!("../../../../assets/localsend/logo-32.png")),
            ctx,
        }
    }

    /// This computer as LocalSend devices know it.
    pub fn me(&self) -> &Me {
        &self.me
    }

    /// The receiver is up: announce the port and protocol it took.
    pub fn listening(&mut self, me: &Me) {
        self.me.port = me.port;
        self.me.protocol = me.protocol;
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
        self.devices.is_empty()
    }

    /// Tests: shows `devices` as found, without scanning.
    #[cfg(test)]
    pub fn fake(&mut self, devices: impl IntoIterator<Item = Device>, expanded: bool) {
        self.offline = true;
        self.expanded = expanded;
        for d in devices {
            self.devices.insert(d);
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

    /// Starts the usual search if none is going: the remembered addresses
    /// and their /24s, our /24 and the multicast group. The whole network
    /// follows only when that found nothing ([`LocalSend::poll`]).
    /// `announce`: this computer takes files, so the scan says so.
    pub fn ensure_scan(&mut self, announce: bool) {
        self.start_scan(Sweep::Near, announce);
    }

    /// "Search Again": forgets the devices and goes through the whole network.
    pub fn rescan(&mut self, announce: bool) {
        self.devices.clear();
        self.scan = None;
        self.expanded = true;
        self.start_scan(Sweep::All, announce);
    }

    pub fn stop(&mut self) {
        self.scan = None;
    }

    /// Stop asking addresses, from the search note's menu.
    pub fn cancel_search(&self) {
        if let Some(scan) = &self.scan {
            scan.cancel_sweep();
        }
    }

    fn start_scan(&mut self, sweep: Sweep, announce: bool) {
        if self.idle() {
            let (tx, ctx) = (self.found_tx.clone(), self.ctx.clone());
            let options = Options { announce, sweep, first: self.remembered.clone() };
            self.scan = Some(Scan::start(self.me.clone(), options, move |d| {
                if tx.send(d).is_ok() {
                    ctx.request_repaint();
                }
            }));
            self.scan_started = now(&self.ctx);
        }
    }

    /// Whether a scan is young enough that an empty list means "not yet".
    pub fn searching(&self) -> bool {
        self.scan.as_ref().is_some_and(|s| s.sweeping() || now(&self.ctx) - self.scan_started < super::SEARCH_SECONDS)
    }

    /// What the search says while it goes: the part of the network being
    /// asked and how far along it is.
    pub fn searching_text(&self) -> String {
        self.scan.as_ref().and_then(Scan::progress).map_or_else(|| "Searching...".to_owned(), |p| progress_text(&p))
    }

    /// Keeps the address of a device found over IPv4 in the settings, so the
    /// next scan looks there first. Addresses of this computer (a LocalSend
    /// app on it) aren't worth keeping.
    fn remember(&mut self, ip: IpAddr) {
        let IpAddr::V4(ip) = ip else { return };
        if cfg!(test) || self.remembered.first() == Some(&ip) || local_ipv4().iter().any(|i| i.ip == ip) {
            return;
        }
        self.remembered.retain(|d| *d != ip);
        self.remembered.insert(0, ip);
        Preferences::update(|p| {
            p.remember_device(ip);
        });
    }

    // ---- what the workers tell ------------------------------------------

    /// Takes in what the workers found and finished, and stops a scan that
    /// is no longer wanted (`visible`: the row can be seen). Call once a frame.
    pub fn poll(&mut self, visible: bool, shared: &mut Shared) {
        let now = now(&self.ctx);
        while let Ok(device) = self.found_rx.try_recv() {
            self.remember(device.ip);
            self.devices.insert(device);
        }
        if let Some((_, offer, view)) = &self.offer
            && let Some(closed) = offer.closed()
        {
            shared.say(Status::note(match closed {
                Closed::Withdrawn => format!("{} took back the files it offered", view.from),
                Closed::Expired => format!("The files {} offered weren't accepted in time", view.from),
            }));
            self.offer = None;
        }
        while let Ok((alias, result)) = self.introduced_rx.try_recv() {
            shared.say(match result {
                Ok(()) => Status::note(format!("{alias} can now find this computer")),
                Err(e) => Status::error(format!("Couldn't tell {alias} about this computer: {e}")),
            });
        }
        while let Ok((id, alias, result)) = self.done_rx.try_recv() {
            shared.finish(id);
            shared.say(match result {
                Ok(Sent { files, declined: 0 }) => Status::note(format!("Sent {files} file(s) to {alias}")),
                Ok(Sent { files, declined }) => Status::note(format!("Sent {files} file(s) to {alias}; {declined} weren't accepted")),
                Err(SendError::Cancelled) => Status::note(format!("Sending to {alias} was cancelled")),
                Err(e) => Status::error(format!("Couldn't send to {alias}: {e}")),
            });
        }
        if self.scan.is_some() && !visible && now > self.scan_until {
            self.scan = None;
        }
        // The near search is over and had a few seconds for the multicast
        // answers: with no device, ask the whole network.
        if self.devices.is_empty()
            && now - self.scan_started >= super::SEARCH_SECONDS
            && let Some(scan) = &mut self.scan
        {
            scan.broaden();
        }
        if self.searching() {
            self.ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    /// What the receiver reports: a device announced itself, one offers
    /// files, a transfer is over.
    pub fn event(&mut self, event: Event, shared: &mut Shared) {
        match event {
            Event::Device(device) => {
                if device.info.fingerprint != self.me.fingerprint {
                    self.devices.insert(device);
                }
            }
            Event::Offer(offer) => {
                if self.offer.is_some() {
                    offer.decline();
                    return;
                }
                let id = shared.next_id();
                let view = OfferView {
                    from: offer.from.alias.clone(),
                    detail: offer_detail(&offer),
                    files: offer.files.iter().map(|f| (f.file_name.clone(), f.size)).collect(),
                    folder: folder_name(&receiving::downloads()),
                };
                shared.say(Status::note(format!("{} wants to send you files", view.from)));
                // If Macsplorer is in the background, the Dock icon bounces.
                self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
                self.offer = Some((id, offer, view));
            }
            Event::Done(received) => self.received(received, shared),
        }
    }

    /// A transfer in is over: its dialog closes, the status bar says how it went.
    fn received(&mut self, r: Received, shared: &mut Shared) {
        for d in &mut shared.dialogs {
            d.finished |= d.kind() == Kind::Receive;
        }
        let folder = folder_name(&receiving::downloads());
        let key = self.sender.take().filter(|k| !k.is_empty());
        // A folder that came in is one item of the list, not all its files.
        let downloads = receiving::downloads();
        for path in &r.saved {
            let top = path.strip_prefix(&downloads).ok().and_then(|p| p.components().next()).map_or_else(|| path.clone(), |c| downloads.join(c));
            shared.got(Service::LocalSend, key.clone(), r.from.clone(), top);
        }
        let (n, from) = (r.saved.len(), &r.from);
        shared.say(match r.end {
            End::Finished if r.missing == 0 => Status::note(format!("Received {n} file(s) from {from} in {folder}")),
            End::Finished => Status::error(format!("Received {n} file(s) from {from} in {folder}; {} didn't arrive", r.missing)),
            End::Cancelled => Status::note(format!("Receiving from {from} was cancelled; {n} file(s) were saved in {folder}")),
            End::CancelledBySender => Status::note(format!("{from} cancelled sending; {n} file(s) were saved in {folder}")),
            End::TimedOut => Status::error(format!("{from} stopped sending; {n} file(s) were saved in {folder}")),
        });
    }

    /// The Receive Files question, while one is open.
    pub fn show_dialogs(&mut self, shared: &mut Shared) {
        let answer = self.offer.as_ref().and_then(|(id, _, view)| offer_dialog::show(&self.ctx, *id, view, self.logo.id()));
        if let Some(answer) = answer
            && let Some((id, offer, view)) = self.offer.take()
        {
            match answer {
                offer_dialog::Answer::Accept => {
                    self.sender = Some(offer.from.fingerprint.clone());
                    let progress = offer.accept();
                    let tracked = Arc::new(Receiving { progress, alias: view.from.clone(), folder: view.folder.clone() });
                    shared.dialogs.push(ProgressDialog::new(id, tracked, Kind::Receive));
                    shared.say(Status::note(format!("Receiving files from {}...", view.from)));
                }
                offer_dialog::Answer::Decline => {
                    offer.decline();
                    shared.say(Status::note(format!("Declined the files from {}", view.from)));
                }
            }
        }
    }

    // ---- doing things ---------------------------------------------------

    /// Sends `paths` to the device `key` on a worker thread, with the
    /// progress dialog.
    pub fn send(&mut self, key: &str, paths: Vec<PathBuf>, shared: &mut Shared) {
        let Some(device) = self.devices.get(key).cloned() else { return };
        let id = shared.next_id();
        let progress = Arc::new(Sending { progress: Progress::default(), alias: device.alias().to_owned(), from: paths[0].clone() });
        shared.dialogs.push(ProgressDialog::new(id, progress.clone(), Kind::Send));
        shared.say(Status::note(format!("Waiting for {} to accept...", device.alias())));
        let (me, tx, ctx) = (self.me.clone(), self.done_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = send::send(&me, &device, &paths, None, &progress.progress);
            if tx.send((id, device.alias().to_owned(), result)).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// Tells the device `key` about this computer on a worker thread, so it
    /// lists it, as it would after hearing our announcement.
    pub fn introduce(&mut self, key: &str, shared: &mut Shared) {
        let Some(device) = self.devices.get(key).cloned() else { return };
        shared.say(Status::note(format!("Telling {} about this computer...", device.alias())));
        let (me, tx, ctx) = (self.me.clone(), self.introduced_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = send::introduce(&me, &device);
            if tx.send((device.alias().to_owned(), result)).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    /// A listed device's name.
    pub fn alias(&self, key: &str) -> Option<String> {
        self.devices.get(key).map(|d| d.alias().to_owned())
    }

    /// A device's tooltip and status bar text.
    pub fn describe(&self, key: &str) -> Option<String> {
        self.devices.get(key).map(describe)
    }

    /// What a device's Properties window shows: where it is and who it is.
    pub fn view(&self, key: &str) -> Option<device_dialog::View> {
        let d = self.devices.get(key)?;
        let model = d.info.device_model.clone().unwrap_or_else(|| "Unknown".to_owned());
        Some(device_dialog::View {
            name: d.alias().to_owned(),
            groups: vec![
                vec![("Type:", sentence(kind_word(d.device_type()))), ("Model:", model)],
                vec![
                    ("Address:", d.ip.to_string()),
                    ("Port:", d.port.to_string()),
                    ("Protocol:", format!("LocalSend {}, {}", d.info.version, d.protocol.scheme().to_uppercase())),
                ],
            ],
        })
    }

    /// Every device found, by name.
    fn sorted(&self) -> Vec<&Device> {
        let mut all: Vec<&Device> = self.devices.iter().collect();
        all.sort_by_cached_key(|d| d.alias().to_lowercase());
        all
    }

    // ---- drawing --------------------------------------------------------

    /// The LocalSend row and, expanded, its devices or a note that there are
    /// none. `last`: no row follows it at its level. `receiving`: this
    /// computer takes files, which the menu of a device depends on.
    pub fn tree(&self, ui: &mut Ui, rows: &mut Rows, last: bool, receiving: bool) {
        let guide = Guide { ancestors: vec![false], has_next: !last, has_prev: true };
        let resp = rows.branch(ui, 1, &guide, RowIcon::Image(self.logo.id()), "LocalSend", Node::Service(Service::LocalSend), self.expanded);
        section_menu(&resp, Service::LocalSend, self.expanded, rows.actions);
        if !self.expanded {
            return;
        }
        // While the sweep goes on, a last row says which part of the network
        // it is at, whether or not devices were found already.
        let sweep = self.scan.as_ref().and_then(Scan::progress).map(|p| progress_text(&p));
        let found = self.sorted();
        let ancestors = vec![false, !last];
        if found.is_empty() {
            let text = match sweep {
                Some(text) => text,
                None if self.searching() => "Searching...".to_owned(),
                None => "No devices found".to_owned(),
            };
            let guide = Guide { ancestors, has_next: false, has_prev: true };
            let resp = note_row(ui, 2, &guide, &text);
            if self.searching() {
                search_menu(&resp, rows.actions);
            }
            return;
        }
        let count = found.len() + usize::from(sweep.is_some());
        for (i, device) in found.iter().enumerate() {
            let key = device.key();
            let guide = Guide { ancestors: ancestors.clone(), has_next: i + 1 < count, has_prev: true };
            let resp = rows.device(ui, 2, &guide, &key, device.alias(), self.logo.id(), describe(device));
            let can_send = rows.can_send;
            let actions = &mut *rows.actions;
            egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| {
                let send = egui_classic_menu::item(ui, RichText::new("Send Selected Items").font(theme::bold(theme::FONT_SIZE)), None);
                if ui.add_enabled(can_send, send).clicked() {
                    actions.push(NetAction::SendSelection(key.clone()));
                }
                let visible = egui_classic_menu::item(ui, "Let Device Find This Computer", None);
                let resp = ui.add_enabled(receiving, visible);
                if receiving && resp.clicked() {
                    actions.push(NetAction::Introduce(key.clone()));
                    ui.close();
                } else if !receiving {
                    let _ = resp.on_disabled_hover_text("Turn on Receive Files first: the device could not send anything to this computer otherwise.");
                }
                properties_item(ui, &key, actions);
            });
        }
        if let Some(text) = sweep {
            let guide = Guide { ancestors, has_next: false, has_prev: true };
            let resp = note_row(ui, 2, &guide, &text);
            search_menu(&resp, rows.actions);
        }
    }

    /// The Send To submenu's items: a device each.
    pub fn send_to_items(&self, ui: &mut Ui, actions: &mut Vec<NetAction>) {
        for device in self.sorted() {
            let button = egui::Button::image_and_text(egui::Image::new((self.logo.id(), egui::vec2(16.0, 16.0))), device.alias());
            if ui.add(button).on_hover_text(describe(device)).clicked() {
                actions.push(NetAction::SendSelection(device.key()));
            }
        }
    }
}

/// The kept identity (with TLS), or none for a fingerprint new to this run.
fn identity() -> Option<::localsend::Identity> {
    #[cfg(all(feature = "localsend-tls", not(test)))]
    return macsplorer_core::preferences::config_dir().and_then(|d| ::localsend::Identity::load_or_generate(&d.join(IDENTITY_FILE)));
    #[cfg(not(all(feature = "localsend-tls", not(test))))]
    None
}

/// The second line of the question: "Phone or tablet (Samsung) at 192.168.2.15".
fn offer_detail(offer: &Offer) -> String {
    let model = offer.from.device_model.as_deref().map_or_else(String::new, |m| format!(" ({m})"));
    let kind = match offer.from.device_type.unwrap_or(DeviceType::Desktop) {
        DeviceType::Mobile => "Phone or tablet",
        DeviceType::Desktop => "Computer",
        DeviceType::Web => "Web browser",
        DeviceType::Headless | DeviceType::Server => "Server",
    };
    format!("{kind}{model} at {}", offer.ip)
}

/// The menu of the note while a search goes on: Cancel stops it.
fn search_menu(resp: &egui::Response, actions: &mut Vec<NetAction>) {
    egui::Popup::context_menu(resp).style(theme::menu_style).show(|ui| {
        if ui.add(egui_classic_menu::item(ui, "Cancel", None)).clicked() {
            actions.push(NetAction::CancelSearch);
            ui.close();
        }
    });
}

/// Which part of the network the sweep is asking, and how far along it is:
/// "Searching 10.11.46.x (38%)", the /24 the address is in.
fn progress_text(p: &SweepProgress) -> String {
    let [a, b, c, _] = p.at.octets();
    format!("Searching {a}.{b}.{c}.x ({}%)", p.percent())
}

fn kind_word(kind: DeviceType) -> &'static str {
    match kind {
        DeviceType::Mobile => "phone or tablet",
        DeviceType::Desktop => "computer",
        DeviceType::Web => "web browser",
        DeviceType::Headless => "LocalSend without a window",
        DeviceType::Server => "server",
    }
}

fn describe(d: &Device) -> String {
    let kind = kind_word(d.device_type());
    let model = d.info.device_model.as_deref().map_or_else(String::new, |m| format!(" ({m})"));
    format!("{}: {kind}{model} at {}, LocalSend protocol {}", d.alias(), d.ip, d.info.version)
}

/// A send as the progress dialog sees it.
struct Sending {
    progress: Progress,
    alias: String,
    /// The first item sent, for the "From" line.
    from: PathBuf,
}

/// A transfer in, as the progress dialog sees it.
struct Receiving {
    progress: Arc<receive::Progress>,
    alias: String,
    folder: String,
}

impl Tracked for Receiving {
    fn snapshot(&self) -> Snapshot {
        let (done, total) = self.progress.bytes();
        let current = self.progress.current();
        let first = if current.is_empty() { format!("Waiting for {}...", self.alias) } else { current };
        Snapshot { done, total, lines: [first, format!("From '{}' to '{}'", self.alias, self.folder)] }
    }

    fn cancel(&self) {
        self.progress.cancel()
    }

    fn is_cancelled(&self) -> bool {
        self.progress.is_cancelled()
    }
}

impl Tracked for Sending {
    fn snapshot(&self) -> Snapshot {
        let (done, total) = self.progress.bytes();
        let folder = self.from.parent().and_then(Path::file_name).map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let lines = match self.progress.stage() {
            Stage::Preparing => ["Preparing...".to_owned(), String::new()],
            Stage::Waiting => [format!("Waiting for {} to accept...", self.alias), String::new()],
            Stage::Sending(name) => [name, format!("From '{folder}' to '{}'", self.alias)],
        };
        // Nothing is done while waiting, so no time left is shown.
        Snapshot { done, total, lines }
    }

    fn cancel(&self) {
        self.progress.cancel()
    }

    fn is_cancelled(&self) -> bool {
        self.progress.is_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use ::localsend::discovery::SweepProgress;

    #[test]
    fn the_search_names_the_part_of_the_network_it_is_at() {
        let at = Ipv4Addr::new(10, 11, 46, 85);
        assert_eq!(super::progress_text(&SweepProgress { done: 76, total: 200, at }), "Searching 10.11.46.x (38%)");
        assert_eq!(super::progress_text(&SweepProgress { done: 0, total: 254, at }), "Searching 10.11.46.x (0%)");
    }
}
