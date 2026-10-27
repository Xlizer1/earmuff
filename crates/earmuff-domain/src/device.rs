//! What a headset is and what it can do.

use std::fmt;
use std::str::FromStr;

use crate::equalizer::EqualizerSpec;
use crate::status::Setting;

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

/// Accepts "03f0:098d" and "0x03f0:0x098d"
impl FromStr for UsbId {
    type Err = UsbIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = || UsbIdError(s.to_string());
        let (vendor, product) = s.trim().split_once(':').ok_or_else(error)?;
        let hex = |part: &str| {
            let digits = part.strip_prefix("0x").unwrap_or(part);
            if digits.is_empty() || digits.len() > 4 {
                return Err(error());
            }
            u16::from_str_radix(digits, 16).map_err(|_| error())
        };
        Ok(UsbId::new(hex(vendor)?, hex(product)?))
    }
}

/// Which adapter talks to a headset
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// Earmuff's own driver, talking to the USB dongle directly
    Native,
    /// The HeadsetControl program, for headsets without a native driver
    HeadsetControl,
    /// A simulated headset
    Demo,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            BackendKind::Native => "native",
            BackendKind::HeadsetControl => "HeadsetControl",
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
    Lights,
    /// Game/chat balance dial (read only)
    ChatMix,
    /// Mutes when the mic boom is raised
    RotateToMute,
    MicVolume,
    MicMuteLedBrightness,
    NoiseFilter,
    VolumeLimiter,
    /// Bluetooth turns on with the headset
    BtWhenPoweredOn,
    BtCallVolume,
    EqualizerPreset,
    Equalizer,
    NotificationSound,
}

impl Capability {
    /// What it's called in messages and on the command line
    pub fn name(self) -> &'static str {
        match self {
            Capability::Battery => "battery",
            Capability::MicMute => "mute",
            Capability::MicAttached => "mic boom detection",
            Capability::Sidetone | Capability::SidetoneLevel => "sidetone",
            Capability::AutoOff => "auto-off",
            Capability::VoicePrompts => "voice-prompts",
            Capability::Lights => "lights",
            Capability::ChatMix => "chatmix",
            Capability::RotateToMute => "rotate-to-mute",
            Capability::MicVolume => "mic-volume",
            Capability::MicMuteLedBrightness => "mute-led",
            Capability::NoiseFilter => "noise-filter",
            Capability::VolumeLimiter => "volume-limiter",
            Capability::BtWhenPoweredOn => "bluetooth-on-power",
            Capability::BtCallVolume => "bluetooth-call-volume",
            Capability::EqualizerPreset => "eq-preset",
            Capability::Equalizer => "eq",
            Capability::NotificationSound => "notification-sound",
        }
    }
}

/// Why a headset can't take a setting
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingError {
    Unsupported(Capability),
    Invalid(String),
}

impl fmt::Display for SettingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SettingError::Unsupported(capability) => write!(f, "this headset has no {} setting", capability.name()),
            SettingError::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for SettingError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub vendor: String,
    pub capabilities: Vec<Capability>,
    /// What its hardware equalizer can do, if it has one
    pub equalizer: Option<EqualizerSpec>,
}

impl DeviceInfo {
    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Whether this headset can take `setting`, with that value
    pub fn check(&self, setting: &Setting) -> Result<(), SettingError> {
        let capability = setting.capability();
        if !self.supports(capability) {
            return Err(SettingError::Unsupported(capability));
        }
        match setting {
            Setting::MicMuteLedBrightness(level) if *level > 3 => {
                Err(SettingError::Invalid(format!("mute LED brightness goes from 0 to 3, not {level}")))
            }
            Setting::Equalizer(curve) => match &self.equalizer {
                Some(spec) => spec.check(curve).map_err(SettingError::Invalid),
                None => Err(SettingError::Invalid("this headset doesn't describe its equalizer".into())),
            },
            Setting::EqualizerPreset(index) => {
                let presets = self.equalizer.as_ref().map_or(0, |spec| spec.presets.len());
                // a headset that doesn't name its presets accepts any index
                if presets > 0 && usize::from(*index) >= presets {
                    Err(SettingError::Invalid(format!(
                        "this headset has {presets} presets, numbered 0 to {}",
                        presets - 1
                    )))
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equalizer::{EqPreset, Gain};

    fn info(capabilities: Vec<Capability>, equalizer: Option<EqualizerSpec>) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "demo-1".into() },
            name: "Test".into(),
            vendor: "Vendor".into(),
            capabilities,
            equalizer,
        }
    }

    fn spec() -> EqualizerSpec {
        EqualizerSpec {
            bands: 2,
            min: Gain::tenths(-60),
            max: Gain::tenths(60),
            step: Gain::tenths(10),
            baseline: Gain::tenths(0),
            presets: vec![EqPreset { name: "flat".into(), curve: vec![Gain::tenths(0); 2] }],
        }
    }

    #[test]
    fn usb_id_round_trips_through_text() {
        let id = UsbId::new(0x03F0, 0x098D);
        assert_eq!(id.to_string(), "03f0:098d");
        assert_eq!("03f0:098d".parse(), Ok(id));
        assert_eq!("03F0:098D".parse(), Ok(id));
        assert_eq!("0x03f0:0x098d".parse(), Ok(id));
    }

    #[test]
    fn usb_id_rejects_garbage() {
        assert!("03f0".parse::<UsbId>().is_err());
        assert!("zzzz:098d".parse::<UsbId>().is_err());
        assert!("03f0:12345".parse::<UsbId>().is_err());
        assert!(":098d".parse::<UsbId>().is_err());
        assert_eq!("x".parse::<UsbId>().unwrap_err().to_string(), "x is not a USB ID like 03f0:098d");
    }

    #[test]
    fn device_info_reports_capabilities() {
        let info = info(vec![Capability::Battery, Capability::MicMute], None);
        assert!(info.supports(Capability::MicMute));
        assert!(!info.supports(Capability::Sidetone));
        assert_eq!(info.id.to_string(), "0001:0002 (demo, demo-1)");
        assert_eq!(BackendKind::HeadsetControl.to_string(), "HeadsetControl");
    }

    #[test]
    fn check_refuses_unsupported_settings() {
        let info = info(vec![Capability::MicMute], None);
        assert_eq!(info.check(&Setting::MicMuted(true)), Ok(()));
        let error = info.check(&Setting::Lights(true)).unwrap_err();
        assert_eq!(error, SettingError::Unsupported(Capability::Lights));
        assert_eq!(error.to_string(), "this headset has no lights setting");
    }

    #[test]
    fn check_validates_values_against_the_headset() {
        let info = info(
            vec![Capability::Equalizer, Capability::EqualizerPreset, Capability::MicMuteLedBrightness],
            Some(spec()),
        );
        assert_eq!(info.check(&Setting::Equalizer(vec![Gain::tenths(10); 2])), Ok(()));
        assert!(info.check(&Setting::Equalizer(vec![Gain::tenths(10); 3])).is_err());
        assert!(info.check(&Setting::Equalizer(vec![Gain::tenths(90); 2])).is_err());
        assert_eq!(info.check(&Setting::EqualizerPreset(0)), Ok(()));
        assert_eq!(
            info.check(&Setting::EqualizerPreset(1)).unwrap_err().to_string(),
            "this headset has 1 presets, numbered 0 to 0"
        );
        assert_eq!(info.check(&Setting::MicMuteLedBrightness(3)), Ok(()));
        assert!(info.check(&Setting::MicMuteLedBrightness(4)).is_err());
    }

    #[test]
    fn capabilities_have_command_line_names() {
        assert_eq!(Capability::MicMute.name(), "mute");
        assert_eq!(Capability::SidetoneLevel.name(), "sidetone");
        assert_eq!(Capability::Equalizer.name(), "eq");
    }
}
