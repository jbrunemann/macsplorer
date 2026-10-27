//! Finding LocalSend devices on the local network, two ways at once:
//!
//! - **Multicast.** The scan listens on the multicast group
//!   ([`MULTICAST`]`:`[`PORT`]), where devices announce themselves when
//!   they start and answer announcements. With [`Options::announce`] it
//!   announces itself a few times, which makes every device answer, either
//!   with a `POST /register` to us (which fails: nothing is served here)
//!   or, as their fallback, with a multicast answer that the scan hears.
//!   The announcement also puts this side on the other devices' lists, as
//!   any LocalSend app is, although nothing can be sent to it yet.
//! - **Sweep.** For networks where multicast doesn't get through, every
//!   address of the private IPv4 networks we are on (their /24 at most)
//!   gets a `GET /info` on [`PORT`]. That route only describes the device,
//!   unlike the spec's legacy `POST /register` sweep, so the sweep doesn't
//!   add this side to anyone's list.
//!
//! The listener binds the group's own address, shared (`SO_REUSEADDR`,
//! `SO_REUSEPORT`), which works next to a LocalSend app on the same
//! computer even though that one binds the port unshared on every address
//! (checked with LocalSend 1.17 on macOS). Where that fails it tries the
//! port on every address; if that is taken too, only the sweep finds
//! devices.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use socket2::{Domain, Protocol as SockProtocol, Socket, Type};

use crate::protocol::{API, Announcement, DeviceInfo, MULTICAST, PORT, Protocol};
use crate::{Device, Me};

/// When the announcement goes out, in milliseconds from the start of the
/// scan. Datagrams get lost, so it is said three times.
const ANNOUNCE_AT_MS: [u64; 3] = [0, 800, 2500];
/// How long the sweep waits for a port to answer before it moves on.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
/// How long the sweep's `/info` request may take on an open port.
const INFO_TIMEOUT: Duration = Duration::from_secs(2);
/// Addresses the sweep tries at once.
const SWEEP_THREADS: usize = 64;
/// How often the listener looks whether the scan was stopped.
const POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Announce this side on the multicast group to get answers.
    pub announce: bool,
    /// Ask every address nearby with `GET /info`.
    pub sweep: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { announce: true, sweep: true }
    }
}

/// A scan going on. It listens until it is dropped; the sweep and the
/// announcements are done once, at the start.
pub struct Scan {
    stop: Arc<AtomicBool>,
    sweeping: Arc<AtomicUsize>,
}

impl Scan {
    /// Starts scanning on worker threads. `found` is called on one of them
    /// for each device that answers, as often as it answers; our own
    /// announcements (by `me`'s fingerprint) are left out.
    pub fn start(me: Me, options: Options, found: impl Fn(Device) + Send + Sync + 'static) -> Scan {
        let stop = Arc::new(AtomicBool::new(false));
        let sweeping = Arc::new(AtomicUsize::new(0));
        let found: Arc<dyn Fn(Device) + Send + Sync> = Arc::new(found);
        let interfaces = local_ipv4();
        let mine = me.fingerprint.clone();
        let report = Arc::new(move |d: Device| {
            if d.info.fingerprint != mine {
                found(d)
            }
        });

        // Without the socket (the port taken unshared) the sweep has to do.
        if let Ok(socket) = listener(&interfaces) {
            let (stop, report) = (stop.clone(), report.clone());
            thread::spawn(move || listen(&socket, &stop, &*report));
        }
        if options.announce {
            let (stop, interfaces) = (stop.clone(), interfaces.clone());
            let message = serde_json::to_vec(&Announcement { info: me.info(), announce: true }).unwrap_or_default();
            thread::spawn(move || announce(&message, &interfaces, &stop));
        }
        if options.sweep {
            let hosts: Arc<Vec<Ipv4Addr>> = Arc::new(sweep_hosts(&interfaces));
            let next = Arc::new(AtomicUsize::new(0));
            let agent = crate::send::agent(&me, Some(INFO_TIMEOUT));
            sweeping.store(SWEEP_THREADS, Ordering::Relaxed);
            for _ in 0..SWEEP_THREADS {
                let (stop, hosts, next, agent, report, sweeping) =
                    (stop.clone(), hosts.clone(), next.clone(), agent.clone(), report.clone(), sweeping.clone());
                thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let Some(&ip) = hosts.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
                        if let Some(device) = probe(&agent, ip) {
                            report(device);
                        }
                    }
                    sweeping.fetch_sub(1, Ordering::Relaxed);
                });
            }
        }
        Scan { stop, sweeping }
    }

    /// Whether the sweep is still going through the addresses.
    pub fn sweeping(&self) -> bool {
        self.sweeping.load(Ordering::Relaxed) > 0
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A network interface's IPv4 address and netmask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interface {
    pub ip: Ipv4Addr,
    pub mask: Ipv4Addr,
}

/// The IPv4 interfaces that are up, without loopback and point-to-point
/// links (VPN tunnels).
pub fn local_ipv4() -> Vec<Interface> {
    let mut out = Vec::new();
    // SAFETY: getifaddrs hands out a list that stays valid until
    // freeifaddrs; every pointer is checked before it is read.
    unsafe {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut at = list;
        while !at.is_null() {
            let ifa = &*at;
            at = ifa.ifa_next;
            let flags = ifa.ifa_flags as libc::c_int;
            let up = flags & libc::IFF_UP != 0 && flags & libc::IFF_LOOPBACK == 0 && flags & libc::IFF_POINTOPOINT == 0;
            if !up || ifa.ifa_addr.is_null() || ifa.ifa_netmask.is_null() || (*ifa.ifa_addr).sa_family as libc::c_int != libc::AF_INET {
                continue;
            }
            let v4 = |sa: *const libc::sockaddr| Ipv4Addr::from(u32::from_be((*(sa as *const libc::sockaddr_in)).sin_addr.s_addr));
            let iface = Interface { ip: v4(ifa.ifa_addr), mask: v4(ifa.ifa_netmask) };
            if !out.contains(&iface) {
                out.push(iface);
            }
        }
        libc::freeifaddrs(list);
    }
    out
}

/// The addresses to sweep: the hosts of each private network we are on,
/// limited to the /24 around our address on bigger ones (as LocalSend's own
/// sweep does). Our own addresses stay in, for a LocalSend app on this
/// computer; this side serves nothing there.
pub fn sweep_hosts(interfaces: &[Interface]) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    for i in interfaces.iter().filter(|i| i.ip.is_private()) {
        let mask = u32::from(i.mask).max(0xFFFF_FF00);
        let net = u32::from(i.ip) & mask;
        let broadcast = net | !mask;
        for host in net + 1..broadcast {
            let ip = Ipv4Addr::from(host);
            if !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

/// The multicast socket: the shared port, the group joined on each
/// interface.
pub(crate) fn listener(interfaces: &[Interface]) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(SockProtocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    let group = SocketAddr::V4(SocketAddrV4::new(MULTICAST, PORT));
    if socket.bind(&group.into()).is_err() {
        socket.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT)).into())?;
    }
    let mut joined = false;
    for i in interfaces {
        joined |= socket.join_multicast_v4(&MULTICAST, &i.ip).is_ok();
    }
    if !joined {
        socket.join_multicast_v4(&MULTICAST, &Ipv4Addr::UNSPECIFIED)?;
    }
    socket.set_read_timeout(Some(POLL))?;
    Ok(socket.into())
}

fn listen(socket: &UdpSocket, stop: &AtomicBool, report: &dyn Fn(Device)) {
    let mut buf = [0u8; 8192];
    while !stop.load(Ordering::Relaxed) {
        let Ok((n, from)) = socket.recv_from(&mut buf) else { continue };
        if let Ok(a) = serde_json::from_slice::<Announcement>(&buf[..n]) {
            report(Device::new(a.info, from.ip(), PORT, Protocol::Https));
        }
    }
}

/// Sends the announcement out of every interface, at [`ANNOUNCE_AT_MS`].
pub(crate) fn announce(message: &[u8], interfaces: &[Interface], stop: &AtomicBool) {
    let sockets: Vec<UdpSocket> = interfaces.iter().filter_map(|i| sender(i.ip).ok()).collect();
    let group = SocketAddr::V4(SocketAddrV4::new(MULTICAST, PORT));
    let mut waited = 0;
    for at in ANNOUNCE_AT_MS {
        thread::sleep(Duration::from_millis(at - waited));
        waited = at;
        if stop.load(Ordering::Relaxed) {
            return;
        }
        for s in &sockets {
            let _ = s.send_to(message, group);
        }
    }
}

pub(crate) fn sender(ip: Ipv4Addr) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(SockProtocol::UDP))?;
    socket.set_multicast_if_v4(&ip)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.set_multicast_loop_v4(true)?;
    socket.bind(&SocketAddr::V4(SocketAddrV4::new(ip, 0)).into())?;
    Ok(socket.into())
}

/// Asks one address for its `/info`, over HTTPS (the default) and then
/// HTTP, if the port takes connections at all.
fn probe(agent: &ureq::Agent, ip: Ipv4Addr) -> Option<Device> {
    let addr = SocketAddr::new(IpAddr::V4(ip), PORT);
    TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    [Protocol::Https, Protocol::Http].into_iter().find_map(|protocol| {
        let url = format!("{}://{addr}{API}/info", protocol.scheme());
        let mut response = agent.get(&url).call().ok()?;
        if !response.status().is_success() {
            return None;
        }
        let info: DeviceInfo = response.body_mut().read_json().ok()?;
        Some(Device::new(info, IpAddr::V4(ip), PORT, protocol))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sweep_covers_the_private_slash_24s_but_not_us() {
        let ifaces = [
            Interface { ip: Ipv4Addr::new(192, 168, 1, 10), mask: Ipv4Addr::new(255, 255, 255, 0) },
            // A /16 is cut down to the /24 around us.
            Interface { ip: Ipv4Addr::new(10, 0, 7, 5), mask: Ipv4Addr::new(255, 255, 0, 0) },
            // A public address isn't swept.
            Interface { ip: Ipv4Addr::new(8, 8, 8, 8), mask: Ipv4Addr::new(255, 255, 255, 0) },
        ];
        let hosts = sweep_hosts(&ifaces);
        assert_eq!(hosts.len(), 254 + 254);
        assert!(hosts.contains(&Ipv4Addr::new(192, 168, 1, 1)));
        assert!(hosts.contains(&Ipv4Addr::new(192, 168, 1, 254)));
        assert!(hosts.contains(&Ipv4Addr::new(192, 168, 1, 10)));
        assert!(!hosts.contains(&Ipv4Addr::new(192, 168, 1, 255)));
        assert!(hosts.contains(&Ipv4Addr::new(10, 0, 7, 1)));
        assert!(!hosts.contains(&Ipv4Addr::new(10, 0, 8, 1)));
        assert!(!hosts.iter().any(|h| h.octets()[0] == 8));
    }

    #[test]
    fn a_small_network_is_swept_whole() {
        let ifaces = [Interface { ip: Ipv4Addr::new(172, 16, 0, 2), mask: Ipv4Addr::new(255, 255, 255, 252) }];
        assert_eq!(sweep_hosts(&ifaces), [Ipv4Addr::new(172, 16, 0, 1), Ipv4Addr::new(172, 16, 0, 2)]);
    }
}
