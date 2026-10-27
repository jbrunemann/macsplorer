//! `send` against a stand-in receiver on localhost, over plain HTTP: the
//! requests it makes and how it reads the answers.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use localsend::send::{Progress, SendError, send};
use localsend::{Device, DeviceInfo, Me, Protocol};

/// A request as the receiver saw it.
struct Seen {
    path: String,
    body: Vec<u8>,
}

/// Serves `answers` to the requests in order, recording them.
fn receiver(answers: Vec<(u16, &'static str)>) -> (u16, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    thread::spawn(move || {
        for (status, body) in answers {
            let Ok((stream, _)) = listener.accept() else { return };
            let request = read_request(&stream);
            log.lock().unwrap().push(request);
            let mut stream = stream;
            let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}", body.len());
        }
    });
    (port, seen)
}

fn read_request(stream: &TcpStream) -> Seen {
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let path = line.split_whitespace().nth(1).unwrap_or_default().to_owned();
    let mut len = 0;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        if h.trim().is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body).unwrap();
    Seen { path, body }
}

fn device(port: u16) -> Device {
    let info = DeviceInfo {
        alias: "Phone".into(),
        version: "2.0".into(),
        device_model: None,
        device_type: None,
        fingerprint: "x".into(),
        port: Some(port),
        protocol: Some(Protocol::Http),
        download: None,
    };
    Device::new(info, IpAddr::V4(Ipv4Addr::LOCALHOST), port, Protocol::Http)
}

fn files(name: &str) -> Vec<PathBuf> {
    let dir = std::env::temp_dir().join(format!("localsend-fake-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Folder")).unwrap();
    std::fs::write(dir.join("a.txt"), "hello").unwrap();
    std::fs::write(dir.join("Folder/b.png"), "png!").unwrap();
    vec![dir.join("a.txt"), dir.join("Folder")]
}

#[test]
fn files_are_announced_then_uploaded_with_their_tokens() {
    let (port, seen) = receiver(vec![(200, r#"{"sessionId":"s 1","files":{"0":"t0","1":"t1"}}"#), (200, ""), (200, "")]);
    let progress = Progress::default();
    let sent = send(&Me::new("Mac", None), &device(port), &files("ok"), None, &progress).unwrap();
    assert_eq!(sent.files, 2);
    assert_eq!(progress.bytes(), (9, 9));
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].path, "/api/localsend/v2/prepare-upload");
    let prepare: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(prepare["info"]["alias"], "Mac");
    assert_eq!(prepare["files"]["1"]["fileName"], "Folder/b.png");
    assert_eq!(prepare["files"]["1"]["fileType"], "image/png");
    assert_eq!(seen[1].path, "/api/localsend/v2/upload?sessionId=s%201&fileId=0&token=t0");
    assert_eq!(seen[1].body, b"hello");
    assert_eq!(seen[2].body, b"png!");
}

#[test]
fn only_the_accepted_files_go() {
    let (port, seen) = receiver(vec![(200, r#"{"sessionId":"s","files":{"1":"t1"}}"#), (200, "")]);
    let sent = send(&Me::new("Mac", None), &device(port), &files("some"), None, &Progress::default()).unwrap();
    assert_eq!((sent.files, sent.declined), (1, 1));
    assert_eq!(seen.lock().unwrap()[1].body, b"png!");
}

#[test]
fn a_declined_transfer_says_so() {
    let (port, _) = receiver(vec![(403, "")]);
    let err = send(&Me::new("Mac", None), &device(port), &files("no"), None, &Progress::default()).unwrap_err();
    assert!(matches!(err, SendError::Rejected), "{err:?}");
    let (port, _) = receiver(vec![(401, "")]);
    let err = send(&Me::new("Mac", None), &device(port), &files("pin"), None, &Progress::default()).unwrap_err();
    assert!(matches!(err, SendError::PinRequired), "{err:?}");
}

#[test]
fn cancel_while_waiting_cancels_the_session() {
    let (port, seen) = receiver(vec![(200, r#"{"sessionId":"s","files":{"0":"t0"}}"#), (200, "")]);
    let progress = Progress::default();
    progress.cancel();
    let err = send(&Me::new("Mac", None), &device(port), &files("cancel"), None, &progress).unwrap_err();
    assert!(matches!(err, SendError::Cancelled), "{err:?}");
    // Give the receiver thread a moment to log the cancel.
    thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(seen.lock().unwrap()[1].path, "/api/localsend/v2/cancel?sessionId=s");
}
