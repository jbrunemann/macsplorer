//! Sending files and folders to a device: `prepare-upload` with the list of
//! files, which waits until someone on the device accepts, then one
//! `upload` per file the device took.
//!
//! A folder goes as the files inside it, each named by its path from the
//! folder's parent ("Photos/2024/a.jpg"), which the official app saves as
//! that folder. Empty folders and anything that isn't a file or a folder
//! (sockets, broken links) are left out; links are followed.
//!
//! The devices' certificates are self-signed, so they are not verified.
//! (The device's fingerprint is its certificate's SHA-256 and could be
//! pinned; not done yet.)

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::protocol::{FileInfo, PrepareResponse, PrepareUpload};
use crate::{Device, Me, mime};

/// How long to wait for a device to take the connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Guards against link loops while listing folders.
const MAX_DEPTH: usize = 64;

/// An HTTP client for LocalSend devices: certificates are not verified,
/// `me`'s certificate goes to receivers that ask for one, error statuses
/// come back as responses. `timeout` limits each whole request;
/// `prepare-upload` needs none, since it waits for a person.
/// Without the `tls` feature only devices with encryption off are reached.
pub fn agent(me: &Me, timeout: Option<Duration>) -> ureq::Agent {
    let config = ureq::Agent::config_builder();
    #[cfg(feature = "tls")]
    let config = {
        use ureq::tls::{Certificate, ClientCert, PrivateKey, TlsConfig};
        let client = me.identity.as_ref().and_then(|i| {
            let key = PrivateKey::from_pem(i.key.as_bytes()).ok()?;
            Some(ClientCert::new_with_certs(&[Certificate::from_der(&i.cert).to_owned()], key))
        });
        config.tls_config(TlsConfig::builder().disable_verification(true).client_cert(client).build())
    };
    #[cfg(not(feature = "tls"))]
    let _ = me;
    config
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(timeout)
        .build()
        .new_agent()
}

/// One file to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub path: PathBuf,
    /// The name the device saves it under, `/`-separated.
    pub name: String,
    pub size: u64,
}

/// The files that sending `paths` sends, in order: files as themselves,
/// folders as the files inside them.
pub fn plan(paths: &[PathBuf]) -> io::Result<Vec<Outgoing>> {
    let mut out = Vec::new();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        let meta = fs::metadata(path)?;
        if meta.is_dir() {
            walk(path, &name, 0, &mut out)?;
        } else if meta.is_file() {
            out.push(Outgoing { path: path.clone(), name, size: meta.len() });
        }
    }
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<Outgoing>) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let name = format!("{prefix}/{}", e.file_name().to_string_lossy());
        let Ok(meta) = fs::metadata(&path) else { continue };
        if meta.is_dir() {
            walk(&path, &name, depth + 1, out)?;
        } else if meta.is_file() {
            out.push(Outgoing { path, name, size: meta.len() });
        }
    }
    Ok(())
}

/// Where a send is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Listing the files.
    Preparing,
    /// The device was asked; it waits for someone to accept.
    Waiting,
    /// Sending this file (its name on the device).
    Sending(String),
}

/// How far a [`send`] got, shared with whoever shows it, and its Cancel.
#[derive(Debug)]
pub struct Progress {
    total: AtomicU64,
    done: AtomicU64,
    cancelled: AtomicBool,
    stage: Mutex<Stage>,
}

impl Default for Progress {
    fn default() -> Self {
        Self { total: AtomicU64::new(0), done: AtomicU64::new(0), cancelled: AtomicBool::new(false), stage: Mutex::new(Stage::Preparing) }
    }
}

impl Progress {
    /// Stops the send: within the file being sent, or as soon as the device
    /// answers when it is waiting for someone to accept.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Bytes sent and bytes to send (0 until the files are listed).
    pub fn bytes(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }

    pub fn stage(&self) -> Stage {
        self.stage.lock().map_or(Stage::Preparing, |s| s.clone())
    }

    fn set_stage(&self, stage: Stage) {
        if let Ok(mut s) = self.stage.lock() {
            *s = stage;
        }
    }
}

/// How a send ended, other than with everything sent.
#[derive(Debug)]
pub enum SendError {
    Cancelled,
    /// Someone on the device said no (403).
    Rejected,
    /// The device asks for a PIN, or the one given was wrong (401).
    PinRequired,
    /// The device is busy with another transfer (409).
    Busy,
    /// Too many requests (429).
    TooManyRequests,
    /// The device took none of the files.
    NothingAccepted,
    /// Another status from the device.
    Status(u16),
    /// The device couldn't be reached, or the connection broke.
    Network(String),
    /// A file couldn't be read.
    Io(PathBuf, io::Error),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Cancelled => write!(f, "cancelled"),
            SendError::Rejected => write!(f, "the transfer was declined"),
            SendError::PinRequired => write!(f, "the device asks for a PIN, which isn't supported yet"),
            SendError::Busy => write!(f, "the device is busy with another transfer"),
            SendError::TooManyRequests => write!(f, "the device got too many requests, try again later"),
            SendError::NothingAccepted => write!(f, "the device didn't accept any of the files"),
            SendError::Status(s) => write!(f, "the device answered with error {s}"),
            SendError::Network(e) => write!(f, "{e}"),
            SendError::Io(path, e) => write!(f, "couldn't read \"{}\": {e}", path.display()),
        }
    }
}

impl std::error::Error for SendError {}

/// What got there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sent {
    pub files: usize,
    /// Files the device turned down while taking others.
    pub declined: usize,
}

/// Sends `paths` (files and folders) to `device`, blocking until it is
/// over. `pin` answers a device that asks for one.
pub fn send(me: &Me, device: &Device, paths: &[PathBuf], pin: Option<&str>, progress: &Progress) -> Result<Sent, SendError> {
    let files = plan(paths).map_err(|e| SendError::Io(paths.first().cloned().unwrap_or_default(), e))?;
    progress.total.store(files.iter().map(|f| f.size).sum::<u64>().max(1), Ordering::Relaxed);
    let body = PrepareUpload { info: me.info(), files: files.iter().enumerate().map(|(i, f)| (i.to_string(), file_info(i, f))).collect() };

    progress.set_stage(Stage::Waiting);
    let base = device.base_url();
    let agent = agent(me, None);
    let mut url = format!("{base}/prepare-upload");
    if let Some(pin) = pin {
        url = format!("{url}?pin={}", encode(pin));
    }
    let mut response = agent.post(&url).send_json(&body).map_err(network)?;
    let session: PrepareResponse = match response.status().as_u16() {
        200 => response.body_mut().read_json().map_err(network)?,
        204 => return Ok(Sent::default()),
        401 => return Err(SendError::PinRequired),
        403 => return Err(SendError::Rejected),
        409 => return Err(SendError::Busy),
        429 => return Err(SendError::TooManyRequests),
        s => return Err(SendError::Status(s)),
    };
    let cancel = |agent: &ureq::Agent| {
        let _ = agent.post(format!("{base}/cancel?sessionId={}", encode(&session.session_id))).send_empty();
    };
    if progress.is_cancelled() {
        cancel(&agent);
        return Err(SendError::Cancelled);
    }
    if session.files.is_empty() {
        return Err(SendError::NothingAccepted);
    }

    // Declined files count as done, so the bar still reaches the end.
    let declined = files.iter().enumerate().filter(|(i, _)| !session.files.contains_key(&i.to_string())).map(|(_, f)| f.size).sum();
    progress.done.fetch_add(declined, Ordering::Relaxed);
    let mut sent = Sent { files: 0, declined: files.len() - session.files.len() };
    for (i, file) in files.iter().enumerate() {
        let id = i.to_string();
        let Some(token) = session.files.get(&id) else { continue };
        progress.set_stage(Stage::Sending(file.name.clone()));
        let result = upload(&agent, &base, &session.session_id, &id, token, file, progress);
        if let Err(e) = result {
            cancel(&agent);
            return Err(if progress.is_cancelled() { SendError::Cancelled } else { e });
        }
        sent.files += 1;
    }
    Ok(sent)
}

fn upload(agent: &ureq::Agent, base: &str, session: &str, id: &str, token: &str, file: &Outgoing, progress: &Progress) -> Result<(), SendError> {
    let reader = File::open(&file.path).map_err(|e| SendError::Io(file.path.clone(), e))?;
    let mut reader = Counted { inner: reader, progress };
    let url = format!("{base}/upload?sessionId={}&fileId={}&token={}", encode(session), encode(id), encode(token));
    let response = agent
        .post(&url)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", file.size.to_string())
        .send(ureq::SendBody::from_reader(&mut reader))
        .map_err(network)?;
    match response.status().as_u16() {
        200..=299 => Ok(()),
        403 => Err(SendError::Rejected),
        409 => Err(SendError::Busy),
        s => Err(SendError::Status(s)),
    }
}

fn file_info(i: usize, f: &Outgoing) -> FileInfo {
    FileInfo { id: i.to_string(), file_name: f.name.clone(), size: f.size, file_type: mime::of(&f.name).into(), sha256: None, preview: None, metadata: None }
}

fn network(e: ureq::Error) -> SendError {
    SendError::Network(match e {
        ureq::Error::Io(e) => e.to_string(),
        ureq::Error::Timeout(_) => "the device didn't answer".into(),
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => "the device couldn't be reached".into(),
        e => e.to_string(),
    })
}

/// A file being read for the upload: counts what was read and stops on
/// Cancel.
struct Counted<'a> {
    inner: File,
    progress: &'a Progress,
}

impl Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = self.inner.read(buf)?;
        self.progress.done.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Percent-encodes a query value.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("localsend-client-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn folders_are_sent_as_their_files_with_paths() {
        let dir = temp("plan");
        fs::write(dir.join("top.txt"), "abc").unwrap();
        fs::create_dir_all(dir.join("Photos/2024")).unwrap();
        fs::create_dir_all(dir.join("Photos/empty")).unwrap();
        fs::write(dir.join("Photos/b.jpg"), "1234").unwrap();
        fs::write(dir.join("Photos/2024/a.jpg"), "12").unwrap();
        let plan = plan(&[dir.join("top.txt"), dir.join("Photos")]).unwrap();
        let names: Vec<_> = plan.iter().map(|f| (f.name.as_str(), f.size)).collect();
        assert_eq!(names, [("top.txt", 3), ("Photos/2024/a.jpg", 2), ("Photos/b.jpg", 4)]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn query_values_are_encoded() {
        assert_eq!(encode("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(encode("Ab-1_.~"), "Ab-1_.~");
    }
}
