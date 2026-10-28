//! Encrypted byte stream for the transfer session.
//!
//! Each side makes a throwaway X25519 key pair and sends its public key during the
//! handshake. Both derive one ChaCha20-Poly1305 key per direction from the shared secret.
//! After that the stream is a series of records: `len: u16 | ciphertext` where the nonce is
//! a per-direction record counter, so records cannot be reordered, replayed or dropped.
//!
//! This hides the traffic from anyone who only records it (packet captures, router logs).
//! It does not authenticate the peer, so someone who can actively sit in the middle of the
//! connection could still read it. See the trust model in the README.

use std::io::{self, Read, Write};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};
use x25519_dalek::{X25519_BASEPOINT_BYTES, x25519};

use crate::protocol::invalid;

/// Largest plaintext per record.
const MAX_PLAIN: usize = 16 * 1024;
const TAG_LEN: usize = 16;

/// One side's half of the key exchange.
pub struct Ephemeral {
    secret: [u8; 32],
    public: [u8; 32],
}

impl Ephemeral {
    pub fn new() -> io::Result<Self> {
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|e| io::Error::other(format!("no random source: {e}")))?;
        Ok(Self { secret, public: x25519(secret, X25519_BASEPOINT_BYTES) })
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    /// Wraps `inner` once the peer's public key is known. `is_sender` must be true on the
    /// side that opened the connection and false on the other.
    pub fn into_stream<S>(self, inner: S, peer_public: [u8; 32], is_sender: bool) -> io::Result<SecureStream<S>> {
        let shared = x25519(self.secret, peer_public);
        if shared == [0u8; 32] {
            return Err(invalid("invalid key exchange"));
        }
        let (sender_pub, receiver_pub) =
            if is_sender { (self.public, peer_public) } else { (peer_public, self.public) };
        let key = |label: &[u8]| {
            let mut h = Sha256::new();
            h.update(b"airlynx v4 ");
            h.update(label);
            h.update(shared);
            h.update(sender_pub);
            h.update(receiver_pub);
            ChaCha20Poly1305::new(Key::from_slice(&h.finalize()))
        };
        let (send, recv) = if is_sender {
            (key(b"sender to receiver"), key(b"receiver to sender"))
        } else {
            (key(b"receiver to sender"), key(b"sender to receiver"))
        };
        Ok(SecureStream {
            inner,
            send,
            recv,
            send_count: 0,
            recv_count: 0,
            out: Vec::new(),
            input: Vec::new(),
            input_pos: 0,
        })
    }
}

/// Reads and writes through the encrypted records. Writes are buffered until a record is
/// full, `flush` is called, or the next read, so a request followed by waiting for the
/// answer needs no explicit flush.
pub struct SecureStream<S> {
    inner: S,
    send: ChaCha20Poly1305,
    recv: ChaCha20Poly1305,
    send_count: u64,
    recv_count: u64,
    out: Vec<u8>,
    input: Vec<u8>,
    input_pos: usize,
}

fn nonce(count: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&count.to_be_bytes());
    *Nonce::from_slice(&n)
}

impl<S: Write> SecureStream<S> {
    /// Encrypts and sends up to `MAX_PLAIN` bytes from the front of `out`.
    fn send_record(&mut self) -> io::Result<()> {
        let take = self.out.len().min(MAX_PLAIN);
        let sealed = self
            .send
            .encrypt(&nonce(self.send_count), &self.out[..take])
            .map_err(|_| io::Error::other("encryption failed"))?;
        self.send_count += 1;
        self.out.drain(..take);
        let mut record = Vec::with_capacity(2 + sealed.len());
        record.extend_from_slice(&(sealed.len() as u16).to_be_bytes());
        record.extend_from_slice(&sealed);
        self.inner.write_all(&record)
    }
}

impl<S: Write> Write for SecureStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.out.extend_from_slice(buf);
        while self.out.len() >= MAX_PLAIN {
            self.send_record()?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        while !self.out.is_empty() {
            self.send_record()?;
        }
        self.inner.flush()
    }
}

impl<S: Read + Write> SecureStream<S> {
    fn read_record(&mut self) -> io::Result<()> {
        let mut len = [0u8; 2];
        self.inner.read_exact(&mut len)?;
        let len = u16::from_be_bytes(len) as usize;
        if !(TAG_LEN..=MAX_PLAIN + TAG_LEN).contains(&len) {
            return Err(invalid("bad record length"));
        }
        let mut sealed = vec![0u8; len];
        self.inner.read_exact(&mut sealed)?;
        self.input = self
            .recv
            .decrypt(&nonce(self.recv_count), sealed.as_slice())
            .map_err(|_| invalid("corrupt or tampered record"))?;
        self.recv_count += 1;
        self.input_pos = 0;
        Ok(())
    }
}

impl<S: Read + Write> Read for SecureStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // Anything we wrote must be out before we wait for the other side.
        self.flush()?;
        if self.input_pos == self.input.len() {
            self.read_record()?;
        }
        let n = buf.len().min(self.input.len() - self.input_pos);
        buf[..n].copy_from_slice(&self.input[self.input_pos..self.input_pos + n]);
        self.input_pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_hides_plaintext() {
        let message: Vec<u8> = b"top secret holiday photo ".iter().copied().cycle().take(100_000).collect();
        let (ea, eb) = (Ephemeral::new().unwrap(), Ephemeral::new().unwrap());
        let (pa, pb) = (ea.public(), eb.public());
        let mut tx = ea.into_stream(Vec::new(), pb, true).unwrap();
        tx.write_all(&message).unwrap();
        tx.flush().unwrap();
        let wire = tx.inner.clone();
        assert!(!wire.windows(10).any(|w| w == b"top secret"));

        let mut rx = eb.into_stream(Cursor::new(wire), pa, false).unwrap();
        let mut got = vec![0u8; message.len()];
        rx.read_exact(&mut got).unwrap();
        assert_eq!(got, message);
    }

    #[test]
    fn tampering_is_detected() {
        let (ea, eb) = (Ephemeral::new().unwrap(), Ephemeral::new().unwrap());
        let (pa, pb) = (ea.public(), eb.public());
        let mut tx = ea.into_stream(Vec::new(), pb, true).unwrap();
        tx.write_all(b"hello").unwrap();
        tx.flush().unwrap();
        let mut wire = tx.inner.clone();
        *wire.last_mut().unwrap() ^= 1;
        let mut rx = eb.into_stream(Cursor::new(wire), pa, false).unwrap();
        let mut buf = [0u8; 5];
        assert_eq!(rx.read(&mut buf).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
