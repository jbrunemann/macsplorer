//! The worker that owns the receivers, LocalSend's and AirLynx's: it runs
//! them while Receive Files is on (`preferences::receive`) and this window
//! holds the lock, and passes on what they tell. AirLynx's is a side line:
//! it listens on its own ports (47800 and 47801), which the AirLynx app may
//! hold, and when it can't start LocalSend goes on without it.
//!
//! Every window is its own process, and only one of them can take the port
//! and answer for this computer, so the receiver goes with a lock file next
//! to `settings.json`. The other windows see that it is taken
//! ([`State::Elsewhere`]) and look again every [`RECHECK`], so when the
//! window that has it closes, another one takes over. The setting is read
//! again just as often, which carries a change made in one window to the
//! others. All of this runs off the UI thread: it reads and writes files.

use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Duration;

use airlynx_core::receive::{self as air, ReceiveEvent, ReceiverConfig, ReceiverHandle};
use airlynx_core::{Favorites, Identity};
use eframe::egui;
use localsend::Me;
use localsend::receive::{self, Event, Options};
use macsplorer_core::preferences::{self, Preferences};

/// How often the setting and the lock are looked at again.
const RECHECK: Duration = Duration::from_secs(2);
const LOCK_FILE: &str = "localsend.lock";

/// Where receiving stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    /// This window receives, on `port`; `me` is this side as announced.
    On(Box<Me>),
    /// Another Macsplorer window receives for this computer.
    Elsewhere,
    /// Receive Files is on, but the receiver couldn't start.
    Failed(String),
}

impl State {
    /// Whether this computer takes files, from this window or another.
    pub fn receiving(&self) -> bool {
        matches!(self, State::On(_) | State::Elsewhere)
    }
}

pub enum Msg {
    /// Receive Files as set, and where that stands.
    State { wanted: bool, state: State },
    Event(Event),
    /// An AirLynx favorite is sending.
    Air(ReceiveEvent),
    /// The AirLynx receiver couldn't start, and why.
    AirUnavailable(String),
}

/// This computer for AirLynx: who it is and who may send.
#[derive(Clone)]
struct Air {
    me: Identity,
    favorites: Favorites,
}

pub struct Supervisor {
    turn: Sender<Option<bool>>,
}

impl Supervisor {
    /// Starts the worker; `tx` gets what it tells, with a repaint.
    pub fn start(me: Me, air_me: Identity, favorites: Favorites, tx: Sender<Msg>, ctx: egui::Context) -> Self {
        let (turn, turns) = channel();
        let air = Air { me: air_me, favorites };
        thread::spawn(move || run(&me, &air, &tx, &turns, &ctx));
        Self { turn }
    }

    /// Turns Receive Files on or off, for every window.
    pub fn set(&self, on: bool) {
        let _ = self.turn.send(Some(on));
    }
}

/// The folder received files go to.
pub fn downloads() -> PathBuf {
    dirs::download_dir().or_else(|| dirs::home_dir().map(|h| h.join("Downloads"))).unwrap_or_else(|| PathBuf::from("."))
}

fn run(me: &Me, air: &Air, tx: &Sender<Msg>, turns: &Receiver<Option<bool>>, ctx: &egui::Context) {
    let send = |msg: Msg| {
        let ok = tx.send(msg).is_ok();
        ctx.request_repaint();
        ok
    };
    let mut running: Option<(receive::Receiver, File, Option<ReceiverHandle>)> = None;
    let mut last: Option<(bool, State)> = None;
    loop {
        let wanted = Preferences::load().receive;
        if !wanted {
            running = None;
        }
        let state = match &running {
            Some((r, ..)) => State::On(Box::new(r.me().clone())),
            None if !wanted => State::Off,
            None => match start(me, air, tx, ctx) {
                Ok(Some(started)) => {
                    let state = State::On(Box::new(started.0.me().clone()));
                    running = Some(started);
                    state
                }
                Ok(None) => State::Elsewhere,
                Err(e) => State::Failed(e.to_string()),
            },
        };
        if last.as_ref() != Some(&(wanted, state.clone())) {
            last = Some((wanted, state.clone()));
            if !send(Msg::State { wanted, state }) {
                return;
            }
        }
        match turns.recv_timeout(RECHECK) {
            Ok(Some(on)) => Preferences::update(|p| p.receive = on),
            Ok(None) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Takes the lock and starts the receiver; `None` while another window
/// holds the lock.
fn start(me: &Me, air: &Air, tx: &Sender<Msg>, ctx: &egui::Context) -> std::io::Result<Option<(receive::Receiver, File, Option<ReceiverHandle>)>> {
    let dir = preferences::config_dir().ok_or_else(|| std::io::Error::other("no config folder"))?;
    std::fs::create_dir_all(&dir)?;
    let lock = File::options().create(true).truncate(false).write(true).open(dir.join(LOCK_FILE))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
        Err(std::fs::TryLockError::Error(e)) => return Err(e),
    }
    let folder = downloads();
    std::fs::create_dir_all(&folder)?;
    let options = Options { quarantine: Some("Macsplorer".into()), ..Options::new(folder.clone()) };
    let (event_tx, event_ctx) = (tx.clone(), ctx.clone());
    let receiver = receive::Receiver::start(me, options, move |event| {
        if event_tx.send(Msg::Event(event)).is_ok() {
            event_ctx.request_repaint();
        }
    })?;
    let config = ReceiverConfig {
        dir: folder,
        port: airlynx_core::DEFAULT_TCP_PORT,
        identity: air.me.clone(),
        favorites: air.favorites.clone(),
        poll_interval: air::DEFAULT_POLL_INTERVAL,
    };
    let (event_tx, event_ctx) = (tx.clone(), ctx.clone());
    let airlynx = match air::start(config, move |event| {
        if event_tx.send(Msg::Air(event)).is_ok() {
            event_ctx.request_repaint();
        }
    }) {
        Ok(handle) => Some(handle),
        Err(e) => {
            let _ = tx.send(Msg::AirUnavailable(e.to_string()));
            ctx.request_repaint();
            None
        }
    };
    Ok(Some((receiver, lock, airlynx)))
}
