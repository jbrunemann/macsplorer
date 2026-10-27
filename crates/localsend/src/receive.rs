//! Receiving: the HTTP server other devices send to, and this side's
//! presence on the multicast group, so they find it.
//!
//! A [`Receiver`] serves until it is dropped:
//!
//! - `GET /info` and `POST /register` describe this side; a `/register`
//!   also tells it about the device that sent it ([`Event::Device`]).
//! - `POST /prepare-upload` is a device asking to send files. The request
//!   waits while the [`Event::Offer`] is shown; accepting answers with a
//!   session and a token per file, declining with 403. One transfer at a
//!   time: while one is asked or going, others get 409.
//! - `POST /upload` brings one file. It is written into the folder the
//!   receiver was given, under a name no file there has yet ("photo
//!   (1).jpg"); a folder sent along gets a new folder rather than mixing
//!   with one already there. Names are cleaned first ([`clean`]): `..`,
//!   absolute paths and control characters never reach the disk.
//! - `POST /cancel` from the sender ends the transfer, or takes back the
//!   question while it is still asked.
//!
//! Presence: it announces itself on the multicast group when it starts and
//! answers every device that announces itself, with a `POST /register` to
//! it or, when that fails, a multicast answer (the protocol's two ways).
//!
//! The port is [`PORT`] when it is free. Next to a LocalSend app on the
//! same computer it isn't, so the next free one is taken and announced;
//! devices that only sweep for [`PORT`] then don't find this side.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::discovery::{self, Interface};
use crate::http::{self, BodyReader, Request};
use crate::protocol::{API, Announcement, DeviceInfo, FileInfo, MULTICAST, PORT, PrepareResponse, PrepareUpload, Protocol};
use crate::{Device, Me};

/// Ports tried after [`PORT`] before any free one is taken.
const PORTS_AFTER: u16 = 10;
/// How long an idle connection is kept, and how long a stalled upload waits.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// A transfer that brings nothing for this long is given up.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How often waiting loops look again.
const TICK: Duration = Duration::from_millis(100);
/// Connections served at once; more are closed at once.
const MAX_CONNECTIONS: usize = 32;
/// The most a `prepare-upload` body may take: its file list.
const MAX_PREPARE: usize = 16 * 1024 * 1024;
/// A device's announcements are answered at most this often.
const ANSWER_EVERY: Duration = Duration::from_secs(5);
/// How long answering a device with `/register` may take.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(2);
/// The most parts a received path may have.
const MAX_DEPTH: usize = 32;
/// The longest file name macOS and Linux take, in bytes.
const MAX_NAME: usize = 255;

#[derive(Clone, Debug)]
pub struct Options {
    /// Where received files go.
    pub folder: PathBuf,
    /// How long a question waits for an answer before it is declined.
    pub ask_timeout: Duration,
    /// macOS: marks received files as downloaded by this app (the
    /// `com.apple.quarantine` attribute browsers set), so Gatekeeper checks
    /// a received app before it first opens. Ignored elsewhere.
    pub quarantine: Option<String>,
    /// The port to try first.
    pub port: u16,
    /// Announce this side and answer announcements.
    pub presence: bool,
}

impl Options {
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self { folder: folder.into(), ask_timeout: Duration::from_secs(120), quarantine: None, port: PORT, presence: true }
    }
}

/// What the receiver has to tell, on one of its threads.
pub enum Event {
    /// A device asks to send files: accept or decline it.
    Offer(Offer),
    /// A device announced or registered itself.
    Device(Device),
    /// A transfer is over.
    Done(Received),
}

/// A device's request to send files, waiting for an answer. Dropping it
/// declines.
pub struct Offer {
    pub from: DeviceInfo,
    pub ip: IpAddr,
    /// In name order.
    pub files: Vec<FileInfo>,
    reply: Sender<Option<Arc<Progress>>>,
    state: Arc<AtomicU8>,
}

/// Why an offer can no longer be answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closed {
    /// The sender cancelled or went away.
    Withdrawn,
    /// Nobody answered in time ([`Options::ask_timeout`]).
    Expired,
}

const OPEN: u8 = 0;
const WITHDRAWN: u8 = 1;
const EXPIRED: u8 = 2;

impl Offer {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// `Some` once the question is over without an answer.
    pub fn closed(&self) -> Option<Closed> {
        match self.state.load(Ordering::Relaxed) {
            WITHDRAWN => Some(Closed::Withdrawn),
            EXPIRED => Some(Closed::Expired),
            _ => None,
        }
    }

    /// Takes the files; the progress follows the transfer and cancels it.
    pub fn accept(self) -> Arc<Progress> {
        let progress = Arc::new(Progress::new(self.total_size()));
        let _ = self.reply.send(Some(progress.clone()));
        progress
    }

    pub fn decline(self) {
        let _ = self.reply.send(None);
    }
}

/// How far a transfer got, and its Cancel.
#[derive(Debug)]
pub struct Progress {
    total: u64,
    done: AtomicU64,
    cancelled: AtomicBool,
    /// The file coming in, as the sender named it.
    current: Mutex<String>,
}

impl Progress {
    fn new(total: u64) -> Self {
        Self { total, done: AtomicU64::new(0), cancelled: AtomicBool::new(false), current: Mutex::new(String::new()) }
    }

    /// Stops the transfer; the file coming in is deleted.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Bytes received and bytes accepted.
    pub fn bytes(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total)
    }

    /// The file coming in; empty before the first one.
    pub fn current(&self) -> String {
        self.current.lock().map(|c| c.clone()).unwrap_or_default()
    }

    fn set_current(&self, name: &str) {
        if let Ok(mut c) = self.current.lock() {
            name.clone_into(&mut c);
        }
    }
}

/// A finished transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    pub from: String,
    /// The files written, where they were written.
    pub saved: Vec<PathBuf>,
    /// Accepted files that didn't arrive.
    pub missing: usize,
    pub end: End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// Everything accepted arrived, or failed on its own (`missing`).
    Finished,
    /// Cancelled on this side ([`Progress::cancel`]).
    Cancelled,
    /// The sender cancelled.
    CancelledBySender,
    /// The sender stopped sending ([`IDLE_TIMEOUT`]).
    TimedOut,
}

/// The server, until dropped.
pub struct Receiver {
    shared: Arc<Shared>,
}

impl Receiver {
    /// Starts serving on worker threads. `me`'s port is replaced by the
    /// one taken ([`Receiver::me`]); with an identity it serves HTTPS.
    pub fn start(me: &Me, options: Options, events: impl Fn(Event) + Send + Sync + 'static) -> io::Result<Receiver> {
        let listener = bind(options.port)?;
        listener.set_nonblocking(true)?;
        let mut me = me.clone();
        me.port = listener.local_addr()?.port();
        #[cfg(feature = "tls")]
        let tls = match (&me.identity, me.protocol) {
            (Some(identity), Protocol::Https) => Some(tls_config(identity)?),
            _ => None,
        };
        #[cfg(not(feature = "tls"))]
        let tls: Option<Tls> = None;
        if tls.is_none() {
            me.protocol = Protocol::Http;
        }
        let shared = Arc::new(Shared {
            me,
            options,
            events: Box::new(events),
            state: Mutex::new(State::default()),
            stop: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
        });
        {
            let shared = shared.clone();
            thread::spawn(move || accept(&listener, tls, &shared));
        }
        if shared.options.presence {
            let shared = shared.clone();
            thread::spawn(move || presence(&shared));
        }
        Ok(Receiver { shared })
    }

    /// Who this side is while it receives: the port it took, the protocol
    /// it serves.
    pub fn me(&self) -> &Me {
        &self.shared.me
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(p) = self.shared.state.lock().ok().and_then(|s| s.session.as_ref().map(|s| s.progress.clone())) {
            p.cancel();
        }
    }
}

struct Shared {
    me: Me,
    options: Options,
    events: Box<dyn Fn(Event) + Send + Sync>,
    state: Mutex<State>,
    stop: AtomicBool,
    connections: AtomicUsize,
}

impl Shared {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Tells about a device, unless it is this side (a device announcing
    /// the port this side took answers `/register` with our own info).
    fn device(&self, device: Device) {
        if device.info.fingerprint != self.me.fingerprint {
            (self.events)(Event::Device(device));
        }
    }

    fn info_json(&self) -> Vec<u8> {
        serde_json::to_vec(&self.me.info()).unwrap_or_default()
    }

    /// Ends the session, once, and tells about it.
    fn finish(&self, end: End) {
        let session = self.state().session.take();
        if let Some(s) = session {
            let saved: Vec<PathBuf> = s.slots.values().filter(|f| f.state == Slot::DONE).map(|f| f.dest.clone()).collect();
            let missing = s.slots.len() - saved.len();
            let mut saved = saved;
            saved.sort();
            (self.events)(Event::Done(Received { from: s.from, saved, missing, end }));
        }
    }
}

#[derive(Default)]
struct State {
    /// A question being asked.
    asking: Option<(IpAddr, Arc<AtomicU8>)>,
    session: Option<Session>,
}

struct Session {
    id: String,
    ip: IpAddr,
    from: String,
    slots: HashMap<String, Slot>,
    progress: Arc<Progress>,
    /// When anything last arrived.
    last: Instant,
    /// The sender cancelled.
    stopped: Arc<AtomicBool>,
}

struct Slot {
    token: String,
    /// The name the sender gave.
    name: String,
    dest: PathBuf,
    /// As offered; a body that goes past it is refused.
    size: u64,
    modified: Option<SystemTime>,
    state: u8,
}

impl Slot {
    const WAITING: u8 = 0;
    const RECEIVING: u8 = 1;
    const DONE: u8 = 2;
    const FAILED: u8 = 3;
}

fn bind(port: u16) -> io::Result<TcpListener> {
    let any = |port| TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)));
    (port..port.saturating_add(PORTS_AFTER)).find_map(|p| any(p).ok()).map_or_else(|| any(0), Ok)
}

#[cfg(feature = "tls")]
fn tls_config(identity: &crate::Identity) -> io::Result<Arc<rustls::ServerConfig>> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let key = PrivateKeyDer::from_pem_slice(identity.key.as_bytes()).map_err(io::Error::other)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(identity.cert.clone())], key)
        .map_err(io::Error::other)?;
    Ok(Arc::new(config))
}

#[cfg(feature = "tls")]
type Tls = Arc<rustls::ServerConfig>;
/// Never there without TLS.
#[cfg(not(feature = "tls"))]
type Tls = Arc<()>;

/// Takes connections until stopped, and gives up on sessions that went
/// quiet or were cancelled between files.
fn accept(listener: &TcpListener, tls: Option<Tls>, shared: &Arc<Shared>) {
    while !shared.stopped() {
        match listener.accept() {
            Ok((stream, peer)) => {
                if shared.connections.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue;
                }
                shared.connections.fetch_add(1, Ordering::Relaxed);
                let (tls, shared) = (tls.clone(), shared.clone());
                thread::spawn(move || {
                    let _ = connection(stream, peer.ip(), tls, &shared);
                    shared.connections.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(TICK),
            Err(_) => thread::sleep(TICK),
        }
        let end = shared.state().session.as_ref().and_then(|s| {
            let busy = s.slots.values().any(|f| f.state == Slot::RECEIVING);
            if s.progress.is_cancelled() && !busy {
                Some(End::Cancelled)
            } else if !busy && s.last.elapsed() > IDLE_TIMEOUT {
                Some(End::TimedOut)
            } else {
                None
            }
        });
        if let Some(end) = end {
            shared.finish(end);
        }
    }
}

fn connection(stream: TcpStream, peer: IpAddr, tls: Option<Tls>, shared: &Shared) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    let probe = stream.try_clone()?;
    match tls {
        #[cfg(feature = "tls")]
        Some(config) => {
            let conn = rustls::ServerConnection::new(config).map_err(io::Error::other)?;
            let mut stream = BufReader::new(rustls::StreamOwned::new(conn, stream));
            let result = serve(&mut stream, peer, &probe, shared);
            let tls = stream.get_mut();
            tls.conn.send_close_notify();
            let _ = tls.flush();
            result
        }
        _ => serve(&mut BufReader::new(stream), peer, &probe, shared),
    }
}

/// Answers requests on one connection until either side closes it.
fn serve<S: Read + Write>(stream: &mut BufReader<S>, peer: IpAddr, probe: &TcpStream, shared: &Shared) -> io::Result<()> {
    while let Some(request) = http::read_head(stream)? {
        let kind = request.body()?;
        let mut body = BodyReader::new(stream, kind);
        let (status, answer) = route(&request, &mut body, peer, probe, shared);
        // An answer before the body is read ends the connection, since the
        // rest of the body would be taken for the next request.
        let keep = request.keep_alive() && body.finished() && !shared.stopped();
        http::respond(stream.get_mut(), status, &answer, keep)?;
        if !keep {
            break;
        }
    }
    Ok(())
}

fn route<R: io::BufRead>(request: &Request, body: &mut BodyReader<R>, peer: IpAddr, probe: &TcpStream, shared: &Shared) -> (u16, Vec<u8>) {
    let Some(route) = request.path.strip_prefix(API) else { return (404, Vec::new()) };
    match (request.method.as_str(), route) {
        ("GET", "/info") => (200, shared.info_json()),
        ("POST", "/register") => {
            if let Some(info) = read_body(body, 64 * 1024).and_then(|b| serde_json::from_slice::<DeviceInfo>(&b).ok()) {
                shared.device(Device::new(info, peer, PORT, Protocol::Https));
            }
            (200, shared.info_json())
        }
        ("POST", "/prepare-upload") => prepare(body, peer, probe, shared),
        ("POST", "/upload") => upload(request, body, peer, shared),
        ("POST", "/cancel") => {
            cancel(request.query("sessionId"), peer, shared);
            (200, Vec::new())
        }
        (_, "/info" | "/register" | "/prepare-upload" | "/upload" | "/cancel") => (405, Vec::new()),
        _ => (404, Vec::new()),
    }
}

fn read_body<R: io::BufRead>(body: &mut BodyReader<R>, limit: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    body.take(limit as u64 + 1).read_to_end(&mut out).ok()?;
    (out.len() <= limit).then_some(out)
}

/// `prepare-upload`: asks, waits for the answer, opens the session.
fn prepare<R: io::BufRead>(body: &mut BodyReader<R>, peer: IpAddr, probe: &TcpStream, shared: &Shared) -> (u16, Vec<u8>) {
    let Some(request) = read_body(body, MAX_PREPARE).and_then(|b| serde_json::from_slice::<PrepareUpload>(&b).ok()) else { return (400, Vec::new()) };
    let state = Arc::new(AtomicU8::new(OPEN));
    {
        let mut s = shared.state();
        if s.asking.is_some() || s.session.is_some() {
            return (409, Vec::new());
        }
        s.asking = Some((peer, state.clone()));
    }
    let mut files: Vec<FileInfo> = request.files.into_values().collect();
    files.sort_by(|a, b| a.file_name.cmp(&b.file_name).then_with(|| a.id.cmp(&b.id)));
    let from = request.info;
    let (reply, answer) = channel();
    (shared.events)(Event::Offer(Offer { from: from.clone(), ip: peer, files: files.clone(), reply, state: state.clone() }));

    let started = Instant::now();
    let progress = loop {
        match answer.recv_timeout(TICK) {
            Ok(answer) => break answer,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if state.load(Ordering::Relaxed) == WITHDRAWN || hung_up(probe) {
            state.store(WITHDRAWN, Ordering::Relaxed);
            break None;
        }
        if shared.stopped() || started.elapsed() > shared.options.ask_timeout {
            state.store(EXPIRED, Ordering::Relaxed);
            break None;
        }
    };
    let mut s = shared.state();
    s.asking = None;
    let Some(progress) = progress else { return (403, Vec::new()) };

    let names: Vec<&str> = files.iter().map(|f| f.file_name.as_str()).collect();
    let mut slots = HashMap::new();
    for (file, dest) in files.iter().zip(destinations(&shared.options.folder, &names)) {
        let Some(dest) = dest else { continue };
        let modified = file.metadata.as_ref().and_then(|m| m.modified.as_deref()).and_then(crate::time::parse);
        slots.insert(file.id.clone(), Slot { token: random_id(), name: file.file_name.clone(), dest, size: file.size, modified, state: Slot::WAITING });
    }
    let id = random_id();
    let tokens = slots.iter().map(|(k, v)| (k.clone(), v.token.clone())).collect();
    let answer = PrepareResponse { session_id: id.clone(), files: tokens };
    if slots.is_empty() {
        drop(s);
        (shared.events)(Event::Done(Received { from: from.alias, saved: Vec::new(), missing: files.len(), end: End::Finished }));
        return (403, Vec::new());
    }
    s.session = Some(Session { id, ip: peer, from: from.alias, slots, progress, last: Instant::now(), stopped: Arc::new(AtomicBool::new(false)) });
    (200, serde_json::to_vec(&answer).unwrap_or_default())
}

/// Whether the client closed its end (or sent anything, which while it
/// waits for an answer can only be TLS's goodbye).
#[cfg(unix)]
fn hung_up(probe: &TcpStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut fd = libc::pollfd { fd: probe.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd, no waiting.
    let n = unsafe { libc::poll(&mut fd, 1, 0) };
    n > 0 && fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0
}

#[cfg(not(unix))]
fn hung_up(_: &TcpStream) -> bool {
    false
}

fn upload<R: io::BufRead>(request: &Request, body: &mut BodyReader<R>, peer: IpAddr, shared: &Shared) -> (u16, Vec<u8>) {
    let (Some(session), Some(file), Some(token)) = (request.query("sessionId"), request.query("fileId"), request.query("token")) else { return (400, Vec::new()) };
    let (dest, name, size, modified, progress, stopped) = {
        let mut s = shared.state();
        let Some(current) = s.session.as_mut() else { return (403, Vec::new()) };
        if current.id != session {
            return (409, Vec::new());
        }
        if current.ip != peer {
            return (403, Vec::new());
        }
        let Some(slot) = current.slots.get_mut(file).filter(|f| f.token == token && f.state == Slot::WAITING) else { return (403, Vec::new()) };
        slot.state = Slot::RECEIVING;
        current.last = Instant::now();
        (slot.dest.clone(), slot.name.clone(), slot.size, slot.modified, current.progress.clone(), current.stopped.clone())
    };
    progress.set_current(&name);
    let result = write_file(&dest, body, size, modified, shared.options.quarantine.as_deref(), &progress, &stopped, &shared.state);
    if result.is_err() {
        let _ = fs::remove_file(&dest);
    }
    let all_done = {
        let mut s = shared.state();
        let Some(current) = s.session.as_mut().filter(|c| c.id == session) else { return (500, Vec::new()) };
        if let Some(slot) = current.slots.get_mut(file) {
            slot.state = if result.is_ok() { Slot::DONE } else { Slot::FAILED };
        }
        current.last = Instant::now();
        current.slots.values().all(|f| f.state >= Slot::DONE)
    };
    if progress.is_cancelled() {
        shared.finish(End::Cancelled);
    } else if stopped.load(Ordering::Relaxed) {
        shared.finish(End::CancelledBySender);
    } else if all_done {
        shared.finish(End::Finished);
    }
    match result {
        Ok(()) => (200, Vec::new()),
        Err(_) => (500, Vec::new()),
    }
}

/// Writes the body to `dest`, a name nothing has: stops on Cancel, on the
/// sender's cancel, on a broken connection and past `size`, so a file
/// accepted as small can't fill the disk.
#[allow(clippy::too_many_arguments)]
fn write_file<R: io::BufRead>(
    dest: &Path,
    body: &mut BodyReader<R>,
    size: u64,
    modified: Option<SystemTime>,
    quarantine: Option<&str>,
    progress: &Progress,
    stopped: &AtomicBool,
    state: &Mutex<State>,
) -> io::Result<()> {
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = File::create_new(dest)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut last_touch = Instant::now();
    let mut written = 0u64;
    loop {
        if progress.is_cancelled() || stopped.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > size {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "more than the size offered"));
        }
        file.write_all(&buf[..n])?;
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
        if last_touch.elapsed() > Duration::from_secs(1) {
            last_touch = Instant::now();
            if let Some(s) = state.lock().ok().as_mut().and_then(|s| s.session.as_mut()) {
                s.last = last_touch;
            }
        }
    }
    file.flush()?;
    if let Some(t) = modified {
        let _ = file.set_modified(t);
    }
    drop(file);
    if let Some(agent) = quarantine {
        mark_quarantined(dest, agent);
    }
    Ok(())
}

/// The sender's `/cancel`: takes back its question, or ends its transfer.
fn cancel(session: Option<&str>, peer: IpAddr, shared: &Shared) {
    let mut s = shared.state();
    if let Some((ip, state)) = &s.asking
        && *ip == peer
    {
        state.store(WITHDRAWN, Ordering::Relaxed);
        return;
    }
    let Some(current) = s.session.as_mut() else { return };
    if current.ip != peer || session.is_some_and(|id| id != current.id) {
        return;
    }
    current.stopped.store(true, Ordering::Relaxed);
    let busy = current.slots.values().any(|f| f.state == Slot::RECEIVING);
    drop(s);
    // An upload going on ends the session itself, once it has stopped.
    if !busy {
        shared.finish(End::CancelledBySender);
    }
}

#[cfg(target_os = "macos")]
fn mark_quarantined(path: &Path, agent: &str) {
    use std::os::unix::ffi::OsStrExt;
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // Flags 0081: downloaded, not yet opened (what Safari writes).
    let value = format!("0081;{secs:08x};{agent};");
    let (Ok(path), Ok(name)) = (std::ffi::CString::new(path.as_os_str().as_bytes()), std::ffi::CString::new("com.apple.quarantine")) else { return };
    // SAFETY: valid C strings and a buffer of the length given.
    unsafe {
        libc::setxattr(path.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0);
    }
}

#[cfg(not(target_os = "macos"))]
fn mark_quarantined(_: &Path, _: &str) {}

// ---- names ----------------------------------------------------------------

/// A received name as path parts that are safe to write: `/` and `\`
/// separate parts, empty and `.` parts are dropped, control characters
/// become `_`, a part longer than the file system takes is shortened
/// (keeping a short extension). `None` for a name with `..`, with nothing
/// left, or nested deeper than [`MAX_DEPTH`].
pub fn clean(name: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            _ => {}
        }
        let part: String = part.chars().map(|c| if c.is_control() { '_' } else { c }).collect();
        parts.push(shorten(&part, MAX_NAME));
    }
    (!parts.is_empty() && parts.len() <= MAX_DEPTH).then_some(parts)
}

/// `name` cut to `max` bytes at a character boundary, keeping an extension
/// of up to 16 bytes.
fn shorten(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_owned();
    }
    let ext = name.rfind('.').filter(|&i| i > 0 && name.len() - i <= 16).map_or("", |i| &name[i..]);
    let mut stem = &name[..name.len() - ext.len()];
    let mut end = max - ext.len();
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    stem = &stem[..end];
    format!("{stem}{ext}")
}

/// Where each of `names` goes inside `folder`, in order: a file under a
/// name nothing in `folder` has; the files of a sent folder in one new
/// folder ("Photos (1)" next to an existing "Photos"). `None` for a name
/// [`clean`] refuses.
pub fn destinations(folder: &Path, names: &[&str]) -> Vec<Option<PathBuf>> {
    let mut taken = HashSet::new();
    let mut folders: HashMap<String, PathBuf> = HashMap::new();
    names
        .iter()
        .map(|name| {
            let parts = clean(name)?;
            let dest = if parts.len() == 1 {
                unique(folder.join(&parts[0]), &taken)
            } else {
                let top = folders.entry(parts[0].clone()).or_insert_with(|| {
                    let top = unique(folder.join(&parts[0]), &taken);
                    taken.insert(top.clone());
                    top
                });
                let path = parts[1..].iter().fold(top.clone(), |p, part| p.join(part));
                if taken.contains(&path) { unique(path, &taken) } else { path }
            };
            taken.insert(dest.clone());
            Some(dest)
        })
        .collect()
}

/// `path`, or "name (1).ext", "name (2).ext"... if it exists or is taken.
fn unique(path: PathBuf, taken: &HashSet<PathBuf>) -> PathBuf {
    let free = |p: &Path| !taken.contains(p) && fs::symlink_metadata(p).is_err();
    if free(&path) {
        return path;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    (1..)
        .map(|n| path.with_file_name(shorten(&format!("{stem} ({n}){ext}"), MAX_NAME)))
        .find(|p| free(p))
        .unwrap_or(path)
}

/// A session id or token: 32 hex digits, from the system's random source
/// with TLS, otherwise as random as [`crate::random_fingerprint`].
fn random_id() -> String {
    #[cfg(feature = "tls")]
    {
        use ring::rand::SecureRandom;
        let mut bytes = [0u8; 16];
        if ring::rand::SystemRandom::new().fill(&mut bytes).is_ok() {
            return bytes.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    crate::random_fingerprint()
}

// ---- presence ---------------------------------------------------------------

/// Announces this side, then answers the devices that announce themselves.
fn presence(shared: &Arc<Shared>) {
    let interfaces = discovery::local_ipv4();
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (interfaces, stop) = (interfaces.clone(), stop.clone());
        let message = serde_json::to_vec(&Announcement { info: shared.me.info(), announce: true }).unwrap_or_default();
        thread::spawn(move || discovery::announce(&message, &interfaces, &stop));
    }
    let Ok(socket) = discovery::listener(&interfaces) else { return };
    let agent = crate::send::agent(&shared.me, Some(REGISTER_TIMEOUT));
    let mut answered: HashMap<String, Instant> = HashMap::new();
    let mut buf = [0u8; 8192];
    while !shared.stopped() {
        let Ok((n, from)) = socket.recv_from(&mut buf) else { continue };
        let Ok(a) = serde_json::from_slice::<Announcement>(&buf[..n]) else { continue };
        if a.info.fingerprint == shared.me.fingerprint {
            continue;
        }
        let device = Device::new(a.info, from.ip(), PORT, Protocol::Https);
        shared.device(device.clone());
        if !a.announce || answered.get(&device.key()).is_some_and(|t| t.elapsed() < ANSWER_EVERY) {
            continue;
        }
        answered.insert(device.key(), Instant::now());
        let (shared, agent, interfaces) = (shared.clone(), agent.clone(), interfaces.clone());
        thread::spawn(move || answer(&shared, &agent, &device, &interfaces));
    }
    stop.store(true, Ordering::Relaxed);
}

/// Tells an announcing device about this side: `POST /register`, or a
/// multicast answer when that doesn't get through.
fn answer(shared: &Shared, agent: &ureq::Agent, device: &Device, interfaces: &[Interface]) {
    let url = format!("{}/register", device.base_url());
    if let Ok(mut response) = agent.post(&url).send_json(shared.me.info())
        && response.status().is_success()
    {
        if let Ok(info) = response.body_mut().read_json::<DeviceInfo>() {
            shared.device(Device::new(info, device.ip, device.port, device.protocol));
        }
        return;
    }
    let message = serde_json::to_vec(&Announcement { info: shared.me.info(), announce: false }).unwrap_or_default();
    let group = SocketAddr::V4(SocketAddrV4::new(MULTICAST, PORT));
    for socket in interfaces.iter().filter_map(|i| discovery::sender(i.ip).ok()) {
        let _ = socket.send_to(&message, group);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("localsend-receive-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean("a.jpg").unwrap(), ["a.jpg"]);
        assert_eq!(clean("Photos/2024/a.jpg").unwrap(), ["Photos", "2024", "a.jpg"]);
        assert_eq!(clean("/etc/passwd").unwrap(), ["etc", "passwd"]);
        assert_eq!(clean("C:\\Users\\x.txt").unwrap(), ["C:", "Users", "x.txt"]);
        assert_eq!(clean("./a//b/").unwrap(), ["a", "b"]);
        assert_eq!(clean("bad\nname\u{7}.txt").unwrap(), ["bad_name_.txt"]);
        assert_eq!(clean(".hidden").unwrap(), [".hidden"]);
        assert_eq!(clean("..."), Some(vec!["...".to_owned()]));
        for refused in ["../x", "a/../../x", "..", "", "/", "./.", &"a/".repeat(40)] {
            assert_eq!(clean(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn long_names_are_shortened_keeping_the_extension() {
        let long = format!("{}.jpeg", "é".repeat(200));
        let short = &clean(&long).unwrap()[0];
        assert!(short.len() <= MAX_NAME, "{}", short.len());
        assert!(short.ends_with("é.jpeg"));
    }

    #[test]
    fn names_already_there_get_a_number() {
        let dir = temp("unique");
        fs::write(dir.join("a.jpg"), "").unwrap();
        fs::write(dir.join("a (1).jpg"), "").unwrap();
        fs::write(dir.join("notes"), "").unwrap();
        fs::create_dir(dir.join("Photos")).unwrap();
        let got = destinations(&dir, &["a.jpg", "b.txt", "b.txt", "notes", "Photos/x.jpg", "Photos/y/z.jpg", "Photos/x.jpg", "../evil", ".bashrc"]);
        let rel: Vec<Option<String>> = got.iter().map(|p| p.as_ref().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned())).collect();
        let want = ["a (2).jpg", "b.txt", "b (1).txt", "notes (1)", "Photos (1)/x.jpg", "Photos (1)/y/z.jpg", "Photos (1)/x (1).jpg"];
        for (got, want) in rel.iter().zip(want) {
            assert_eq!(got.as_deref(), Some(want));
        }
        assert_eq!(rel[7], None);
        assert_eq!(rel[8].as_deref(), Some(".bashrc"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_and_a_folder_of_one_name_both_fit() {
        let dir = temp("clash");
        let got = destinations(&dir, &["Docs/a.txt", "Docs"]);
        assert_eq!(got[0].as_ref().unwrap(), &dir.join("Docs/a.txt"));
        assert_eq!(got[1].as_ref().unwrap(), &dir.join("Docs (1)"));
        fs::remove_dir_all(dir).unwrap();
    }
}
