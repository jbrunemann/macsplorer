//! Just enough HTTP/1.1 to serve LocalSend's routes ([`crate::receive`]):
//! a request's head, its body by `Content-Length` or chunked, and answers
//! with a body of known length. Connections are kept open between requests
//! unless either side says `close`.

use std::io::{self, BufRead, Read, Write};

/// The most a request's head may take, line and headers together.
const MAX_HEAD: usize = 64 * 1024;

#[derive(Debug)]
pub struct Request {
    pub method: String,
    /// Without the query.
    pub path: String,
    query: Vec<(String, String)>,
    /// Names in lowercase.
    headers: Vec<(String, String)>,
}

impl Request {
    pub fn query(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Whether the client wants the connection kept for another request.
    pub fn keep_alive(&self) -> bool {
        !self.header("connection").is_some_and(|c| c.eq_ignore_ascii_case("close"))
    }

    /// How the body comes.
    pub fn body(&self) -> io::Result<Body> {
        if self.header("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
            return Ok(Body::Chunked);
        }
        match self.header("content-length") {
            Some(len) => len.trim().parse().map(Body::Length).map_err(|_| bad("bad Content-Length")),
            None => Ok(Body::Length(0)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    Length(u64),
    Chunked,
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// Reads the next request's head; `None` when the client closed the
/// connection between requests.
pub fn read_head(r: &mut impl BufRead) -> io::Result<Option<Request>> {
    let mut taken = 0;
    let mut line = String::new();
    // Blank lines before a request are allowed (RFC 9112 2.2).
    loop {
        line.clear();
        if read_line(r, &mut line, &mut taken)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target), Some(_version)) = (parts.next(), parts.next(), parts.next()) else { return Err(bad("bad request line")) };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let request_path = decode(path);
    let query = query.split('&').filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (decode(k), decode(v))
    });
    let mut request = Request { method: method.to_owned(), path: request_path, query: query.collect(), headers: Vec::new() };
    loop {
        line.clear();
        if read_line(r, &mut line, &mut taken)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            return Ok(Some(request));
        }
        let (name, value) = header.split_once(':').ok_or_else(|| bad("bad header"))?;
        request.headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
}

fn read_line(r: &mut impl BufRead, line: &mut String, taken: &mut usize) -> io::Result<usize> {
    let n = r.take((MAX_HEAD - *taken) as u64).read_line(line)?;
    *taken += n;
    if *taken >= MAX_HEAD {
        return Err(bad("request head too long"));
    }
    Ok(n)
}

/// `%XX` escapes decoded; invalid ones and invalid UTF-8 are kept as they are.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).and_then(|c| hex(*c)), b.get(i + 2).and_then(|c| hex(*c)))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

/// A request's body, which ends where the request does.
pub struct BodyReader<'a, R> {
    inner: &'a mut R,
    kind: Body,
    /// Left in the body (`Length`) or in this chunk (`Chunked`).
    left: u64,
    done: bool,
}

impl<'a, R: BufRead> BodyReader<'a, R> {
    pub fn new(inner: &'a mut R, kind: Body) -> Self {
        let left = match kind {
            Body::Length(n) => n,
            Body::Chunked => 0,
        };
        Self { inner, kind, left, done: kind == Body::Length(0) }
    }

    /// Whether all of it was read, so the next request can follow.
    pub fn finished(&self) -> bool {
        self.done
    }

    fn next_chunk(&mut self) -> io::Result<()> {
        let mut line = String::new();
        self.inner.by_ref().take(1024).read_line(&mut line)?;
        let size = line.trim().split(';').next().unwrap_or_default();
        self.left = u64::from_str_radix(size.trim(), 16).map_err(|_| bad("bad chunk size"))?;
        if self.left == 0 {
            // Trailers, up to the blank line.
            loop {
                line.clear();
                if self.inner.by_ref().take(8192).read_line(&mut line)? == 0 || line.trim().is_empty() {
                    break;
                }
            }
            self.done = true;
        }
        Ok(())
    }
}

impl<R: BufRead> Read for BodyReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.kind == Body::Chunked && self.left == 0 {
            self.next_chunk()?;
            if self.done {
                return Ok(0);
            }
        }
        let want = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.left -= n as u64;
        if self.left == 0 {
            match self.kind {
                Body::Length(_) => self.done = true,
                Body::Chunked => {
                    let mut crlf = [0u8; 2];
                    self.inner.read_exact(&mut crlf)?;
                }
            }
        }
        Ok(n)
    }
}

/// Writes an answer. `body` is JSON when there is one.
pub fn respond(w: &mut impl Write, status: u16, body: &[u8], keep_alive: bool) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    };
    let connection = if keep_alive { "keep-alive" } else { "close" };
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: {connection}\r\n", body.len());
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    w.write_all(body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn a_request_head_reads() {
        let raw = b"POST /api/localsend/v2/upload?sessionId=a%20b&fileId=1&token=t HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
        let mut r = BufReader::new(&raw[..]);
        let req = read_head(&mut r).unwrap().unwrap();
        assert_eq!((req.method.as_str(), req.path.as_str()), ("POST", "/api/localsend/v2/upload"));
        assert_eq!(req.query("sessionId"), Some("a b"));
        assert_eq!(req.query("token"), Some("t"));
        assert_eq!(req.query("pin"), None);
        assert!(!req.keep_alive());
        assert_eq!(req.body().unwrap(), Body::Length(5));
        let mut body = Vec::new();
        BodyReader::new(&mut r, req.body().unwrap()).read_to_end(&mut body).unwrap();
        assert_eq!(body, b"hello");
        assert!(read_head(&mut r).unwrap().is_none());
    }

    #[test]
    fn chunked_bodies_read_and_the_next_request_follows() {
        let raw = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n6;x=y\r\npedia!\r\n0\r\nTrailer: 1\r\n\r\nGET /b HTTP/1.1\r\n\r\n";
        let mut r = BufReader::new(&raw[..]);
        let req = read_head(&mut r).unwrap().unwrap();
        let mut body = BodyReader::new(&mut r, req.body().unwrap());
        let mut out = Vec::new();
        body.read_to_end(&mut out).unwrap();
        assert!(body.finished());
        assert_eq!(out, b"Wikipedia!");
        assert_eq!(read_head(&mut r).unwrap().unwrap().path, "/b");
    }

    #[test]
    fn a_short_body_is_an_error() {
        let raw = b"abc";
        let mut r = BufReader::new(&raw[..]);
        let mut out = Vec::new();
        assert!(BodyReader::new(&mut r, Body::Length(5)).read_to_end(&mut out).is_err());
    }

    #[test]
    fn garbage_is_not_a_request() {
        let mut r = BufReader::new(&b"hello\r\n\r\n"[..]);
        assert!(read_head(&mut r).is_err());
        let mut r = BufReader::new(&b"GET / HTTP/1.1\r\nno colon\r\n\r\n"[..]);
        assert!(read_head(&mut r).is_err());
    }

    #[test]
    fn answers_carry_their_length() {
        let mut out = Vec::new();
        respond(&mut out, 403, b"", false).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
}
