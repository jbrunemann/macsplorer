//! The messages of the LocalSend protocol v2 (v2.2 of the spec), as JSON.
//!
//! All HTTP routes are under [`API`] on port [`PORT`]; discovery also
//! multicasts on [`MULTICAST`]`:`[`PORT`].

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use serde::{Deserialize, Deserializer, Serialize};

/// The protocol version this side speaks (major.minor).
pub const VERSION: &str = "2.0";
/// The default port, for UDP and TCP alike.
pub const PORT: u16 = 53317;
/// The default multicast group. The spec picks one in 224.0.0.0/24 because
/// some Android devices reject any other.
pub const MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
pub const API: &str = "/api/localsend/v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Http,
    Https,
}

impl Protocol {
    pub fn scheme(self) -> &'static str {
        match self {
            Protocol::Http => "http",
            Protocol::Https => "https",
        }
    }
}

impl<'de> Deserialize<'de> for Protocol {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Anything but "http" is taken as the default, HTTPS.
        Ok(if String::deserialize(d)?.eq_ignore_ascii_case("http") { Protocol::Http } else { Protocol::Https })
    }
}

/// What kind of device it is. The protocol treats them all the same; it
/// only picks the icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Mobile,
    Desktop,
    Web,
    Headless,
    Server,
}

impl<'de> Deserialize<'de> for DeviceType {
    /// Unknown values become `Desktop`, as in the official implementation.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "mobile" => DeviceType::Mobile,
            "web" => DeviceType::Web,
            "headless" => DeviceType::Headless,
            "server" => DeviceType::Server,
            _ => DeviceType::Desktop,
        })
    }
}

/// A device as it describes itself in announcements, `/register`, `/info`
/// and `prepare-upload`. `/info` and the `/register` answer leave out
/// `port` and `protocol`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub alias: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub device_model: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub device_type: Option<DeviceType>,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Protocol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<bool>,
}

fn default_version() -> String {
    "1.0".into()
}

/// A `null` device type is `None`, any string goes through [`DeviceType`]'s
/// fallback.
fn lenient<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DeviceType>, D::Error> {
    Option::<DeviceType>::deserialize(d)
}

/// The multicast message: an announcement (`announce` true) or the answer
/// to one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    #[serde(flatten)]
    pub info: DeviceInfo,
    /// v1 devices call it `announcement`.
    #[serde(default, alias = "announcement")]
    pub announce: bool,
}

/// File details in `prepare-upload`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub id: String,
    /// The name the receiver saves under. A folder's files carry their path
    /// inside it, `/`-separated ("Photos/2024/a.jpg"), which the official
    /// app recreates.
    pub file_name: String,
    pub size: u64,
    /// A MIME type ([`crate::mime`]).
    pub file_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<FileMetadata>,
}

/// ISO 8601 times, such as "2021-01-01T12:34:56Z".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessed: Option<String>,
}

/// The body of `POST /prepare-upload`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareUpload {
    pub info: DeviceInfo,
    pub files: BTreeMap<String, FileInfo>,
}

/// The answer to `prepare-upload`: a token for each file the receiver took.
/// Files it turned down have none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareResponse {
    pub session_id: String,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_announcement_from_the_spec_reads() {
        let json = r#"{"alias":"Nice Orange","version":"2.0","deviceModel":"Samsung","deviceType":"mobile",
            "fingerprint":"random string","port":53317,"protocol":"https","download":true,"announce":true}"#;
        let a: Announcement = serde_json::from_str(json).unwrap();
        assert!(a.announce);
        assert_eq!(a.info.alias, "Nice Orange");
        assert_eq!(a.info.device_type, Some(DeviceType::Mobile));
        assert_eq!(a.info.protocol, Some(Protocol::Https));
        assert_eq!(a.info.port, Some(53317));
    }

    #[test]
    fn unknown_or_missing_fields_fall_back() {
        let a: Announcement = serde_json::from_str(r#"{"alias":"X","deviceType":"toaster","deviceModel":null,"announcement":true}"#).unwrap();
        assert_eq!(a.info.device_type, Some(DeviceType::Desktop));
        assert!(a.announce);
        assert_eq!(a.info.port, None);
        let a: Announcement = serde_json::from_str(r#"{"alias":"X","deviceType":null}"#).unwrap();
        assert_eq!(a.info.device_type, None);
        assert!(!a.announce);
    }

    #[test]
    fn prepare_upload_is_camel_case_without_empty_options() {
        let file = FileInfo { id: "1".into(), file_name: "a b.png".into(), size: 3, file_type: "image/png".into(), sha256: None, preview: None, metadata: None };
        let body = PrepareUpload {
            info: crate::Me { alias: "Mac".into(), device_model: None, device_type: DeviceType::Desktop, fingerprint: "f".into(), identity: None }.info(),
            files: [("1".to_owned(), file)].into(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["files"]["1"]["fileName"], "a b.png");
        assert_eq!(json["files"]["1"]["fileType"], "image/png");
        assert!(json["files"]["1"].get("sha256").is_none());
        assert_eq!(json["info"]["deviceType"], "desktop");
        assert_eq!(json["info"]["protocol"], "https");
        assert_eq!(json["info"]["version"], "2.0");
    }

    #[test]
    fn prepare_response_reads() {
        let r: PrepareResponse = serde_json::from_str(r#"{"sessionId":"s","files":{"a":"t1","b":"t2"}}"#).unwrap();
        assert_eq!(r.session_id, "s");
        assert_eq!(r.files["b"], "t2");
    }
}
