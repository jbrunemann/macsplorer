//! Sends files to the first device whose alias contains the text given:
//! `cargo run -p localsend-client --example send <alias> <file>...`.

use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

use localsend_client::Me;
use localsend_client::discovery::{Options, Scan};
use localsend_client::send::{Progress, send};

fn main() {
    let mut args = std::env::args().skip(1);
    let wanted = args.next().expect("alias");
    let paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
    let me = Me::new("localsend-client send", None);
    let (tx, rx) = channel();
    let _scan = Scan::start(me.clone(), Options::default(), move |d| {
        let _ = tx.send(d);
    });
    let device = loop {
        let d = rx.recv_timeout(Duration::from_secs(10)).expect("no such device");
        if d.alias().contains(&wanted) {
            break d;
        }
    };
    println!("sending to {} at {}", device.alias(), device.addr());
    let progress = Progress::default();
    std::thread::scope(|s| {
        s.spawn(|| println!("{:?}", send(&me, &device, &paths, None, &progress)));
        for _ in 0..8 {
            std::thread::sleep(Duration::from_secs(1));
            println!("{:?} {:?}", progress.stage(), progress.bytes());
        }
        std::process::exit(0);
    });
}
