//! The sending side of the [LocalSend protocol v2](https://github.com/localsend/protocol):
//! find the LocalSend devices on the local network ([`discovery`]) and send
//! them files and folders ([`send`]).
//!
//! The crate knows nothing of any UI. Everything that touches the network
//! blocks, so run it on worker threads: [`discovery::Scan`] starts its own
//! and reports each device it finds to a callback, [`send::send`] blocks
//! until the transfer is over and reports through a shared [`send::Progress`].
//!
//! Receiving is not here: this side never runs the HTTP server that
//! `prepare-upload` and `upload` are sent to.

pub mod devices;
pub mod discovery;
pub mod mime;
pub mod protocol;
pub mod send;

pub use devices::{Device, Devices};
pub use protocol::{DeviceInfo, DeviceType, Protocol};

/// Who this side says it is, in announcements and `prepare-upload`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Me {
    pub alias: String,
    pub device_model: Option<String>,
    pub device_type: DeviceType,
    /// The SHA-256 of [`Me::identity`]'s certificate, in uppercase hex as
    /// LocalSend writes it; a random string without one. Tells our own
    /// announcements apart from the others', and receivers may check it
    /// against the certificate we connect with.
    pub fingerprint: String,
    /// The certificate presented to receivers that ask for one.
    pub identity: Option<Identity>,
}

/// A self-signed certificate (DER) and its private key (PKCS#8 PEM).
///
/// Receivers speaking protocol 2.2 (LocalSend 1.17 on Linux and Android,
/// checked) ask the sender for a certificate during the TLS handshake and
/// reset the connection without one, before any request: every route,
/// `/info` included. Which certificate doesn't matter, as LocalSend's own
/// are self-signed too.
#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    pub cert: Vec<u8>,
    pub key: String,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("cert", &self.cert.len()).finish_non_exhaustive()
    }
}

impl Identity {
    /// A new certificate for "LocalSend User", the name LocalSend gives
    /// its own, with an ECDSA P-256 key.
    #[cfg(feature = "tls")]
    pub fn generate() -> Option<Self> {
        let key = rcgen::KeyPair::generate().ok()?;
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).ok()?;
        params.distinguished_name.push(rcgen::DnType::CommonName, "LocalSend User");
        let cert = params.self_signed(&key).ok()?;
        Some(Self { cert: cert.der().to_vec(), key: key.serialize_pem() })
    }

    /// The certificate's SHA-256 in uppercase hex.
    #[cfg(feature = "tls")]
    pub fn fingerprint(&self) -> String {
        ring::digest::digest(&ring::digest::SHA256, &self.cert).as_ref().iter().map(|b| format!("{b:02X}")).collect()
    }
}

impl Me {
    /// A desktop device called `alias`, with a certificate (with the `tls`
    /// feature) and a fingerprint new to this run.
    pub fn new(alias: impl Into<String>, device_model: Option<String>) -> Self {
        #[cfg(feature = "tls")]
        let identity = Identity::generate();
        #[cfg(not(feature = "tls"))]
        let identity: Option<Identity> = None;
        #[cfg(feature = "tls")]
        let fingerprint = identity.as_ref().map_or_else(random_fingerprint, Identity::fingerprint);
        #[cfg(not(feature = "tls"))]
        let fingerprint = random_fingerprint();
        Self { alias: alias.into(), device_model, device_type: DeviceType::Desktop, fingerprint, identity }
    }

    /// Our part of `prepare-upload` and of an announcement. `port` and
    /// `protocol` are the defaults since nothing is served on them.
    pub fn info(&self) -> DeviceInfo {
        DeviceInfo {
            alias: self.alias.clone(),
            version: protocol::VERSION.into(),
            device_model: self.device_model.clone(),
            device_type: Some(self.device_type),
            fingerprint: self.fingerprint.clone(),
            port: Some(protocol::PORT),
            protocol: Some(Protocol::Https),
            download: Some(false),
        }
    }
}

/// 32 hex digits from the standard library's randomly seeded hasher, the
/// clock and the process id: random enough to be told apart, which is all
/// a fingerprint does for a side without a certificate.
fn random_fingerprint() -> String {
    use std::hash::{BuildHasher, Hasher};
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let half = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(time);
        h.write_u32(std::process::id());
        h.write_u64(salt);
        h.finish()
    };
    format!("{:016x}{:016x}", half(1), half(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_differ() {
        let (a, b) = (Me::new("A", None), Me::new("A", None));
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn the_fingerprint_is_the_certificates_hash() {
        let me = Me::new("A", None);
        let identity = me.identity.as_ref().expect("certificate");
        assert_eq!(me.fingerprint.len(), 64);
        assert!(me.fingerprint.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_uppercase()));
        assert_eq!(me.fingerprint, identity.fingerprint());
    }
}
