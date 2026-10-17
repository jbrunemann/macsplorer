//! The devices a scan found, and where to reach them.

use std::net::{IpAddr, SocketAddr};

use crate::protocol::{DeviceInfo, DeviceType, Protocol};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub info: DeviceInfo,
    pub ip: IpAddr,
    pub port: u16,
    pub protocol: Protocol,
}

impl Device {
    /// The device that sent `info` from `ip`. Port and protocol come from
    /// `info` when it has them, from where it was found otherwise.
    pub fn new(info: DeviceInfo, ip: IpAddr, port: u16, protocol: Protocol) -> Self {
        let port = info.port.unwrap_or(port);
        let protocol = info.protocol.unwrap_or(protocol);
        Self { info, ip, port, protocol }
    }

    /// Tells devices apart: the fingerprint (the certificate's hash under
    /// HTTPS), or the address when a device sends none.
    pub fn key(&self) -> String {
        if self.info.fingerprint.is_empty() { self.addr().to_string() } else { self.info.fingerprint.clone() }
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }

    pub fn alias(&self) -> &str {
        &self.info.alias
    }

    pub fn device_type(&self) -> DeviceType {
        self.info.device_type.unwrap_or(DeviceType::Desktop)
    }

    /// The start of every URL on the device, such as
    /// `https://192.168.1.20:53317/api/localsend/v2`.
    pub fn base_url(&self) -> String {
        format!("{}://{}{}", self.protocol.scheme(), self.addr(), crate::protocol::API)
    }
}

/// The devices found so far, in alias order (case folded), one per key.
#[derive(Clone, Debug, Default)]
pub struct Devices {
    list: Vec<Device>,
}

impl Devices {
    /// Adds a device, or replaces the one with its key (it may have a new
    /// address or name). Returns whether anything changed.
    pub fn insert(&mut self, device: Device) -> bool {
        let key = device.key();
        if let Some(i) = self.list.iter().position(|d| d.key() == key) {
            if self.list[i] == device {
                return false;
            }
            self.list.remove(i);
        }
        let at = self.list.partition_point(|d| sort_key(d) <= sort_key(&device));
        self.list.insert(at, device);
        true
    }

    pub fn get(&self, key: &str) -> Option<&Device> {
        self.list.iter().find(|d| d.key() == key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Device> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }
}

fn sort_key(d: &Device) -> (String, String) {
    (d.alias().to_lowercase(), d.key())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(alias: &str, fingerprint: &str, ip: [u8; 4]) -> Device {
        let info = DeviceInfo {
            alias: alias.into(),
            version: "2.0".into(),
            device_model: None,
            device_type: None,
            fingerprint: fingerprint.into(),
            port: None,
            protocol: None,
            download: None,
        };
        Device::new(info, IpAddr::from(ip), 53317, Protocol::Https)
    }

    #[test]
    fn devices_are_sorted_by_alias_and_kept_once() {
        let mut d = Devices::default();
        assert!(d.insert(device("phone", "b", [10, 0, 0, 2])));
        assert!(d.insert(device("Laptop", "a", [10, 0, 0, 3])));
        assert!(!d.insert(device("phone", "b", [10, 0, 0, 2])));
        // Same device, new address: replaced, not added.
        assert!(d.insert(device("Phone", "b", [10, 0, 0, 9])));
        let names: Vec<_> = d.iter().map(|d| (d.alias().to_owned(), d.ip.to_string())).collect();
        assert_eq!(names, [("Laptop".to_owned(), "10.0.0.3".to_owned()), ("Phone".to_owned(), "10.0.0.9".to_owned())]);
    }

    #[test]
    fn a_device_without_fingerprint_is_known_by_its_address() {
        let d = device("x", "", [192, 168, 1, 5]);
        assert_eq!(d.key(), "192.168.1.5:53317");
        assert_eq!(d.base_url(), "https://192.168.1.5:53317/api/localsend/v2");
    }
}
