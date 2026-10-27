//! Receives into a folder, accepting whatever comes, until stopped:
//! `cargo run -p localsend --example receive [folder]` (the current folder
//! without one). Prints what happens.

use std::path::PathBuf;
use std::sync::mpsc::channel;

use localsend::Me;
use localsend::receive::{Event, Options, Receiver};

fn main() {
    let folder = std::env::args().nth(1).map_or_else(|| PathBuf::from("."), PathBuf::from);
    let (tx, rx) = channel();
    let receiver = Receiver::start(&Me::new("localsend receive", None), Options::new(&folder), move |e| {
        let _ = tx.send(e);
    })
    .expect("couldn't start the receiver");
    let me = receiver.me();
    println!("receiving as \"{}\" on port {} ({:?}) into {}", me.alias, me.port, me.protocol, folder.display());
    for event in rx {
        match event {
            Event::Device(d) => println!("seen {} at {}", d.alias(), d.addr()),
            Event::Offer(o) => {
                println!("{} at {} offers {} file(s), {} bytes; accepting", o.from.alias, o.ip, o.files.len(), o.total_size());
                o.accept();
            }
            Event::Done(r) => println!("{:?}", r),
        }
    }
}
