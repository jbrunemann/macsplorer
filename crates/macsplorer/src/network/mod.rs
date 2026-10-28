//! Network Neighborhood: the LocalSend and AirLynx devices nearby, as a
//! section of the Folders pane, sending files to them and receiving theirs.
//! Not from Windows beyond the name and the place in the tree: Explorer's
//! Network Neighborhood listed SMB computers and shares, this one lists
//! LocalSend and AirLynx apps.
//!
//! Network Neighborhood has one row for each of the two, which expand on
//! their own with a +:
//!
//! ```text
//! Network Neighborhood
//!   LocalSend
//!     Jan's Pixel
//!   AirLynx
//!     Living Room Mac
//! ```
//!
//! They are separate in the code too. [`localsend`] and [`airlynx`] each own
//! their devices, scan, sends and rows, and neither knows the other; nothing
//! of a protocol runs until its row is expanded. This module is the glue
//! between them and the app: the root row, what they share (the status bar
//! message, the progress dialogs, the device Properties windows), and the
//! routing of an action to the protocol its device key belongs to. The app
//! talks to it through [`Network`] and [`NetAction`] only, so the feature
//! comes out by removing this folder, the crates and the calls, and either
//! protocol by removing its file and the lines that route to it.
//!
//! - Expanding a protocol's row starts its scan, which goes on while it stays
//!   expanded; collapsing it (or Network Neighborhood) stops it.
//! - The Send To submenu starts both scans, if none is running, and keeps
//!   them for [`MENU_SCAN_SECONDS`]: it is how a device is reached without
//!   opening the Folders pane.
//! - Items dropped on a device, or sent with Send To, go there.
//!
//! Devices carry their app's own logo (`assets/localsend`, `assets/airlynx`),
//! not a Windows icon: they are LocalSend or AirLynx apps, whatever they run
//! on.
//!
//! - Receive Files (Network Neighborhood's menu, or the computer pane of
//!   the status bar) lets nearby devices send files here, over both
//!   protocols ([`receiving`]). It is off until turned on, remembered in
//!   `settings.json` and shared by all windows. Each LocalSend offer asks
//!   first; accepted files go to Downloads, as do the files of AirLynx
//!   favorites, which arrive without a question and show in the status bar.
//!   While it is on, this computer is shown shared: Windows 2000's sharing
//!   hand under Network Neighborhood's icon and the status bar's computer,
//!   whose tooltip says under what name others see it. LocalSend's scans
//!   only announce this side while it receives, so a computer that doesn't
//!   take files stays off the other devices' lists.

mod airlynx;
mod device_dialog;
mod localsend;
mod offer_dialog;
mod receiving;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;
use egui::{RichText, Sense, Ui};

use airlynx::AirLynx;
use localsend::LocalSend;
use receiving::{Msg, State, Supervisor};

use crate::app::{Guide, Status, TREE_INDENT, TREE_ROW_HEIGHT, paint_branch};
use crate::item_drag::{DragFrame, ItemDrag, Mark};
use crate::progress_dialog::ProgressDialog;
use crate::theme::{self, Icon};

/// The root row's label.
pub const TITLE: &str = "Network Neighborhood";
/// How long a scan the Send To menu started keeps going.
const MENU_SCAN_SECONDS: f64 = 30.0;
/// How long a scan may look before an empty list says nothing was found:
/// the announcements are out by then (`localsend::discovery`).
const SEARCH_SECONDS: f64 = 3.0;

/// The two kinds of device Network Neighborhood lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    LocalSend,
    AirLynx,
}

impl Service {
    /// The protocol a device key belongs to.
    fn of(key: &str) -> Self {
        if key.starts_with(airlynx::KEY_PREFIX) { Service::AirLynx } else { Service::LocalSend }
    }
}

/// A row of the section, as selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Network Neighborhood itself.
    Root,
    /// LocalSend's or AirLynx's row.
    Service(Service),
    /// A device, by its key.
    Device(String),
}

/// What the user did with the network rows and menus.
#[derive(Clone, Debug)]
pub enum NetAction {
    /// The +/− box, a double-click, Expand or Collapse: of a protocol's row,
    /// or `None` for Network Neighborhood.
    Toggle(Option<Service>),
    /// "Search Again" on a protocol's row: forget its devices and scan anew.
    Rescan(Service),
    /// A LocalSend device's menu: tell it about this computer, by the
    /// device's key.
    Introduce(String),
    /// Stop asking addresses, from LocalSend's search note's menu.
    CancelSearch,
    /// Make sure the scans run, for the Send To menu.
    Scan,
    /// A click on a row.
    Select(Node),
    /// Send To, or the device's "Send Selected Items": the selected items.
    SendSelection(String),
    /// Receive Files, on or off.
    ToggleReceive,
    /// "Open Downloads", where received files go.
    OpenDownloads,
    /// An AirLynx device's menu: add it to the favorites, or take it off.
    ToggleFavorite(String),
    /// A device's menu: its Properties window, by the device's key.
    Properties(String),
}

/// What the protocols share: the message for the status bar and the
/// progress dialogs, with the numbering of both.
#[derive(Default)]
struct Shared {
    /// A message for the status bar, taken by the app.
    status: Option<Status>,
    dialogs: Vec<ProgressDialog>,
    count: u64,
}

impl Shared {
    fn say(&mut self, status: Status) {
        self.status = Some(status);
    }

    /// An id for a dialog (a send's, or a question's).
    fn next_id(&mut self) -> u64 {
        self.count += 1;
        self.count
    }

    /// The transfer behind dialog `id` is over.
    fn finish(&mut self, id: u64) {
        for d in &mut self.dialogs {
            d.finished |= d.id == id;
        }
    }
}

pub struct Network {
    /// Network Neighborhood itself is expanded: the protocols' rows show.
    expanded: bool,
    /// The row last clicked, while the network has the tree's selection.
    selected: Option<Node>,
    local: LocalSend,
    air: AirLynx,
    shared: Shared,
    /// Open device Properties windows, by the device's key. The view is
    /// refreshed while the device is still listed, and stays as it was
    /// when it isn't any more.
    device_props: Vec<(String, device_dialog::View)>,
    /// Runs the receivers while Receive Files is on; `None` in tests.
    supervisor: Option<Supervisor>,
    msg_rx: Receiver<Msg>,
    /// Receive Files as set.
    receive_wanted: bool,
    receive_state: State,
    ctx: egui::Context,
}

impl Network {
    /// `alias` is the name other devices list this one under.
    pub fn new(ctx: egui::Context, alias: &str) -> Self {
        let alias = alias.strip_suffix(".local").unwrap_or(alias);
        let local = LocalSend::new(ctx.clone(), alias);
        let air = AirLynx::new(ctx.clone(), alias);
        let (msg_tx, msg_rx) = channel();
        let supervisor = (!cfg!(test)).then(|| Supervisor::start(local.me().clone(), air.me.clone(), air.favorites.clone(), msg_tx, ctx.clone()));
        Self {
            expanded: false,
            selected: None,
            local,
            air,
            shared: Shared::default(),
            device_props: Vec::new(),
            supervisor,
            msg_rx,
            receive_wanted: false,
            receive_state: State::Off,
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
        let name = &self.local.me().alias;
        match &self.receive_state {
            State::On(_) | State::Elsewhere => format!(
                "Receiving files: nearby devices with LocalSend or AirLynx see this computer as \"{name}\". LocalSend devices can offer it files, which go to {} once you accept them, and AirLynx favorites can send them right away.",
                folder_name(&receiving::downloads())
            ),
            State::Failed(e) => format!("Receive Files is on, but this computer can't receive: {e}"),
            State::Off => "Not receiving files. Click to let nearby devices with LocalSend or AirLynx send files to this computer.".into(),
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
        if ui.add(egui_classic_menu::item(ui, "Open Downloads", None)).clicked() {
            actions.push(NetAction::OpenDownloads);
            ui.close();
        }
    }

    /// Tests: shows `devices` as found under LocalSend, without scanning.
    #[cfg(test)]
    pub fn fake(&mut self, devices: impl IntoIterator<Item = ::localsend::Device>, expanded: bool, selected: Option<String>) {
        self.expanded = expanded;
        self.local.fake(devices, expanded);
        self.selected = selected.map(Node::Device);
    }

    /// Tests: shows `peers` as found under AirLynx.
    #[cfg(test)]
    pub fn fake_air(&mut self, peers: impl IntoIterator<Item = airlynx_core::Peer>) {
        self.air.fake(peers);
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
        self.shared.status.take()
    }

    /// Whether the protocol's scan has a reason to run: its row can be seen.
    fn visible(&self, service: Service) -> bool {
        self.expanded
            && match service {
                Service::LocalSend => self.local.expanded(),
                Service::AirLynx => self.air.expanded(),
            }
    }

    /// Takes in what the workers found and finished, and stops a scan that
    /// is no longer wanted. Call once a frame.
    pub fn poll(&mut self) {
        let (local, air) = (self.visible(Service::LocalSend), self.visible(Service::AirLynx));
        self.local.poll(local, &mut self.shared);
        self.air.poll(air, &mut self.shared);
        while let Ok(msg) = self.msg_rx.try_recv() {
            self.receive_msg(msg);
        }
    }

    fn receive_msg(&mut self, msg: Msg) {
        match msg {
            Msg::State { wanted, state } => {
                if let State::Failed(e) = &state {
                    self.shared.say(Status::error(format!("Couldn't receive files: {e}")));
                }
                if let State::On(me) = &state {
                    self.local.listening(me);
                }
                // The next scan announces this side, or no longer does.
                if state.receiving() != self.receive_state.receiving() && !self.visible(Service::LocalSend) {
                    self.local.stop();
                }
                self.receive_wanted = wanted;
                self.receive_state = state;
            }
            Msg::Event(event) => self.local.event(event, &mut self.shared),
            Msg::Air(event) => self.air.event(event, &mut self.shared),
            Msg::AirUnavailable(why) => self.air.unavailable(&why, &mut self.shared),
        }
    }

    /// The "Sending..." dialogs that are due. Cancel closes one at once:
    /// while the device hasn't answered yet, the worker can only drop the
    /// transfer once it does.
    pub fn show_dialogs(&mut self) {
        let now = now(&self.ctx);
        let ctx = self.ctx.clone();
        self.shared.dialogs.retain_mut(|d| {
            d.finished |= d.progress.is_cancelled();
            d.show(&ctx, now, false)
        });
        let (local, air) = (&self.local, &self.air);
        self.device_props.retain_mut(|(key, view)| {
            let (fresh, logo) = match Service::of(key) {
                Service::LocalSend => (local.view(key), local.logo()),
                Service::AirLynx => (air.view(key), air.logo()),
            };
            if let Some(fresh) = fresh {
                *view = fresh;
            }
            !device_dialog::show(&ctx, key, view, logo)
        });
        self.local.show_dialogs(&mut self.shared);
    }

    /// Does `action`; `Some` is a folder to open.
    pub fn apply(&mut self, action: NetAction, selection: Vec<PathBuf>) -> Option<PathBuf> {
        match action {
            NetAction::Toggle(None) => {
                self.expanded = !self.expanded;
                if !self.expanded && self.selected.as_ref().is_some_and(|n| *n != Node::Root) {
                    self.selected = Some(Node::Root);
                }
                self.sync(Service::LocalSend);
                self.sync(Service::AirLynx);
            }
            NetAction::Toggle(Some(service)) => {
                match service {
                    Service::LocalSend => self.local.toggle(),
                    Service::AirLynx => self.air.toggle(),
                }
                if !self.visible(service) && matches!(&self.selected, Some(Node::Device(key)) if Service::of(key) == service) {
                    self.selected = Some(Node::Service(service));
                }
                self.sync(service);
            }
            NetAction::Rescan(service) => {
                self.expanded = true;
                let announce = self.receiving();
                match service {
                    Service::LocalSend => self.local.rescan(announce),
                    Service::AirLynx => self.air.rescan(),
                }
            }
            NetAction::Introduce(key) => self.local.introduce(&key, &mut self.shared),
            NetAction::CancelSearch => self.local.cancel_search(),
            NetAction::Scan => {
                let until = now(&self.ctx) + MENU_SCAN_SECONDS;
                let announce = self.receiving();
                self.local.keep_until(until);
                self.local.ensure_scan(announce);
                self.air.keep_until(until);
                self.air.ensure_scan();
            }
            NetAction::Select(node) => {
                let text = match &node {
                    Node::Device(key) => self.describe(key),
                    Node::Root | Node::Service(_) => None,
                };
                self.shared.status = text.map(Status::note);
                self.selected = Some(node);
            }
            NetAction::ToggleFavorite(key) => self.air.toggle_favorite(&key, &mut self.shared),
            NetAction::Properties(key) => {
                if !self.device_props.iter().any(|(k, _)| *k == key)
                    && let Some(view) = self.device_view(&key)
                {
                    self.device_props.push((key, view));
                }
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

    /// Starts the protocol's scan when its row is in sight, and ends it when
    /// it isn't.
    fn sync(&mut self, service: Service) {
        let (visible, announce) = (self.visible(service), self.receiving());
        match service {
            Service::LocalSend if visible => self.local.ensure_scan(announce),
            Service::LocalSend => self.local.stop(),
            Service::AirLynx if visible => self.air.ensure_scan(),
            Service::AirLynx => self.air.stop(),
        }
    }

    /// A device's tooltip and status bar text.
    fn describe(&self, key: &str) -> Option<String> {
        match Service::of(key) {
            Service::LocalSend => self.local.describe(key),
            Service::AirLynx => self.air.describe(key),
        }
    }

    /// What a device's Properties window shows.
    fn device_view(&self, key: &str) -> Option<device_dialog::View> {
        match Service::of(key) {
            Service::LocalSend => self.local.view(key),
            Service::AirLynx => self.air.view(key),
        }
    }

    /// Sends `paths` to the device `key` on a worker thread, with the
    /// progress dialog.
    pub fn send(&mut self, key: &str, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        match Service::of(key) {
            Service::LocalSend => self.local.send(key, paths, &mut self.shared),
            Service::AirLynx => self.air.send(key, paths, &mut self.shared),
        }
    }

    // ---- drawing --------------------------------------------------------

    /// The Network section of the Folders pane: its heading, Network
    /// Neighborhood and, expanded, the two protocols with their devices.
    /// Device rows take drops.
    pub fn tree(&self, ui: &mut Ui, can_send: bool, focused: bool, (drag, frame): (Option<&ItemDrag>, &mut DragFrame), actions: &mut Vec<NetAction>) {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Network").font(theme::bold(theme::FONT_SIZE)));
        });
        ui.add_space(1.0);
        let receiving = self.receiving();
        let mut rows = Rows { can_send, focused, drag, frame, selected: self.selected.as_ref(), actions };
        let root = Guide { ancestors: Vec::new(), has_next: false, has_prev: false };
        let resp = rows.branch(ui, 0, &root, RowIcon::Paint(Icon::Network, receiving), TITLE, Node::Root, self.expanded);
        let actions = &mut *rows.actions;
        egui::Popup::context_menu(&resp).style(theme::menu_style).show(|ui| {
            let label = if self.expanded { "Collapse" } else { "Expand" };
            if ui.add(egui_classic_menu::item(ui, RichText::new(label).font(theme::bold(theme::FONT_SIZE)), None)).clicked() {
                actions.push(NetAction::Toggle(None));
            }
            ui.separator();
            self.receive_items(ui, actions);
        });
        if receiving {
            let _ = resp.on_hover_text(self.computer_tooltip());
        }
        if !self.expanded {
            return;
        }
        self.local.tree(ui, &mut rows, false, receiving);
        self.air.tree(ui, &mut rows, true);
    }

    /// The Send To submenu's items: a device each, or a note while there
    /// are none.
    pub fn send_to_items(&self, ui: &mut Ui, actions: &mut Vec<NetAction>) {
        if self.local.idle() || self.air.idle() {
            actions.push(NetAction::Scan);
        }
        if self.local.is_empty() && self.air.is_empty() {
            let looking = self.local.searching() || self.air.searching() || !(self.local.scanning() || self.air.scanning());
            let text = if looking { self.local.searching_text() } else { "No LocalSend or AirLynx devices found".to_owned() };
            ui.add_enabled(false, egui::Button::new(text));
            return;
        }
        self.local.send_to_items(ui, actions);
        self.air.send_to_items(ui, actions);
    }
}

/// What the protocols' rows need to draw: the pane's state, and where the
/// clicks and menu choices go.
struct Rows<'a> {
    can_send: bool,
    focused: bool,
    drag: Option<&'a ItemDrag>,
    frame: &'a mut DragFrame,
    selected: Option<&'a Node>,
    actions: &'a mut Vec<NetAction>,
}

impl Rows<'_> {
    /// A row that expands: Network Neighborhood or a protocol.
    #[allow(clippy::too_many_arguments)]
    fn branch(&mut self, ui: &mut Ui, depth: usize, guide: &Guide, icon: RowIcon, label: &str, node: Node, expanded: bool) -> egui::Response {
        let selected = self.selected == Some(&node);
        row(ui, depth, guide, icon, label, Some((&node, expanded)), selected, self.focused, self.actions)
    }

    /// A device's row, a drop target; the caller adds its menu. A click
    /// selects it, and `tooltip` is what it shows when pointed at.
    #[allow(clippy::too_many_arguments)]
    fn device(&mut self, ui: &mut Ui, depth: usize, guide: &Guide, key: &str, alias: &str, logo: egui::TextureId, tooltip: String) -> egui::Response {
        let marked = self.drag.is_some_and(|d| d.marks(&Mark::Device(key.to_owned())));
        let selected = self.selected.is_some_and(|n| matches!(n, Node::Device(k) if k == key));
        let resp = row(ui, depth, guide, RowIcon::Image(logo), alias, None, selected || marked, self.focused || marked, self.actions);
        self.frame.device(ui.clip_rect(), resp.rect, key);
        if resp.clicked() {
            self.actions.push(NetAction::Select(Node::Device(key.to_owned())));
        }
        resp.on_hover_text(tooltip)
    }
}

/// One row: lines, box, icon and a label that highlights only its text, as
/// `app::tree_row` draws a folder. A `branch` (the node and whether it is
/// expanded) has a box: clicks on the box or a double-click toggle it, a
/// click elsewhere selects it. A device has none, and the caller handles its
/// clicks.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    depth: usize,
    guide: &Guide,
    icon: RowIcon,
    label: &str,
    branch: Option<(&Node, bool)>,
    selected: bool,
    focused: bool,
    actions: &mut Vec<NetAction>,
) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), theme::color::text()).size().x;
    let x0 = ui.max_rect().left() + 2.0;
    let label_x = x0 + (depth + 1) as f32 * TREE_INDENT + 16.0 + 4.0;
    let width = (label_x - ui.max_rect().left() + text_w + 6.0).max(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, TREE_ROW_HEIGHT), Sense::click());
    let boxr = paint_branch(ui, rect, depth, guide, branch.is_some(), branch.is_some_and(|(_, expanded)| expanded));
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
    if let Some((node, _)) = branch {
        let service = match node {
            Node::Service(s) => Some(*s),
            Node::Root | Node::Device(_) => None,
        };
        let on_box = resp.interact_pointer_pos().is_some_and(|p| boxr.expand(3.0).contains(p));
        if resp.clicked() {
            actions.push(if on_box { NetAction::Toggle(service) } else { NetAction::Select(node.clone()) });
        }
        if resp.double_clicked() && !on_box {
            actions.push(NetAction::Toggle(service));
        }
    }
    resp
}

/// The menu of a protocol's row: Expand or Collapse, and Search Again.
fn section_menu(resp: &egui::Response, service: Service, expanded: bool, actions: &mut Vec<NetAction>) {
    egui::Popup::context_menu(resp).style(theme::menu_style).show(|ui| {
        let label = if expanded { "Collapse" } else { "Expand" };
        if ui.add(egui_classic_menu::item(ui, RichText::new(label).font(theme::bold(theme::FONT_SIZE)), None)).clicked() {
            actions.push(NetAction::Toggle(Some(service)));
        }
        if ui.add(egui_classic_menu::item(ui, "Search Again", None)).clicked() {
            actions.push(NetAction::Rescan(service));
        }
    });
}

/// The device menu's last item, set off by a line: its Properties window.
fn properties_item(ui: &mut Ui, key: &str, actions: &mut Vec<NetAction>) {
    ui.separator();
    if ui.add(egui_classic_menu::item(ui, "Properties", None)).clicked() {
        actions.push(NetAction::Properties(key.to_owned()));
        ui.close();
    }
}

/// What a row shows left of its label: an icon, with the sharing hand
/// under it, or an image.
enum RowIcon {
    Paint(Icon, bool),
    Image(egui::TextureId),
}

/// A gray note in a protocol's list: "No devices found", or how the search
/// is going. `depth` is the level of the devices it sits under.
fn note_row(ui: &mut Ui, depth: usize, guide: &Guide, text: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TREE_ROW_HEIGHT), Sense::click());
    paint_branch(ui, rect, depth, guide, false, false);
    let x = ui.max_rect().left() + 2.0 + (depth + 1) as f32 * TREE_INDENT;
    ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, text, egui::TextStyle::Body.resolve(ui.style()), theme::color::gray_text());
    resp
}

/// A logo as a texture, 32 pixels for the 16 point icon.
fn load_png(ctx: &egui::Context, name: &str, png: &[u8]) -> egui::TextureHandle {
    let rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("the embedded logo is a PNG").into_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}

/// A folder as the user knows it: "Downloads".
fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// A word with a capital: "computer" is "Computer".
fn sentence(word: &str) -> String {
    word[..1].to_uppercase() + &word[1..]
}

/// The frame's time, in seconds.
fn now(ctx: &egui::Context) -> f64 {
    ctx.input(|i| i.time)
}

#[cfg(test)]
pub mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;

    use eframe::egui;
    use egui_kittest::Harness;

    use ::localsend::{Device, DeviceInfo, DeviceType, Protocol};

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

    /// An AirLynx device as a scan would report it.
    pub fn air_peer(name: &str, kind: airlynx_core::DeviceKind, mutual: bool, last: u8) -> airlynx_core::Peer {
        let mut id = [0u8; 16];
        id[15] = last;
        airlynx_core::Peer {
            addr: std::net::SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, last)), 47800),
            id: airlynx_core::DeviceId::from_bytes(id),
            name: name.into(),
            mutual,
            kind,
        }
    }

    /// A Network with a phone under LocalSend and a computer under AirLynx.
    fn both() -> (super::Network, String, String) {
        let mut network = super::Network::new(egui::Context::default(), "Test Mac");
        let phone = device("Jan's Pixel", DeviceType::Mobile, 31);
        let mac = air_peer("Living Room Mac", airlynx_core::DeviceKind::Desktop, true, 44);
        let keys = (phone.key(), super::airlynx::key(&mac));
        network.fake([phone], true, None);
        network.fake_air([mac]);
        (network, keys.0, keys.1)
    }

    /// A device's Properties name where it is, who it is and when it was seen.
    #[test]
    fn a_device_has_properties() {
        let (mut network, local_key, air_key) = both();
        let rows = |view: &super::device_dialog::View| view.groups.iter().flatten().map(|(l, v)| format!("{l} {v}")).collect::<Vec<_>>();

        let local = rows(&network.device_view(&local_key).unwrap());
        assert!(local.iter().any(|r| r == "Address: 192.168.1.31"), "{local:?}");
        assert!(local.iter().any(|r| r == "Protocol: LocalSend 2.1, HTTPS"), "{local:?}");
        assert!(local.iter().all(|r| !r.starts_with("Fingerprint") && !r.contains("seen")), "{local:?}");

        let air = network.device_view(&air_key).unwrap();
        let air_rows = rows(&air);
        assert!(air_rows.iter().any(|r| r == "Address: 192.168.1.44"), "{air_rows:?}");
        assert!(air_rows.iter().any(|r| r == "ID: 0000000000000000000000000000002c"), "{air_rows:?}");
        assert!(air_rows.iter().any(|r| r == "Short ID: 00000000"), "{air_rows:?}");
        assert!(air_rows.iter().any(|r| r.starts_with("Favorite: No")), "{air_rows:?}");
        assert!(air_rows.iter().any(|r| r.starts_with("Added you: Yes")), "{air_rows:?}");
        assert!(air_rows.iter().all(|r| !r.ends_with(": ") && !r.ends_with(':')), "a date is missing: {air_rows:?}");
        assert_eq!(air.title(), "Living Room Mac Properties");

        // The menu's action opens one window, however often it is chosen.
        network.apply(super::NetAction::Properties(local_key.clone()), Vec::new());
        network.apply(super::NetAction::Properties(local_key), Vec::new());
        network.apply(super::NetAction::Properties("gone".into()), Vec::new());
        assert_eq!(network.device_props.len(), 1);
    }

    /// Each protocol's row expands on its own, and a scan only runs while it
    /// is expanded and in sight.
    #[test]
    fn the_protocols_expand_on_their_own() {
        use super::{NetAction, Node, Service};
        let (mut network, local_key, air_key) = both();
        let toggle = |n: &mut super::Network, s| {
            n.apply(NetAction::Toggle(s), Vec::new());
        };
        assert!(network.visible(Service::LocalSend) && network.visible(Service::AirLynx));

        // AirLynx collapses, LocalSend stays; a device of AirLynx that was
        // selected leaves the selection on its row.
        network.apply(NetAction::Select(Node::Device(air_key.clone())), Vec::new());
        toggle(&mut network, Some(Service::AirLynx));
        assert!(network.visible(Service::LocalSend) && !network.visible(Service::AirLynx));
        assert_eq!(network.selected, Some(Node::Service(Service::AirLynx)));

        // A device of the other one stays selected.
        network.apply(NetAction::Select(Node::Device(local_key.clone())), Vec::new());
        toggle(&mut network, Some(Service::AirLynx));
        assert_eq!(network.selected, Some(Node::Device(local_key)));

        // Collapsing Network Neighborhood hides both, and keeps their state.
        toggle(&mut network, None);
        assert!(!network.visible(Service::LocalSend) && !network.visible(Service::AirLynx));
        assert_eq!(network.selected, Some(Node::Root));
        toggle(&mut network, None);
        assert!(network.visible(Service::LocalSend) && network.visible(Service::AirLynx));
    }

    /// The device Properties windows, for a LocalSend and an AirLynx device.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn device_properties() {
        let (network, local_key, air_key) = both();
        let views = [(network.device_view(&local_key).unwrap(), false), (network.device_view(&air_key).unwrap(), true)];
        for (view, air) in views {
            let mut harness = Harness::builder().with_size(view.size()).with_pixels_per_point(2.0).wgpu().build_ui_state(
                |ui, apps: &mut Option<(super::LocalSend, super::AirLynx)>| {
                    let Some((local, air_app)) = apps else {
                        crate::theme::install(ui.ctx());
                        crate::theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                        *apps = Some((super::LocalSend::new(ui.ctx().clone(), "Test Mac"), super::AirLynx::new(ui.ctx().clone(), "Test Mac")));
                        return;
                    };
                    let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                    super::device_dialog::draw(&mut window, &view, if air { air_app.logo() } else { local.logo() });
                },
                None,
            );
            for _ in 0..3 {
                harness.run();
            }
            let image = harness.render().expect("render");
            let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
            std::fs::create_dir_all(&out).unwrap();
            image.save(out.join(if air { "device-properties-airlynx.png" } else { "device-properties-localsend.png" })).unwrap();
        }
    }

    /// The Receive Files question, for a phone offering photos.
    #[test]
    #[ignore = "renders a screenshot; run with --ignored"]
    fn receive_offer() {
        let mut files: Vec<(String, u64)> = (0..7).map(|i| (format!("IMG_20241017_18{:04}.jpg", 3205 + i * 7), 2_400_000 + i * 310_000)).collect();
        files.insert(0, ("Holiday video.mp4".into(), 48_500_000));
        let view = super::offer_dialog::View { from: "Jan's Pixel".into(), detail: "Phone or tablet (Pixel 8) at 192.168.2.31".into(), files, folder: "Downloads".into() };
        let size = view.size();
        let mut harness = Harness::builder().with_size(size).with_pixels_per_point(2.0).wgpu().build_ui_state(
            |ui, app: &mut Option<super::LocalSend>| {
                let Some(local) = app else {
                    crate::theme::install(ui.ctx());
                    crate::theme::set_dark(ui.ctx(), std::env::var_os("SHOT_DARK").is_some());
                    *app = Some(super::LocalSend::new(ui.ctx().clone(), "Test Mac"));
                    return;
                };
                let mut window = ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()));
                super::offer_dialog::draw(&mut window, &view, local.logo());
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
