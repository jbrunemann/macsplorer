//! The [LocalSend protocol v2](https://github.com/localsend/protocol): find
//! the LocalSend devices on the local network ([`discovery`]), send them
//! files and folders ([`send`]) and take the files they send ([`receive`]).
//!
//! The crate knows nothing of any UI. Everything that touches the network
//! blocks, so run it on worker threads: [`discovery::Scan`] and
//! [`receive::Receiver`] start their own and report to a callback,
//! [`send::send`] blocks until the transfer is over and reports through a
//! shared [`send::Progress`].

pub mod devices;
pub mod discovery;
pub mod mime;
pub mod protocol;
pub mod receive;
pub mod send;
mod http;
mod time;

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
    /// The certificate presented to receivers that ask for one, and the
    /// one [`receive::Receiver`] serves HTTPS with.
    pub identity: Option<Identity>,
    /// Where [`receive::Receiver`] takes requests, as announced. Nothing
    /// is served there unless one runs.
    pub port: u16,
    /// HTTPS with an identity, HTTP without.
    pub protocol: Protocol,
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

    /// The certificate and the key as one PEM text, the way
    /// [`Identity::load_or_generate`] stores them.
    #[cfg(feature = "tls")]
    pub fn to_pem(&self) -> String {
        let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
        let b64 = base64(&self.cert);
        for line in b64.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(line).unwrap_or_default());
            out.push('\n');
        }
        out.push_str("-----END CERTIFICATE-----\n");
        out.push_str(&self.key);
        out
    }

    /// Reads [`Identity::to_pem`]'s text.
    #[cfg(feature = "tls")]
    pub fn from_pem(text: &str) -> Option<Self> {
        use rustls::pki_types::pem::PemObject;
        let cert = rustls::pki_types::CertificateDer::from_pem_slice(text.as_bytes()).ok()?;
        let start = text.find("-----BEGIN PRIVATE KEY-----")?;
        let key = text[start..].to_owned();
        rustls::pki_types::PrivateKeyDer::from_pem_slice(key.as_bytes()).ok()?;
        Some(Self { cert: cert.to_vec(), key })
    }

    /// The identity stored at `path`, or a new one, stored there for the
    /// next run (readable by the user only). Keeping it keeps the
    /// fingerprint, so other devices know this one again.
    #[cfg(feature = "tls")]
    pub fn load_or_generate(path: &std::path::Path) -> Option<Self> {
        if let Some(identity) = std::fs::read_to_string(path).ok().and_then(|t| Self::from_pem(&t)) {
            return Some(identity);
        }
        let identity = Self::generate()?;
        let _ = write_private(path, identity.to_pem().as_bytes());
        Some(identity)
    }
}

/// Writes a file only its owner can read, replacing it in one step.
#[cfg(feature = "tls")]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&temp)?.write_all(bytes)?;
    std::fs::rename(&temp, path)
}

#[cfg(feature = "tls")]
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ABC[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

impl Me {
    /// A desktop device called `alias`, with a certificate (with the `tls`
    /// feature) and a fingerprint new to this run.
    pub fn new(alias: impl Into<String>, device_model: Option<String>) -> Self {
        #[cfg(feature = "tls")]
        let identity = Identity::generate();
        #[cfg(not(feature = "tls"))]
        let identity = None;
        Self::with_identity(alias, device_model, identity)
    }

    /// A desktop device called `alias` with `identity` (one kept from an
    /// earlier run, [`Identity::load_or_generate`]); without one the
    /// fingerprint is new to this run.
    pub fn with_identity(alias: impl Into<String>, device_model: Option<String>, identity: Option<Identity>) -> Self {
        #[cfg(feature = "tls")]
        let fingerprint = identity.as_ref().map_or_else(random_fingerprint, Identity::fingerprint);
        #[cfg(not(feature = "tls"))]
        let fingerprint = random_fingerprint();
        let protocol = if cfg!(feature = "tls") && identity.is_some() { Protocol::Https } else { Protocol::Http };
        Self { alias: alias.into(), device_model, device_type: DeviceType::Desktop, fingerprint, identity, port: protocol::PORT, protocol }
    }

    /// Our part of `prepare-upload`, `/info` and announcements.
    pub fn info(&self) -> DeviceInfo {
        DeviceInfo {
            alias: self.alias.clone(),
            version: protocol::VERSION.into(),
            device_model: self.device_model.clone(),
            device_type: Some(self.device_type),
            fingerprint: self.fingerprint.clone(),
            port: Some(self.port),
            protocol: Some(self.protocol),
            download: Some(false),
        }
    }
}

/// 32 hex digits from the standard library's randomly seeded hasher, the
/// clock and the process id: random enough to be told apart, which is all
/// a fingerprint does for a side without a certificate.
pub(crate) fn random_fingerprint() -> String {
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
        assert_eq!(me.protocol, Protocol::Https);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn an_identity_is_kept_between_runs() {
        let dir = std::env::temp_dir().join(format!("localsend-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("identity.pem");
        let first = Identity::load_or_generate(&path).unwrap();
        let again = Identity::load_or_generate(&path).unwrap();
        assert_eq!(first, again);
        assert_eq!(Identity::from_pem(&first.to_pem()).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A damaged file is replaced.
        std::fs::write(&path, "nonsense").unwrap();
        assert_ne!(Identity::load_or_generate(&path).unwrap(), first);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
