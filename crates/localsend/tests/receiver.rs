//! The receiver on localhost, sent to by this crate's own `send` (HTTPS
//! with the `tls` feature, plain HTTP without an identity), and by hand
//! for what `send` never does.

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver as Rx, channel};
use std::thread;
use std::time::{Duration, SystemTime};

use localsend::receive::{Closed, End, Event, Offer, Options, Received, Receiver};
use localsend::send::{Progress, SendError, Sent, send};
use localsend::{Device, Me};

const WAIT: Duration = Duration::from_secs(10);

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("localsend-recv-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A receiver saving into a new folder, and its events.
fn receiver(name: &str, me: &Me) -> (Receiver, Rx<Event>, PathBuf) {
    let folder = temp(&format!("{name}-in"));
    let options = Options { port: 0, presence: false, ask_timeout: Duration::from_secs(5), quarantine: Some("LocalSendTest".into()), ..Options::new(&folder) };
    let (tx, rx) = channel();
    let r = Receiver::start(me, options, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    (r, rx, folder)
}

fn device(r: &Receiver) -> Device {
    let me = r.me();
    Device::new(me.info(), IpAddr::V4(Ipv4Addr::LOCALHOST), me.port, me.protocol)
}

fn offer(rx: &Rx<Event>) -> Offer {
    loop {
        if let Event::Offer(o) = rx.recv_timeout(WAIT).expect("an offer") {
            return o;
        }
    }
}

fn done(rx: &Rx<Event>) -> Received {
    loop {
        if let Event::Done(d) = rx.recv_timeout(WAIT).expect("the end") {
            return d;
        }
    }
}

/// `a.txt` (5 bytes, modified in 2001) and `Folder/b.png` (4 bytes).
fn files(name: &str) -> Vec<PathBuf> {
    let dir = temp(&format!("{name}-out"));
    fs::create_dir_all(dir.join("Folder")).unwrap();
    fs::write(dir.join("a.txt"), "hello").unwrap();
    fs::write(dir.join("Folder/b.png"), "png!").unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options().write(true).open(dir.join("a.txt")).unwrap().set_modified(old).unwrap();
    vec![dir.join("a.txt"), dir.join("Folder")]
}

fn send_in_background(device: Device, paths: Vec<PathBuf>) -> thread::JoinHandle<Result<Sent, SendError>> {
    thread::spawn(move || send(&Me::new("Phone", None), &device, &paths, None, &Progress::default()))
}

fn transfer(name: &str, me: Me) {
    let (r, rx, folder) = receiver(name, &me);
    let sender = send_in_background(device(&r), files(name));
    let offer = offer(&rx);
    assert_eq!(offer.from.alias, "Phone");
    let names: Vec<_> = offer.files.iter().map(|f| f.file_name.as_str()).collect();
    assert_eq!(names, ["Folder/b.png", "a.txt"]);
    assert_eq!(offer.total_size(), 9);
    let progress = offer.accept();
    let got = done(&rx);
    assert_eq!(got.end, End::Finished);
    assert_eq!(got.missing, 0);
    assert_eq!(got.saved, [folder.join("Folder/b.png"), folder.join("a.txt")]);
    assert_eq!(progress.bytes(), (9, 9));
    assert_eq!(fs::read(folder.join("a.txt")).unwrap(), b"hello");
    assert_eq!(fs::read(folder.join("Folder/b.png")).unwrap(), b"png!");
    let modified = fs::metadata(folder.join("a.txt")).unwrap().modified().unwrap();
    assert_eq!(modified, SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000));
    let sent = sender.join().unwrap().unwrap();
    assert_eq!((sent.files, sent.declined), (2, 0));
}

#[cfg(feature = "tls")]
#[test]
fn files_arrive_over_https() {
    let me = Me::new("Mac", None);
    assert_eq!(me.protocol, localsend::Protocol::Https);
    transfer("https", me);
}

#[test]
fn files_arrive_over_http() {
    transfer("http", Me::with_identity("Mac", None, None));
}

#[test]
fn names_taken_get_a_number_and_files_are_quarantined() {
    let me = Me::with_identity("Mac", None, None);
    let (r, rx, folder) = receiver("taken", &me);
    fs::write(folder.join("a.txt"), "old").unwrap();
    fs::create_dir(folder.join("Folder")).unwrap();
    let sender = send_in_background(device(&r), files("taken"));
    offer(&rx).accept();
    let got = done(&rx);
    assert_eq!(got.saved, [folder.join("Folder (1)/b.png"), folder.join("a (1).txt")]);
    assert_eq!(fs::read(folder.join("a.txt")).unwrap(), b"old");
    sender.join().unwrap().unwrap();
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("xattr").arg("-p").arg("com.apple.quarantine").arg(folder.join("a (1).txt")).output().unwrap();
        let value = String::from_utf8_lossy(&out.stdout);
        assert!(value.starts_with("0081;") && value.trim_end().ends_with(";LocalSendTest;"), "{value:?}");
    }
}

#[test]
fn a_declined_offer_is_refused_and_a_second_one_waits_its_turn() {
    let me = Me::with_identity("Mac", None, None);
    let (r, rx, folder) = receiver("decline", &me);
    let first = send_in_background(device(&r), files("decline1"));
    let offer = offer(&rx);
    // While the first is asked, another device is told the receiver is busy.
    let second = send_in_background(device(&r), files("decline2")).join().unwrap();
    assert!(matches!(second, Err(SendError::Busy)), "{second:?}");
    offer.decline();
    assert!(matches!(first.join().unwrap(), Err(SendError::Rejected)));
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn a_sender_that_gives_up_withdraws_the_offer() {
    let me = Me::with_identity("Mac", None, None);
    let (r, rx, _) = receiver("withdraw", &me);
    let d = device(&r);
    let progress = std::sync::Arc::new(Progress::default());
    let p = progress.clone();
    let paths = files("withdraw");
    let agent = thread::spawn(move || {
        // A request that hangs up while it waits, as a sender that is
        // closed does.
        let agent = localsend::send::agent(&Me::with_identity("Phone", None, None), Some(Duration::from_millis(500)));
        let body = r#"{"info":{"alias":"Phone","fingerprint":"p"},"files":{"1":{"id":"1","fileName":"x","size":1,"fileType":"text/plain"}}}"#;
        let _ = agent.post(format!("{}/prepare-upload", d.base_url())).header("Content-Type", "application/json").send(body);
        drop((p, paths));
    });
    let offer = offer(&rx);
    agent.join().unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    while offer.closed().is_none() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(offer.closed(), Some(Closed::Withdrawn));
    drop(progress);
}

#[test]
fn unsafe_names_are_not_given_a_token() {
    let me = Me::with_identity("Mac", None, None);
    let (r, rx, folder) = receiver("unsafe", &me);
    let d = device(&r);
    let post = thread::spawn(move || {
        let agent = localsend::send::agent(&Me::with_identity("Phone", None, None), Some(WAIT));
        let body = r#"{"info":{"alias":"Phone","fingerprint":"p"},"files":{
            "1":{"id":"1","fileName":"../../evil.txt","size":1,"fileType":"text/plain"},
            "2":{"id":"2","fileName":"/tmp/ok.txt","size":2,"fileType":"text/plain"},
            "3":{"id":"3","fileName":"small.txt","size":3,"fileType":"text/plain"}}}"#;
        let mut response = agent.post(format!("{}/prepare-upload", d.base_url())).header("Content-Type", "application/json").send(body).unwrap();
        let answer: serde_json::Value = response.body_mut().read_json().unwrap();
        let token = answer["files"]["2"].as_str().unwrap().to_owned();
        assert!(answer["files"].get("1").is_none(), "{answer}");
        let session = answer["sessionId"].as_str().unwrap().to_owned();
        // A wrong token is refused, the right one takes the file.
        let url = |t: &str| format!("{}/upload?sessionId={session}&fileId=2&token={t}", d.base_url());
        assert_eq!(agent.post(url("nope")).send("xx").unwrap().status().as_u16(), 403);
        assert_eq!(agent.post(url(&token)).send("ok").unwrap().status().as_u16(), 200);
        // A second file brings more than it said it would.
        let token = answer["files"]["3"].as_str().unwrap().to_owned();
        let url = format!("{}/upload?sessionId={session}&fileId=3&token={token}", d.base_url());
        assert_eq!(agent.post(url).send("far too long").unwrap().status().as_u16(), 500);
    });
    offer(&rx).accept();
    post.join().unwrap();
    let got = done(&rx);
    assert_eq!(got.saved, [folder.join("tmp/ok.txt")]);
    assert_eq!(got.missing, 1);
    assert!(!folder.join("small.txt").exists());
    assert!(!folder.parent().unwrap().join("evil.txt").exists());
}

#[test]
fn info_and_register_describe_the_receiver() {
    let me = Me::with_identity("Mac", Some("Macsplorer".into()), None);
    let (r, rx, _) = receiver("info", &me);
    let d = device(&r);
    let agent = localsend::send::agent(&me, Some(WAIT));
    let info: serde_json::Value = agent.get(format!("{}/info", d.base_url())).call().unwrap().body_mut().read_json().unwrap();
    assert_eq!(info["alias"], "Mac");
    assert_eq!(info["port"], r.me().port);
    assert_eq!(info["protocol"], "http");
    let body = r#"{"alias":"Tablet","version":"2.1","deviceType":"mobile","fingerprint":"tab","port":53317,"protocol":"https"}"#;
    let answer: serde_json::Value = agent.post(format!("{}/register", d.base_url())).header("Content-Type", "application/json").send(body).unwrap().body_mut().read_json().unwrap();
    assert_eq!(answer["alias"], "Mac");
    match rx.recv_timeout(WAIT).unwrap() {
        Event::Device(d) => assert_eq!((d.alias(), d.port), ("Tablet", 53317)),
        _ => panic!("expected the tablet"),
    }
}

#[test]
fn the_sender_cancelling_ends_the_transfer() {
    let me = Me::with_identity("Mac", None, None);
    let (r, rx, _) = receiver("cancel", &me);
    let d = device(&r);
    let post = thread::spawn(move || {
        let agent = localsend::send::agent(&Me::with_identity("Phone", None, None), Some(WAIT));
        let body = r#"{"info":{"alias":"Phone","fingerprint":"p"},"files":{"1":{"id":"1","fileName":"x.txt","size":1,"fileType":"text/plain"}}}"#;
        let mut response = agent.post(format!("{}/prepare-upload", d.base_url())).header("Content-Type", "application/json").send(body).unwrap();
        let answer: serde_json::Value = response.body_mut().read_json().unwrap();
        let session = answer["sessionId"].as_str().unwrap().to_owned();
        agent.post(format!("{}/cancel?sessionId={session}", d.base_url())).send_empty().unwrap();
    });
    offer(&rx).accept();
    post.join().unwrap();
    let got = done(&rx);
    assert_eq!((got.end, got.missing), (End::CancelledBySender, 1));
}
