# localsend-client

The sending side of the [LocalSend protocol v2](https://github.com/localsend/protocol)
in plain Rust, with no UI and no async runtime: find the LocalSend devices on
the local network and send them files and folders.

```rust
use localsend_client::{Me, discovery::{Options, Scan}, send::{Progress, send}};

let me = Me::new("My Mac", Some("Macsplorer".into()));
let (tx, rx) = std::sync::mpsc::channel();
let scan = Scan::start(me.clone(), Options::default(), move |device| { let _ = tx.send(device); });
let device = rx.recv().unwrap();
let progress = Progress::default(); // share it to show progress or cancel
let sent = send(&me, &device, &["photo.jpg".into()], None, &progress)?;
```

- `discovery`: multicast announcements on 224.0.0.167:53317 plus a `GET /info`
  sweep of the private /24 networks the computer is on. A `Scan` listens
  until it is dropped.
- `send`: `prepare-upload` (which waits until the receiver accepts), then one
  `upload` per accepted file; folders go as their files with `/`-separated
  paths. `Progress` reports bytes and the stage and has the Cancel.
- `protocol`: the JSON messages. `devices`: a de-duplicated, sorted list.

Not here (yet): receiving, PINs (a device that asks for one gives
`SendError::PinRequired`), certificate pinning by fingerprint.

The `tls` feature (on by default) brings rustls with ring for the HTTPS that
LocalSend uses by default; ring needs a C compiler for the target. Receivers
on protocol 2.2 also want a certificate from the sender (mutual TLS) and drop
the connection without one, so `Me::new` makes a self-signed one with rcgen
and uses its SHA-256 as the fingerprint.

Try it: `cargo run -p localsend-client --example scan` lists what answers, and
`--example send <alias> <file>...` sends.

It lives in the Macsplorer repository for now and depends on nothing in it.
