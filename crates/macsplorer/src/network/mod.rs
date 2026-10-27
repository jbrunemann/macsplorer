//! Network Neighborhood: the LocalSend devices nearby, as a section of the
//! Folders pane, sending files to them and receiving theirs. Not from
//! Windows beyond the name and the place in the tree: Explorer's Network
//! Neighborhood listed SMB computers and shares, this one lists LocalSend
//! apps.
//!
//! The protocol is the `localsend` crate; this module is the glue: the
//! rows (drawn like the tree's own, with `app::paint_branch`), the Send To
//! submenu, drops on a device, the "Sending..." and "Receiving..." progress
//! dialogs, the Receive Files question ([`offer_dialog`]) and the worker
//! that runs the receiver ([`receiving`]). The app talks to it through
//! [`Network`] and [`NetAction`] only, so the feature comes out by
//! removing this folder, the crate and the calls.
//!
//! - Expanding Network Neighborhood starts a scan ([`Scan`]), which goes on
//!   listening while it stays expanded; collapsing stops it. "Search
//!   Again" starts over.
//! - The Send To submenu starts a scan too, if none is running, and keeps
//!   it for [`MENU_SCAN_SECONDS`].
//! - Items dropped on a device, or sent with Send To, go there: the device
//!   asks its user to accept, and the dialog waits until they do.
//!
//! Devices carry LocalSend's own logo (`assets/localsend`), not a Windows
//! icon: they are LocalSend apps, whatever they run on.
//!
//! - Receive Files (Network Neighborhood's menu, or the computer pane of
//!   the status bar) lets nearby devices send files here. It is off until
//!   turned on, remembered in `settings.json` and shared by all windows.
//!   Each offer asks first; accepted files go to Downloads. While it is
//!   on, this computer is shown shared: Windows 2000's sharing hand under
//!   Network Neighborhood's icon and the status bar's computer, whose
//!   tooltip says under what name others see it. Scans only announce this
//!   side while it receives, so a computer that doesn't take files stays
//!   off the other devices' lists.

mod offer_dialog;
mod receiving;

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui;
use egui::{RichText, Sense, Ui};

use localsend::discovery::{Options, Scan, local_ipv4};
use localsend::receive::{self, Closed, End, Event, Offer, Received};
use localsend::send::{self, Progress, SendError, Sent, Stage};
use localsend::{Device, DeviceType, Devices, Me};

pub use offer_dialog::View as OfferView;
use receiving::{Msg, State, Supervisor};

use crate::app::{Guide, Status, TREE_INDENT, TREE_ROW_HEIGHT, paint_branch};
use crate::item_drag::{DragFrame, ItemDrag, Mark};
use crate::progress_dialog::{Kind, ProgressDialog, Snapshot, Tracked};
use crate::theme::{self, Icon};
use macsplorer_core::preferences::Preferences;

/// The root row's label.
pub const TITLE: &str = "Network Neighborhood";
/// How long a scan the Send To menu started keeps going.
const MENU_SCAN_SECONDS: f64 = 30.0;
/// How long a scan may look before an empty list says nothing was found:
/// the announcements are out by then (`localsend::discovery`).
const SEARCH_SECONDS: f64 = 3.0;

/// What the user did with the network rows and menus.
#[derive(Clone, Debug)]
pub enum NetAction {
    /// The +/− box, a double-click, Expand or Collapse.
    Toggle,
    /// "Search Again": forget the devices and scan anew.
    Rescan,
    /// Make sure a scan runs, for the Send To menu.
    Scan,
    /// A click on Network Neighborhood (`None`) or a device.
    Select(Option<String>),
    /// Send To, or the device's "Send Selected Items": the selected items.
    SendSelection(String),
    /// Receive Files, on or off.
    ToggleReceive,
    /// "Open Downloads", where received files go.
    OpenDownloads,
}

/// The identity kept between runs, so other devices know this computer
/// again: next to `settings.json`.
#[cfg(all(feature = "localsend-tls", not(test)))]
const IDENTITY_FILE: &str = "localsend-identity.pem";

/// A send's outcome, tagged with its dialog.
type SendDone = (u64, String, Result<Sent, SendError>);

pub struct Network {
    me: Me,
    devices: Devices,
    expanded: bool,
    /// The row last clicked, while the network has the tree's selection:
    /// `Some(None)` for Network Neighborhood itself.
    selected: Option<Option<String>>,
    scan: Option<Scan>,
    /// When the scan started, for "Searching..."
    scan_started: f64,
    /// A scan started by the menu stops then, unless the section is expanded.
    scan_until: f64,
    found_tx: Sender<Device>,
    found_rx: Receiver<Device>,
    /// Where devices were found before, from the settings; the next scan
    /// asks there first.
    remembered: Vec<Ipv4Addr>,
    dialogs: Vec<ProgressDialog>,
    send_count: u64,
    done_tx: Sender<SendDone>,
    done_rx: Receiver<SendDone>,
    /// A message for the status bar, taken by the app.
    status: Option<Status>,
    /// Tests: never touch the network.
    offline: bool,
    /// LocalSend's logo, which every device row and menu item shows.
    logo: egui::TextureHandle,
    /// Runs the receiver while Receive Files is on; `None` in tests.
    supervisor: Option<Supervisor>,
    msg_rx: Receiver<Msg>,
    /// Receive Files as set.
    receive_wanted: bool,
    receive_state: State,
    /// The offer being asked, with its dialog's id and what it shows.
    offer: Option<(u64, Offer, OfferView)>,
    ctx: egui::Context,
}

impl Network {
    /// `alias` is the name other devices list this one under.
    pub fn new(ctx: egui::Context, alias: &str) -> Self {
        let (found_tx, found_rx) = channel();
        let (done_tx, done_rx) = channel();
        let alias = alias.strip_suffix(".local").unwrap_or(alias);
        let me = Me::with_identity(alias, Some("Macsplorer".into()), identity());
        let (msg_tx, msg_rx) = channel();
        let supervisor = (!cfg!(test)).then(|| Supervisor::start(me.clone(), msg_tx, ctx.clone()));
        Self {
            me,
            devices: Devices::default(),
            expanded: false,
            selected: None,
            scan: None,
            scan_started: 0.0,
            scan_until: 0.0,
            found_tx,
            found_rx,
            remembered: if cfg!(test) { Vec::new() } else { Preferences::load().devices },
            dialogs: Vec::new(),
            send_count: 0,
            done_tx,
            done_rx,
            status: None,
            offline: false,
            logo: load_logo(&ctx),
            supervisor,
            msg_rx,
            receive_wanted: false,
            receive_state: State::Off,
            offer: None,
            ctx,
        }
    }

    /// Tests: shows Receive Files as on.
    #[cfg(test)]
    pub fn fake_receiving(&mut self) {
        self.receive_wanted = true;
        self.receive_state = State::Elsewhere;
    }

    /// Whether this computer takes files from nearby devices, which the
    /// status bar shows.
    pub fn receiving(&self) -> bool {
        self.receive_state.receiving()
    }

    /// The status bar computer's tooltip.
    pub fn computer_tooltip(&self) -> String {
        let name = &self.me.alias;
        match &self.receive_state {
            State::On(_) | State::Elsewhere => format!(
                "Receiving files: nearby devices with LocalSend see this computer as \"{name}\" and can offer it files, which go to {} once you accept them.",
                folder_name(&receiving::downloads())
            ),
            State::Failed(e) => format!("Receive Files is on, but this computer can't receive: {e}"),
            State::Off => "Not receiving files. Click to let nearby devices with LocalSend send files to this computer.".into(),
        }
    }

    /// The menu of the status bar's computer pane.
    pub fn computer_menu(&self, ui: &mut Ui, actions: &mut Vec<NetAction>) {
        self.receive_items(ui, actions);
    }

    /// Receive Files and Open Downloads.
    fn receive_items(&self, ui: &mut Ui, actions: &mut Vec<NetAction>) {
        let mut on = self.receive_wanted;
        if egui_classic_menu::checkbox(ui, &mut on, "Receive Files").clicked() {
            actions.push(NetAction::ToggleReceive);
            ui.close();
        }
        if ui.button("Open Downloads").clicked() {
            actions.push(NetAction::OpenDownloads);
        }
    }

    /// Tests: shows `devices` as found, without scanning.
    #[cfg(test)]
    pub fn fake(&mut self, devices: impl IntoIterator<Item = Device>, expanded: bool, selected: Option<String>) {
        self.offline = true;
        self.expanded = expanded;
        devices.into_iter().for_each(|d| _ = self.devices.insert(d));
        self.selected = selected.map(Some);
    }

    /// Whether a network row has the tree's selection, which the folder
    /// rows then don't show.
    pub fn has_selection(&self) -> bool {
        self.selected.is_some()
    }

    /// A folder row was selected.
    pub fn deselect(&mut self) {
        self.selected = None;
    }

    pub fn take_status(&mut self) -> Option<Status> {
        self.status.take()
    }

    /// Takes in what the workers found and finished, and stops a scan that
    /// is no longer wanted. Call once a frame.
    pub fn poll(&mut self) {
        let now = self.now();
        while let Ok(device) = self.found_rx.try_recv() {
            self.remember(&device);
            self.devices.insert(device);
        }
        while let Ok(msg) = self.msg_rx.try_recv() {
            self.receive_msg(msg);
        }
        if let Some((_, offer, view)) = &self.offer
            && let Some(closed) = offer.closed()
        {
            self.status = Some(Status::note(match closed {
                Closed::Withdrawn => format!("{} took back the files it offered", view.from),
                Closed::Expired => format!("The files {} offered weren't accepted in time", view.from),
            }));
            self.offer = None;
        }
        while let Ok((id, alias, result)) = self.done_rx.try_recv() {
            for d in &mut self.dialogs {
                d.finished |= d.id == id;
            }
            self.status = Some(match result {
                Ok(Sent { files, declined: 0 }) => Status::note(format!("Sent {files} file(s) to {alias}")),
                Ok(Sent { files, declined }) => Status::note(format!("Sent {files} file(s) to {alias}; {declined} weren't accepted")),
                Err(SendError::Cancelled) => Status::note(format!("Sending to {alias} was cancelled")),
                Err(e) => Status::error(format!("Couldn't send to {alias}: {e}")),
            });
        }
        if self.scan.is_some() && !self.expanded && now > self.scan_until {
            self.scan = None;
        }
        if self.searching() {
            self.ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn receive_msg(&mut self, msg: Msg) {
        match msg {
            Msg::State { wanted, state } => {
                if let State::Failed(e) = &state {
                    self.status = Some(Status::error(format!("Couldn't receive files: {e}")));
                }
                if let State::On(me) = &state {
                    self.me.port = me.port;
                    self.me.protocol = me.protocol;
                }
                // The next scan announces this side, or no longer does.
                if state.receiving() != self.receive_state.receiving() && !self.expanded {
                    self.scan = None;
                }
                self.receive_wanted = wanted;
                self.receive_state = state;
            }
            Msg::Event(Event::Device(device)) => {
                if device.info.fingerprint != self.me.fingerprint {
                    self.devices.insert(device);
                }
            }
            Msg::Event(Event::Offer(offer)) => {
                if self.offer.is_some() {
                    offer.decline();
                    return;
                }
                self.send_count += 1;
                let view = OfferView {
                    from: offer.from.alias.clone(),
                    detail: offer_detail(&offer),
                    files: offer.files.iter().map(|f| (f.file_name.clone(), f.size)).collect(),
                    folder: folder_name(&receiving::downloads()),
                };
                self.status = Some(Status::note(format!("{} wants to send you files", view.from)));
                // If Macsplorer is in the background, the Dock icon bounces.
                self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
                self.offer = Some((self.send_count, offer, view));
            }
            Msg::Event(Event::Done(received)) => self.received(received),
        }
    }

    /// A transfer in is over: its dialog closes, the status bar says how it went.
    fn received(&mut self, r: Received) {
        for d in &mut self.dialogs {
            d.finished |= d.kind() == Kind::Receive;
        }
        let folder = folder_name(&receiving::downloads());
        let (n, from) = (r.saved.len(), &r.from);
        self.status = Some(match r.end {
            End::Finished if r.missing == 0 => Status::note(format!("Received {n} file(s) from {from} in {folder}")),
            End::Finished => Status::error(format!("Received {n} file(s) from {from} in {folder}; {} didn't arrive", r.missing)),
            End::Cancelled => Status::note(format!("Receiving from {from} was cancelled; {n} file(s) were saved in {folder}")),
            End::CancelledBySender => Status::note(format!("{from} cancelled sending; {n} file(s) were saved in {folder}")),
            End::TimedOut => Status::error(format!("{from} stopped sending; {n} file(s) were saved in {folder}")),
        });
    }

    /// The "Sending..." dialogs that are due. Cancel closes one at once:
    /// while the device hasn't answered yet, the worker can only drop the
    /// transfer once it does.
    pub fn show_dialogs(&mut self) {
        let now = self.now();
        let ctx = self.ctx.clone();
        self.dialogs.retain_mut(|d| {
            d.finished |= d.progress.is_cancelled();
            d.show(&ctx, now, false)
        });
        let answer = self.offer.as_ref().and_then(|(id, _, view)| offer_dialog::show(&ctx, *id, view, self.logo.id()));
        if let Some(answer) = answer
            && let Some((id, offer, view)) = self.offer.take()
        {
            match answer {
                offer_dialog::Answer::Accept => {
                    let progress = offer.accept();
                    let tracked = Arc::new(Receiving { progress, alias: view.from.clone(), folder: view.folder.clone() });
                    self.dialogs.push(ProgressDialog::new(id, tracked, Kind::Receive));
                    self.status = Some(Status::note(format!("Receiving files from {}...", view.from)));
                }
                offer_dialog::Answer::Decline => {
                    offer.decline();
                    self.status = Some(Status::note(format!("Declined the files from {}", view.from)));
                }
            }
        }
    }

    /// Does `action`; `Some` is a folder to open.
    pub fn apply(&mut self, action: NetAction, selection: Vec<PathBuf>) -> Option<PathBuf> {
        match action {
            NetAction::Toggle => {
                self.expanded = !self.expanded;
                if self.expanded {
                    self.ensure_scan();
                } else {
                    self.scan = None;
                    if self.selected.as_ref().is_some_and(Option::is_some) {
                        self.selected = Some(None);
                    }
                }
            }
            NetAction::Rescan => {
                self.devices.clear();
                self.scan = None;
                self.expanded = true;
                self.ensure_scan();
            }
            NetAction::Scan => {
                self.scan_until = self.now() + MENU_SCAN_SECONDS;
                self.ensure_scan();
            }
            NetAction::Select(key) => {
                self.status = key.as_deref().and_then(|k| self.devices.get(k)).map(|d| Status::note(describe(d)));
                self.selected = Some(key);
            }
            NetAction::SendSelection(key) => self.send(&key, selection),
            NetAction::ToggleReceive => {
                self.receive_wanted = !self.receive_wanted;
                if let Some(s) = &self.supervisor {
                    s.set(self.receive_wanted);
                }
            }
            NetAction::OpenDownloads => return Some(receiving::downloads()),
        }
        None
    }

    /// Keeps the address of a device found over IPv4 in the settings, so the
    /// next scan looks there first. Addresses of this computer (a LocalSend
    /// app on it) aren't worth keeping.
    fn remember(&mut self, device: &Device) {
        let IpAddr::V4(ip) = device.ip else { return };
        if cfg!(test) || self.remembered.first() == Some(&ip) || local_ipv4().iter().any(|i| i.ip == ip) {
            return;
        }
        self.remembered.retain(|d| *d != ip);
        self.remembered.insert(0, ip);
        Preferences::update(|p| {
            p.remember_device(ip);
        });
    }

    fn ensure_scan(&mut self) {
        if self.scan.is_none() && !self.offline {
            let (tx, ctx) = (self.found_tx.clone(), self.ctx.clone());
            let options = Options { announce: self.receive_state.receiving(), sweep: true, first: self.remembered.clone() };
            self.scan = Some(Scan::start(self.me.clone(), options, move |d| {
                if tx.send(d).is_ok() {
                    ctx.request_repaint();
                }
            }));
            self.scan_started = self.now();
        }
    }

    /// Whether a scan is young enough that an empty list means "not yet".
    fn searching(&self) -> bool {
        self.scan.as_ref().is_some_and(|s| s.sweeping() || self.now() - self.scan_started < SEARCH_SECONDS)
    }

    fn now(&self) -> f64 {
        self.ctx.input(|i| i.time)
    }

    /// Sends `paths` to the device `key` on a worker thread, with the
    /// progress dialog.
    pub fn send(&mut self, key: &str, paths: Vec<PathBuf>) {
        let Some(device) = self.devices.get(key).cloned() else { return };
        if paths.is_empty() {
            return;
        }
        self.send_count += 1;
        let id = self.send_count;
        let progress = Arc::new(Sending { progress: Progress::default(), alias: device.alias().to_owned(), from: paths[0].clone() });
        self.dialogs.push(ProgressDialog::new(id, progress.clone(), Kind::Send));
        self.status = Some(Status::note(format!("Waiting for {} to accept...", device.alias())));
        let (me, tx, ctx) = (self.me.clone(), self.done_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = send::send(&me, &device, &paths, None, &progress.progress);
            if tx.send((id, device.alias().to_owned(), result)).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    // ---- drawing --------------------------------------------------------

    /// The Network section of the Folders pane: its heading, Network
    /// Neighborhood and, expanded, the devices or a note that there are
    /// none. Device rows take drops.
    pub fn tree(&self, ui: &mut Ui, can_send: bool, focused: bool, (drag, frame): (Option<&ItemDrag>, &mut DragFrame), actions: &mut Vec<NetAction>) {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Network").font(theme::bold(theme::FONT_SIZE)));
        });
        ui.add_space(1.0);
        let root = Guide { ancestors: Vec::new(), has_next: false, has_prev: false };
        let selected = self.selected == Some(None);
        let icon = RowIcon::Paint(Icon::Network, self.receiving());
        let resp = row(ui, 0, &root, icon, TITLE, true, self.expanded, selected, focused, false, actions);
        egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| {
            let label = if self.expanded { "Collapse" } else { "Expand" };
            if ui.add(egui_classic_menu::item(ui, RichText::new(label).font(theme::bold(theme::FONT_SIZE)), None)).clicked() {
                actions.push(NetAction::Toggle);
            }
            if ui.add(egui_classic_menu::item(ui, "Search Again", None)).clicked() {
                actions.push(NetAction::Rescan);
            }
            ui.separator();
            self.receive_items(ui, actions);
        });
        if self.receiving() {
            let _ = resp.on_hover_text(self.computer_tooltip());
        }
        if !self.expanded {
            return;
        }
        if self.devices.is_empty() {
            let text = if self.searching() { "Searching..." } else { "No LocalSend devices found" };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TREE_ROW_HEIGHT), Sense::hover());
            let leaf = Guide { ancestors: vec![false], has_next: false, has_prev: true };
            paint_branch(ui, rect, 1, &leaf, false, false);
            let x = ui.max_rect().left() + 2.0 + 2.0 * TREE_INDENT;
            ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, text, egui::TextStyle::Body.resolve(ui.style()), theme::color::gray_text());
            return;
        }
        let count = self.devices.len();
        for (i, device) in self.devices.iter().enumerate() {
            let key = device.key();
            let guide = Guide { ancestors: vec![false], has_next: i + 1 < count, has_prev: true };
            let mark = Mark::Device(key.clone());
            let marked = drag.is_some_and(|d| d.marks(&mark));
            let selected = self.selected.as_ref().is_some_and(|s| s.as_deref() == Some(key.as_str()));
            let resp = row(ui, 1, &guide, RowIcon::Image(self.logo.id()), device.alias(), false, false, selected || marked, focused || marked, true, actions);
            frame.device(ui.clip_rect(), resp.rect, &key);
            if resp.clicked() {
                actions.push(NetAction::Select(Some(key.clone())));
            }
            let resp = resp.on_hover_text(describe(device));
            egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| {
                let send = egui_classic_menu::item(ui, RichText::new("Send Selected Items").font(theme::bold(theme::FONT_SIZE)), None);
                if ui.add_enabled(can_send, send).clicked() {
                    actions.push(NetAction::SendSelection(key.clone()));
                }
            });
        }
    }

    /// The Send To submenu's items: a device each, or a note while there
    /// are none.
    pub fn send_to_items(&self, ui: &mut Ui, actions: &mut Vec<NetAction>) {
        if self.scan.is_none() && !self.offline {
            actions.push(NetAction::Scan);
        }
        if self.devices.is_empty() {
            let text = if self.scan.is_none() || self.searching() { "Searching..." } else { "No LocalSend devices found" };
            ui.add_enabled(false, egui::Button::new(text));
            return;
        }
        for device in self.devices.iter() {
            let button = egui::Button::image_and_text(egui::Image::new((self.logo.id(), egui::vec2(16.0, 16.0))), device.alias());
            if ui.add(button).on_hover_text(describe(device)).clicked() {
                actions.push(NetAction::SendSelection(device.key()));
            }
        }
    }
}

/// One row: lines, box, icon and a label that highlights only its text, as
/// `app::tree_row` draws a folder. Clicks on the box or a double-click
/// toggle an `expandable` row; a click elsewhere selects it (the caller
/// handles that for devices, `device`).
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    depth: usize,
    guide: &Guide,
    icon: RowIcon,
    label: &str,
    expandable: bool,
    expanded: bool,
    selected: bool,
    focused: bool,
    device: bool,
    actions: &mut Vec<NetAction>,
) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), theme::color::text()).size().x;
    let x0 = ui.max_rect().left() + 2.0;
    let label_x = x0 + (depth + 1) as f32 * TREE_INDENT + 16.0 + 4.0;
    let width = (label_x - ui.max_rect().left() + text_w + 6.0).max(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, TREE_ROW_HEIGHT), Sense::click());
    let boxr = paint_branch(ui, rect, depth, guide, expandable, expanded);
    let p = ui.painter();
    let icon_rect = egui::Rect::from_min_size(egui::pos2(x0 + (depth + 1) as f32 * TREE_INDENT, rect.center().y - 8.0), egui::vec2(16.0, 16.0));
    match icon {
        RowIcon::Paint(icon, shared) => {
            theme::paint_icon(p, icon_rect, icon, true);
            if shared {
                theme::paint_shared(p, icon_rect);
            }
        }
        RowIcon::Image(id) => {
            p.image(id, icon_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
    }
    let text_rect = egui::Rect::from_min_size(egui::pos2(label_x - 2.0, rect.center().y - 7.0), egui::vec2(text_w + 4.0, 14.0));
    let color = if selected {
        p.rect_filled(text_rect, 0.0, if focused { theme::color::selection() } else { theme::color::face() });
        if focused { theme::color::selection_text() } else { theme::color::text() }
    } else {
        theme::color::text()
    };
    p.text(egui::pos2(label_x, rect.center().y), egui::Align2::LEFT_CENTER, label, font, color);
    if !device {
        let on_box = resp.interact_pointer_pos().is_some_and(|p| boxr.expand(3.0).contains(p));
        if resp.clicked() {
            actions.push(if on_box { NetAction::Toggle } else { NetAction::Select(None) });
        }
        if resp.double_clicked() && !on_box {
            actions.push(NetAction::Toggle);
        }
    }
    resp
}

/// What a row shows left of its label: an icon, with the sharing hand
/// under it, or an image.
enum RowIcon {
    Paint(Icon, bool),
    Image(egui::TextureId),
}

/// LocalSend's logo as a texture, 32 pixels for the 16 point icon.
fn load_logo(ctx: &egui::Context) -> egui::TextureHandle {
    let png = include_bytes!("../../../../assets/localsend/logo-32.png");
    let rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("the embedded LocalSend logo is a PNG").into_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    ctx.load_texture("localsend-logo", image, egui::TextureOptions::LINEAR)
}

/// The kept identity (with TLS), or none for a fingerprint new to this run.
fn identity() -> Option<localsend::Identity> {
    #[cfg(all(feature = "localsend-tls", not(test)))]
    return macsplorer_core::preferences::config_dir().and_then(|d| localsend::Identity::load_or_generate(&d.join(IDENTITY_FILE)));
    #[cfg(not(all(feature = "localsend-tls", not(test))))]
    None
}

/// A folder as the user knows it: "Downloads".
fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
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

/// A device's tooltip and status bar text.
fn describe(d: &Device) -> String {
    let kind = match d.device_type() {
        DeviceType::Mobile => "phone or tablet",
        DeviceType::Desktop => "computer",
        DeviceType::Web => "web browser",
        DeviceType::Headless => "LocalSend without a window",
        DeviceType::Server => "server",
    };
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
        let folder = self.from.parent().and_then(|p| p.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into_owned());
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
pub mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;

    use eframe::egui;
    use egui_kittest::Harness;

    use localsend::{Device, DeviceInfo, DeviceType, Protocol};

    /// A device as a scan would report it.
    pub fn device(alias: &str, kind: DeviceType, last: u8) -> Device {
        let info = DeviceInfo {
            alias: alias.into(),
            version: "2.1".into(),
            device_model: Some(if kind == DeviceType::Mobile { "Pixel" } else { "macOS" }.into()),
            device_type: Some(kind),
            fingerprint: format!("fp-{last}"),
            port: None,
            protocol: None,
            download: Some(false),
        };
        Device::new(info, IpAddr::V4(Ipv4Addr::new(192, 168, 1, last)), 53317, Protocol::Https)
    }

    /// The Receive Files question, for a phone offering photos.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn receive_offer() {
        let mut files: Vec<(String, u64)> = (0..7).map(|i| (format!("IMG_20241017_18{:04}.jpg", 3205 + i * 7), 2_400_000 + i * 310_000)).collect();
        files.insert(0, ("Holiday video.mp4".into(), 48_500_000));
        let view = super::OfferView { from: "Jan's Pixel".into(), detail: "Phone or tablet (Pixel 8) at 192.168.2.31".into(), files, folder: "Downloads".into() };
        let size = view.size();
        let mut harness = Harness::builder().with_size(size).with_pixels_per_point(2.0).wgpu().build_ui_state(
            |ui, logo: &mut Option<egui::TextureHandle>| {
                let Some(logo) = logo else {
                    crate::theme::install(ui.ctx());
                    crate::theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                    *logo = Some(super::load_logo(ui.ctx()));
                    return;
                };
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                super::offer_dialog::draw(&mut window, &view, logo.id());
            },
            None,
        );
        for _ in 0..3 {
            harness.run();
        }
        let image = harness.render().expect("render");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        std::fs::create_dir_all(&out).unwrap();
        image.save(out.join("receive-offer.png")).unwrap();
    }
}
