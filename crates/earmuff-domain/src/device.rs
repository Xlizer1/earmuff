//! What a headset is and what it can do.

use std::fmt;
use std::str::FromStr;

/// USB vendor and product ID, written like `03f0:098d`
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UsbId {
    pub vendor: u16,
    pub product: u16,
}

impl UsbId {
    pub const fn new(vendor: u16, product: u16) -> Self {
        UsbId { vendor, product }
    }
}

impl fmt::Display for UsbId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:04x}:{:04x}", self.vendor, self.product)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbIdError(String);

impl fmt::Display for UsbIdError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} is not a USB ID like 03f0:098d", self.0)
    }
}

impl std::error::Error for UsbIdError {}

impl FromStr for UsbId {
    type Err = UsbIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = || UsbIdError(s.to_string());
        let (vendor, product) = s.trim().split_once(':').ok_or_else(error)?;
        let hex = |part: &str| u16::from_str_radix(part, 16).map_err(|_| error());
        Ok(UsbId::new(hex(vendor)?, hex(product)?))
    }
}

/// Which adapter talks to a headset
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// Earmuff's own driver, talking to the USB dongle directly
    Native,
    /// A simulated headset
    Demo,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            BackendKind::Native => "native",
            BackendKind::Demo => "demo",
        })
    }
}

/// One connected headset
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId {
    pub backend: BackendKind,
    pub usb: UsbId,
    /// Where the backend reaches it, e.g. `/dev/hidraw3`
    pub instance: String,
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} ({}, {})", self.usb, self.backend, self.instance)
    }
}

/// Something a headset can do. The UI and the command line only offer what a headset supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Battery level and charging state
    Battery,
    MicMute,
    /// Whether the detachable mic boom is plugged in
    MicAttached,
    /// Mic monitoring on/off
    Sidetone,
    /// Mic monitoring volume
    SidetoneLevel,
    AutoOff,
    VoicePrompts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub vendor: String,
    pub capabilities: Vec<Capability>,
}

impl DeviceInfo {
    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_id_round_trips_through_text() {
        let id = UsbId::new(0x03F0, 0x098D);
        assert_eq!(id.to_string(), "03f0:098d");
        assert_eq!("03f0:098d".parse(), Ok(id));
        assert_eq!("03F0:098D".parse(), Ok(id));
    }

    #[test]
    fn usb_id_rejects_garbage() {
        assert!("03f0".parse::<UsbId>().is_err());
        assert!("zzzz:098d".parse::<UsbId>().is_err());
        assert!("03f0:12345".parse::<UsbId>().is_err());
        assert_eq!("x".parse::<UsbId>().unwrap_err().to_string(), "x is not a USB ID like 03f0:098d");
    }

    #[test]
    fn device_info_reports_capabilities() {
        let info = DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "demo-1".into() },
            name: "Test".into(),
            vendor: "Vendor".into(),
            capabilities: vec![Capability::Battery, Capability::MicMute],
        };
        assert!(info.supports(Capability::MicMute));
        assert!(!info.supports(Capability::Sidetone));
        assert_eq!(info.id.to_string(), "0001:0002 (demo, demo-1)");
    }
}
