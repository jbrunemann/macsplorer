//! Lists the LocalSend devices that answer within a few seconds:
//! `cargo run -p localsend-client --example scan [seconds]`.

use std::time::Duration;

use localsend_client::Me;
use localsend_client::discovery::{Options, Scan, local_ipv4};

fn main() {
    let seconds = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    println!("interfaces: {:?}", local_ipv4());
    let scan = Scan::start(Me::new("localsend-client scan", None), Options { announce: true, sweep: std::env::var_os("NO_SWEEP").is_none() }, |d| {
        println!("{:<24} {:?} {} ({})", d.alias(), d.device_type(), d.addr(), d.protocol.scheme());
    });
    std::thread::sleep(Duration::from_secs(seconds));
    println!("sweep still running: {}", scan.sweeping());
}
