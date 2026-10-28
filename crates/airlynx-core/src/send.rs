use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::identity::Identity;
use crate::protocol::*;
use crate::secure::Ephemeral;

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub file_index: usize,
    pub file_count: usize,
    pub file_name: String,
    pub file_sent: u64,
    pub file_size: u64,
    pub total_sent: u64,
    pub total_size: u64,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub sent: Vec<String>,
    /// Source paths of the files in `sent`, in the same order.
    pub sent_paths: Vec<PathBuf>,
    pub failed: Vec<String>,
    pub bytes: u64,
    pub elapsed: Duration,
}

/// Checks that every path is a readable regular file with a UTF-8 name.
pub fn prepare(paths: impl IntoIterator<Item = PathBuf>) -> io::Result<Vec<Entry>> {
    paths
        .into_iter()
        .map(|path| {
            let meta = std::fs::metadata(&path)
                .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
            if !meta.is_file() {
                return Err(invalid(format!("{} is not a regular file", path.display())));
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| invalid(format!("{} has no valid UTF-8 name", path.display())))?
                .to_string();
            Ok(Entry { path, name, size: meta.len() })
        })
        .collect()
}

/// Sends `entries` to `addr` as `me`. The receiver must have `me` as a favorite,
/// otherwise this fails with `ErrorKind::PermissionDenied`. `on_progress` is
/// called after every chunk; returning `ControlFlow::Break` cancels the transfer.
pub fn send_files(
    addr: SocketAddr,
    me: &Identity,
    entries: &[Entry],
    mut on_progress: impl FnMut(&Progress) -> ControlFlow<()>,
) -> io::Result<Report> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;

    let ephemeral = Ephemeral::new()?;
    stream.write_all(MAGIC)?;
    stream.write_all(&[VERSION])?;
    stream.write_all(&ephemeral.public())?;
    if read_u8(&mut stream)? != STATUS_OK {
        return Err(invalid("receiver rejected the handshake (version mismatch?)"));
    }
    let mut peer_public = [0u8; 32];
    stream.read_exact(&mut peer_public)?;
    let mut stream = ephemeral.into_stream(stream, peer_public, true)?;

    write_hello(&mut stream, me)?;
    match read_u8(&mut stream)? {
        STATUS_OK => {}
        STATUS_NOT_FAVORITE => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the receiver has not added this device as a favorite",
            ));
        }
        _ => return Err(invalid("receiver rejected the handshake")),
    }
    stream.write_all(&(entries.len() as u32).to_be_bytes())?;

    let started = Instant::now();
    let mut progress = Progress {
        file_index: 0,
        file_count: entries.len(),
        file_name: String::new(),
        file_sent: 0,
        file_size: 0,
        total_sent: 0,
        total_size: entries.iter().map(|e| e.size).sum(),
    };
    let mut report = Report {
        sent: Vec::new(),
        sent_paths: Vec::new(),
        failed: Vec::new(),
        bytes: 0,
        elapsed: Duration::ZERO,
    };

    for (i, entry) in entries.iter().enumerate() {
        progress.file_index = i;
        progress.file_name.clone_from(&entry.name);
        progress.file_sent = 0;
        progress.file_size = entry.size;

        write_file_header(&mut stream, &entry.name, entry.size)?;
        let hash = stream_file(&mut stream, entry, &mut progress, &mut on_progress)?;
        stream.write_all(&hash)?;
        match read_u8(&mut stream)? {
            STATUS_OK => {
                report.bytes += entry.size;
                report.sent.push(entry.name.clone());
                report.sent_paths.push(entry.path.clone());
            }
            STATUS_HASH_MISMATCH => report.failed.push(entry.name.clone()),
            other => return Err(invalid(format!("unexpected status {other}"))),
        }
    }

    report.elapsed = started.elapsed();
    Ok(report)
}

/// Writes the file body to `stream` and returns its SHA-256.
fn stream_file(
    stream: &mut impl Write,
    entry: &Entry,
    progress: &mut Progress,
    on_progress: &mut impl FnMut(&Progress) -> ControlFlow<()>,
) -> io::Result<[u8; 32]> {
    let mut file = File::open(&entry.path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut remaining = entry.size;

    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            return Err(invalid(format!("{} shrank while sending", entry.name)));
        }
        hasher.update(&buf[..n]);
        stream.write_all(&buf[..n])?;
        remaining -= n as u64;
        progress.file_sent += n as u64;
        progress.total_sent += n as u64;
        if on_progress(progress).is_break() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
    }
    Ok(hasher.finalize().into())
}

/// Accepts "host:port", "ip:port", or a bare host/IP (uses the default port).
pub fn resolve(target: &str) -> io::Result<SocketAddr> {
    let target = target.trim();
    let with_port = if target.contains(':') {
        target.to_string()
    } else {
        format!("{target}:{DEFAULT_TCP_PORT}")
    };
    with_port
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| invalid(format!("could not resolve {target}")))
}
