use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::discovery;
use crate::identity::{DeviceId, Favorites, Identity};
use crate::protocol::*;
use crate::secure::Ephemeral;
use crate::stats;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Emit a progress event roughly every this many bytes.
const PROGRESS_STEP: u64 = 512 * 1024;

/// How long the accept loop sleeps between checks. The discovery loop waits twice as long for a packet.
/// Both only bound how quickly the receiver notices it was stopped, apart from the accept loop also
/// delaying a new connection by up to this long. Longer means fewer wakeups, which saves battery.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    pub dir: PathBuf,
    pub port: u16,
    pub identity: Identity,
    /// Only these devices may send. Shared, so changes apply to the running receiver.
    pub favorites: Favorites,
    pub poll_interval: Duration,
}

/// Everything a receiver reports. `peer` is the sender's "ip:port".
#[derive(Debug, Clone)]
pub enum ReceiveEvent {
    /// A device that is not a favorite tried to send. Nothing was accepted.
    Refused { peer: String, id: DeviceId, name: String },
    Connected { peer: String, files: u32 },
    FileStarted { peer: String, name: String, size: u64 },
    FileProgress { peer: String, name: String, received: u64, size: u64 },
    FileSaved { peer: String, path: PathBuf, size: u64 },
    FileRejected { peer: String, name: String, reason: String },
    SessionEnded { peer: String, error: Option<String> },
}

/// A running receiver. Stops listening when `stop` is called or it is dropped.
/// Transfers already in progress run to completion.
pub struct ReceiverHandle {
    port: u16,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl ReceiverHandle {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for ReceiverHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Binds the TCP and discovery ports and starts accepting transfers in the background.
pub fn start(
    config: ReceiverConfig,
    on_event: impl Fn(ReceiveEvent) + Send + Sync + 'static,
) -> io::Result<ReceiverHandle> {
    fs::create_dir_all(&config.dir)?;
    let listener = TcpListener::bind(("0.0.0.0", config.port))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let udp = discovery::bind_responder(config.poll_interval * 2)?;

    let stop = Arc::new(AtomicBool::new(false));
    let responder = discovery::spawn_responder(
        udp,
        port,
        config.identity.clone(),
        config.favorites.clone(),
        stop.clone(),
    );

    let on_event: Arc<dyn Fn(ReceiveEvent) + Send + Sync> = Arc::new(on_event);
    let accept_stop = stop.clone();
    let dir = config.dir;
    let favorites = config.favorites;
    let poll_interval = config.poll_interval;
    let acceptor = thread::spawn(move || {
        while !accept_stop.load(Ordering::Relaxed) {
            stats::accept_poll();
            match listener.accept() {
                Ok((stream, addr)) => {
                    stats::session();
                    let dir = dir.clone();
                    let on_event = on_event.clone();
                    let favorites = favorites.clone();
                    thread::spawn(move || {
                        let peer = addr.to_string();
                        let result = handle_session(stream, &dir, &peer, &favorites, &*on_event);
                        on_event(ReceiveEvent::SessionEnded {
                            peer,
                            error: result.err().map(|e| e.to_string()),
                        });
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(poll_interval);
                }
                Err(_) => thread::sleep(poll_interval),
            }
        }
    });

    Ok(ReceiverHandle {
        port,
        stop,
        threads: vec![responder, acceptor],
    })
}

fn handle_session(
    mut stream: TcpStream,
    dir: &Path,
    peer: &str,
    favorites: &Favorites,
    on_event: &(dyn Fn(ReceiveEvent) + Send + Sync),
) -> io::Result<()> {
    // Accepted sockets can inherit the listener's non-blocking mode.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;

    let mut magic = [0u8; 4];
    stream.read_exact(&mut magic)?;
    let version = read_u8(&mut stream)?;
    if &magic != MAGIC || version != VERSION {
        stream.write_all(&[STATUS_REJECTED])?;
        return Err(invalid(format!("bad handshake (version {version})")));
    }
    let mut peer_public = [0u8; 32];
    stream.read_exact(&mut peer_public)?;
    let ephemeral = Ephemeral::new()?;
    stream.write_all(&[STATUS_OK])?;
    stream.write_all(&ephemeral.public())?;
    let mut stream = ephemeral.into_stream(stream, peer_public, false)?;

    let sender = read_hello(&mut stream)?;
    if !favorites.contains(sender.id) {
        stream.write_all(&[STATUS_NOT_FAVORITE])?;
        stream.flush()?;
        on_event(ReceiveEvent::Refused { peer: peer.into(), id: sender.id, name: sender.name });
        return Ok(());
    }
    stream.write_all(&[STATUS_OK])?;
    stream.flush()?;

    let count = read_u32(&mut stream)?;
    on_event(ReceiveEvent::Connected { peer: peer.into(), files: count });

    for _ in 0..count {
        let name_len = read_u16(&mut stream)? as usize;
        if name_len == 0 || name_len > MAX_NAME_LEN {
            return Err(invalid("file name length out of range"));
        }
        let mut name_buf = vec![0u8; name_len];
        stream.read_exact(&mut name_buf)?;
        let raw_name = String::from_utf8(name_buf).map_err(|_| invalid("file name is not UTF-8"))?;
        let size = read_u64(&mut stream)?;
        let name = sanitize_name(&raw_name)?;
        on_event(ReceiveEvent::FileStarted { peer: peer.into(), name: name.clone(), size });

        let tmp_path = dir.join(format!(
            ".{name}.{}.{}.part",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let progress = |received| {
            on_event(ReceiveEvent::FileProgress { peer: peer.into(), name: name.clone(), received, size });
        };
        let (actual, expected) = match receive_file(&mut stream, &tmp_path, size, progress) {
            Ok(hashes) => hashes,
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(e);
            }
        };

        if actual == expected {
            let final_path = unique_path(dir, &name);
            fs::rename(&tmp_path, &final_path)?;
            stream.write_all(&[STATUS_OK])?;
            stream.flush()?;
            on_event(ReceiveEvent::FileSaved { peer: peer.into(), path: final_path, size });
        } else {
            let _ = fs::remove_file(&tmp_path);
            stream.write_all(&[STATUS_HASH_MISMATCH])?;
            stream.flush()?;
            on_event(ReceiveEvent::FileRejected {
                peer: peer.into(),
                name,
                reason: "checksum mismatch".into(),
            });
        }
    }
    Ok(())
}

/// Streams `size` bytes into `path`, then reads the sender's hash.
/// Returns (computed hash, sender hash).
fn receive_file(
    stream: &mut impl Read,
    path: &Path,
    size: u64,
    mut progress: impl FnMut(u64),
) -> io::Result<([u8; 32], [u8; 32])> {
    let mut file = File::create(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut received = 0u64;
    let mut next_report = PROGRESS_STEP;

    while received < size {
        let want = (size - received).min(buf.len() as u64) as usize;
        let n = stream.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed mid-file"));
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        received += n as u64;
        if received >= next_report || received == size {
            progress(received);
            next_report = received + PROGRESS_STEP;
        }
    }
    file.sync_all()?;

    let mut expected = [0u8; 32];
    stream.read_exact(&mut expected)?;
    Ok((hasher.finalize().into(), expected))
}

/// Reduces a sender-supplied name to a plain file name so it cannot escape `dir`.
fn sanitize_name(raw: &str) -> io::Result<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err(invalid(format!("unusable file name {raw:?}")));
    }
    Ok(cleaned.to_string())
}

/// Returns `dir/name`, or `dir/stem (n).ext` if that already exists.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = path.extension().and_then(|e| e.to_str());
    (1..)
        .map(|i| match ext {
            Some(ext) => dir.join(format!("{stem} ({i}).{ext}")),
            None => dir.join(format!("{stem} ({i})")),
        })
        .find(|p| !p.exists())
        .expect("unbounded range")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_paths() {
        assert_eq!(sanitize_name("../../etc/passwd").unwrap(), "passwd");
        assert_eq!(sanitize_name("C:\\evil\\a.txt").unwrap(), "a.txt");
        assert_eq!(sanitize_name("photo.jpg").unwrap(), "photo.jpg");
        assert!(sanitize_name("..").is_err());
        assert!(sanitize_name("dir/").is_err());
    }

    use crate::identity::{DeviceId, DeviceKind};
    use crate::send::{self, Entry};
    use std::net::TcpListener;
    use std::ops::ControlFlow;
    use std::sync::Mutex;

    /// Runs one receiver session on loopback and returns the sender's result
    /// and the receiver's events.
    fn loopback(favorites: &Favorites, sender: &Identity) -> (io::Result<send::Report>, Vec<ReceiveEvent>) {
        let dir = std::env::temp_dir().join(format!("airlynx-recv-{}", DeviceId::random()));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("hello.txt");
        fs::write(&src, b"hello favorites").unwrap();
        let entries = vec![Entry { path: src, name: "hello.txt".into(), size: 15 }];

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let server = {
            let (favorites, events, dir) = (favorites.clone(), events.clone(), dir.join("in"));
            fs::create_dir_all(&dir).unwrap();
            thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let sink = |e| events.lock().unwrap().push(e);
                let _ = handle_session(stream, &dir, "test", &favorites, &sink);
            })
        };
        let result = send::send_files(addr, sender, &entries, |_| ControlFlow::Continue(()));
        server.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let events = std::mem::take(&mut *events.lock().unwrap());
        (result, events)
    }

    #[test]
    fn favorite_can_send() {
        let me = Identity::new(DeviceId::random(), "Phone");
        let favorites = Favorites::in_memory();
        favorites.add(me.id, "Phone", DeviceKind::Unknown).unwrap();
        let (result, events) = loopback(&favorites, &me);
        assert_eq!(result.unwrap().sent, ["hello.txt"]);
        assert!(events.iter().any(|e| matches!(e, ReceiveEvent::FileSaved { .. })));
    }

    #[test]
    fn stranger_is_refused() {
        let me = Identity::new(DeviceId::random(), "Phone");
        let (result, events) = loopback(&Favorites::in_memory(), &me);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(matches!(&events[..], [ReceiveEvent::Refused { id, name, .. }] if *id == me.id && name == "Phone"));
    }
}
