//! Lists the LocalSend devices that answer within a few seconds:
//! `cargo run -p localsend --example scan [seconds [known address...]]`; the
//! known addresses are swept first.

use std::time::Duration;

use localsend::Me;
use localsend::discovery::{Options, Scan, Sweep, local_ipv4};

fn main() {
    let seconds = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let first = std::env::args().skip(2).filter_map(|a| a.parse().ok()).collect();
    println!("interfaces: {:?}", local_ipv4());
    let scan = Scan::start(Me::new("localsend scan", None), Options { announce: true, sweep: match std::env::var("SWEEP").as_deref() { Ok("off") => Sweep::Off, Ok("near") => Sweep::Near, _ => Sweep::All }, first }, |d| {
        println!("{:<24} {:?} {} ({})", d.alias(), d.device_type(), d.addr(), d.protocol.scheme());
    });
    std::thread::sleep(Duration::from_secs(seconds));
    println!("sweep still running: {}, at {:?}", scan.sweeping(), scan.progress());
}
