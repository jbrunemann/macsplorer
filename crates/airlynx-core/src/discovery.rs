//! UDP broadcast discovery: devices broadcast "AIRLYNX_DISCOVER <id>" and every
//! other running device answers "AIRLYNX_HERE <tcp_port> <id> <mutual> <kind> <name>".
//! `mutual` is 1 when the answering device has the asker as a favorite.

use std::collections::HashSet;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::identity::{DeviceId, DeviceKind, Favorites, Identity, clean_name};
use crate::protocol::{ANNOUNCE_PREFIX, DISCOVER_PREFIX, DISCOVERY_PORT};
use crate::stats;

#[derive(Debug, Clone)]
pub struct Peer {
    pub addr: SocketAddr,
    pub id: DeviceId,
    pub name: String,
    /// The peer has us as a favorite.
    pub mutual: bool,
    pub kind: DeviceKind,
}

/// Binds the discovery port. Done separately from `spawn_responder` so bind
/// errors surface to the caller instead of inside a background thread.
pub(crate) fn bind_responder(poll_interval: Duration) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT))?;
    socket.set_read_timeout(Some(poll_interval))?;
    Ok(socket)
}

/// Answers discovery broadcasts until `stop` is set.
pub(crate) fn spawn_responder(
    socket: UdpSocket,
    tcp_port: u16,
    me: Identity,
    favorites: Favorites,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 256];
        while !stop.load(Ordering::Relaxed) {
            stats::discovery_poll();
            match socket.recv_from(&mut buf) {
                Ok((n, from)) => {
                    let Some(asker) = parse_request(&buf[..n]) else { continue };
                    let mutual = u8::from(favorites.contains(asker));
                    let reply = format!("{ANNOUNCE_PREFIX} {tcp_port} {} {mutual} {} {}", me.id, me.kind.code(), me.name);
                    let _ = socket.send_to(reply.as_bytes(), from);
                    stats::discovery_reply();
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(_) => thread::sleep(Duration::from_millis(200)),
            }
        }
    })
}

/// Broadcasts a discovery request and collects answers until `wait` elapses.
/// `me` is left out of the result, so a device never lists itself.
pub fn discover(me: &Identity, wait: Duration) -> io::Result<Vec<Peer>> {
    let socket = UdpSocket::bind(("0.0.0.0", 0))?;
    socket.set_broadcast(true)?;

    // UDP may drop packets, so ask a couple of times.
    let request = format!("{DISCOVER_PREFIX} {}", me.id);
    for _ in 0..2 {
        socket.send_to(request.as_bytes(), ("255.255.255.255", DISCOVERY_PORT))?;
    }

    let deadline = Instant::now() + wait;
    let mut seen = HashSet::new();
    let mut peers = Vec::new();
    let mut buf = [0u8; 512];

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        socket.set_read_timeout(Some(remaining))?;
        let (n, from) = match socket.recv_from(&mut buf) {
            Ok(r) => r,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
            Err(e) => return Err(e),
        };
        let Some(peer) = parse_announce(&buf[..n], from) else {
            continue;
        };
        // A device with several interfaces answers once per interface.
        if peer.id != me.id && seen.insert(peer.id) {
            peers.push(peer);
        }
    }
    Ok(peers)
}

fn parse_request(msg: &[u8]) -> Option<DeviceId> {
    let text = std::str::from_utf8(msg).ok()?;
    let (prefix, id) = text.split_once(' ')?;
    if prefix != DISCOVER_PREFIX {
        return None;
    }
    id.parse().ok()
}

fn parse_announce(msg: &[u8], from: SocketAddr) -> Option<Peer> {
    let text = std::str::from_utf8(msg).ok()?;
    let mut parts = text.splitn(6, ' ');
    if parts.next()? != ANNOUNCE_PREFIX {
        return None;
    }
    let port: u16 = parts.next()?.parse().ok()?;
    let id: DeviceId = parts.next()?.parse().ok()?;
    let mutual = parts.next()? == "1";
    let kind = DeviceKind::from_code(parts.next()?);
    let name = clean_name(parts.next().unwrap_or(""));
    Some(Peer {
        addr: SocketAddr::new(from.ip(), port),
        id,
        name,
        mutual,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announce_round_trip() {
        let id = DeviceId::random();
        let from: SocketAddr = "192.168.1.5:5555".parse().unwrap();
        let peer = parse_announce(format!("{ANNOUNCE_PREFIX} 47800 {id} 1 d Living room Mac").as_bytes(), from).unwrap();
        assert_eq!(peer.addr, "192.168.1.5:47800".parse().unwrap());
        assert_eq!((peer.id, peer.mutual, peer.name.as_str()), (id, true, "Living room Mac"));
        assert_eq!(peer.kind, DeviceKind::Desktop);
        // Older formats lack the id or the kind and must not parse.
        assert!(parse_announce(b"AIRLYNX_HERE 47800 Old Mac", from).is_none());
        assert!(parse_announce(format!("{ANNOUNCE_PREFIX} 47800 {id} 1").as_bytes(), from).is_none());
    }

    #[test]
    fn request_needs_an_id() {
        let id = DeviceId::random();
        assert_eq!(parse_request(format!("{DISCOVER_PREFIX} {id}").as_bytes()), Some(id));
        assert_eq!(parse_request(DISCOVER_PREFIX.as_bytes()), None);
    }
}
