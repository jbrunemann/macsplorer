# localsend

The [LocalSend protocol v2](https://github.com/localsend/protocol) in plain
Rust, with no UI and no async runtime: find the LocalSend devices on the local
network, send them files and folders, and receive theirs.

```rust
use localsend::{Me, discovery::{Options, Scan}, send::{Progress, send}};

let me = Me::new("My Mac", Some("Macsplorer".into()));
let (tx, rx) = std::sync::mpsc::channel();
let scan = Scan::start(me.clone(), Options::default(), move |device| { let _ = tx.send(device); });
let device = rx.recv().unwrap();
let progress = Progress::default(); // share it to show progress or cancel
let sent = send(&me, &device, &["photo.jpg".into()], None, &progress)?;
```

Receiving:

```rust
use localsend::receive::{Event, Options, Receiver};

let receiver = Receiver::start(&me, Options::new("/Users/me/Downloads"), |event| match event {
    Event::Offer(offer) => { offer.accept(); } // or ask someone first; dropping it declines
    Event::Device(device) => {}                // a device announced itself
    Event::Done(received) => println!("{:?}", received.saved),
})?;
// It serves until `receiver` is dropped.
```

- `discovery`: multicast announcements on 224.0.0.167:53317 plus a `GET /info`
  sweep of the private /24 networks the computer is on. A `Scan` listens
  until it is dropped.
- `send`: `prepare-upload` (which waits until the receiver accepts), then one
  `upload` per accepted file; folders go as their files with `/`-separated
  paths, with their modified times. `Progress` reports bytes and the stage and
  has the Cancel.
- `receive`: the HTTP(S) server for `info`, `register`, `prepare-upload`,
  `upload` and `cancel`, and this side's presence on the multicast group (it
  announces itself and answers others). Every offer goes to the callback,
  which accepts or declines it. Received names are cleaned (no `..`, no
  absolute paths, no control characters), never replace an existing file
  ("photo (1).jpg") and never merge into an existing folder. Modified times
  are kept; on macOS files can be marked as downloaded (`com.apple.quarantine`)
  for Gatekeeper. One transfer at a time; others get 409.
- `protocol`: the JSON messages. `devices`: a de-duplicated, sorted list.

Not here (yet): PINs (a device that asks for one gives
`SendError::PinRequired`; the receiver never asks), certificate pinning by
fingerprint, text messages (they arrive as a small text file), the
download API (`prepare-download`).

The `tls` feature (on by default) brings rustls with ring for the HTTPS that
LocalSend uses by default; ring needs a C compiler for the target. Receivers
on protocol 2.2 also want a certificate from the sender (mutual TLS) and drop
the connection without one, so `Me::new` makes a self-signed one with rcgen
and uses its SHA-256 as the fingerprint; the receiver serves HTTPS with the
same certificate. `Identity::load_or_generate` keeps it in a file, so other
devices know this one again after a restart. Without the feature everything
is plain HTTP.

Try it: `cargo run -p localsend --example scan` lists what answers,
`--example send <alias> <file>...` sends, and `--example receive [folder]`
receives (accepting everything) until stopped.

It lives in the Macsplorer repository for now and depends on nothing in it.
