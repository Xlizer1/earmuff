//! Reading HeadsetControl's JSON output (API 1.x) and turning it into domain values.

use earmuff_domain::{
    BackendKind, Capability, ChargeState, DeviceId, DeviceInfo, EqPreset, EqualizerSpec, Event, Gain, Percent, UsbId,
};
use serde_json::Value;

/// Sidetone and mic volume are 0-128 in HeadsetControl
pub const LEVEL_MAX: i32 = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub api_version: String,
    pub devices: Vec<DeviceReport>,
    pub actions: Vec<ActionReport>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceReport {
    pub usb: UsbId,
    pub name: String,
    pub vendor: String,
    /// "success", "partial" or "failure"
    pub status: String,
    pub capabilities: Vec<String>,
    pub battery: Option<BatteryReport>,
    pub chatmix: Option<u8>,
    /// 0-128
    pub sidetone_level: Option<i32>,
    pub equalizer: Option<EqualizerSpec>,
    /// What went wrong per value, e.g. ("battery", "Device is offline or not responding")
    pub errors: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatteryReport {
    /// BATTERY_AVAILABLE, BATTERY_CHARGING, BATTERY_UNAVAILABLE, ...
    pub status: String,
    /// -1 when unknown
    pub level: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionReport {
    /// e.g. CAP_LIGHTS
    pub capability: String,
    pub success: bool,
    pub error: Option<String>,
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn hex_id(value: &Value, key: &str) -> Option<u16> {
    let raw = value.get(key)?.as_str()?;
    u16::from_str_radix(raw.trim_start_matches("0x"), 16).ok()
}

fn gain(value: &Value) -> Option<Gain> {
    value.as_f64().map(Gain::from_db)
}

fn equalizer(device: &Value) -> Option<EqualizerSpec> {
    let eq = device.get("equalizer")?;
    let presets = device
        .get("equalizer_presets")
        .and_then(Value::as_object)
        .map(|presets| {
            presets
                .iter()
                .map(|(name, curve)| EqPreset {
                    name: name.clone(),
                    curve: curve.as_array().map(|c| c.iter().filter_map(gain).collect()).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(EqualizerSpec {
        bands: usize::try_from(eq.get("bands")?.as_u64()?).ok()?,
        min: gain(eq.get("min")?)?,
        max: gain(eq.get("max")?)?,
        step: eq.get("step").and_then(gain).unwrap_or(Gain::tenths(10)),
        baseline: eq.get("baseline").and_then(gain).unwrap_or_default(),
        presets,
    })
}

fn device(value: &Value) -> Option<DeviceReport> {
    let errors = value
        .get("errors")
        .and_then(Value::as_object)
        .map(|errors| errors.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default();
    Some(DeviceReport {
        usb: UsbId::new(hex_id(value, "id_vendor")?, hex_id(value, "id_product")?),
        name: text(value, "device"),
        vendor: text(value, "vendor"),
        status: text(value, "status"),
        capabilities: value
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|caps| caps.iter().filter_map(|c| c.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        battery: value.get("battery").map(|b| BatteryReport {
            status: text(b, "status"),
            level: b.get("level").and_then(Value::as_i64).unwrap_or(-1),
        }),
        chatmix: value.get("chatmix").and_then(Value::as_u64).map(|v| v.min(u64::from(u8::MAX)) as u8),
        sidetone_level: value
            .get("sidetone")
            .and_then(|s| s.get("level"))
            .and_then(Value::as_i64)
            .map(|v| v.clamp(0, i64::from(LEVEL_MAX)) as i32),
        equalizer: equalizer(value),
        errors,
    })
}

/// Parses one run's JSON output
pub fn parse(stdout: &str) -> Result<Report, String> {
    let root: Value = serde_json::from_str(stdout).map_err(|e| format!("unreadable HeadsetControl output: {e}"))?;
    let actions = root
        .get("actions")
        .and_then(Value::as_array)
        .map(|actions| {
            actions
                .iter()
                .map(|a| ActionReport {
                    capability: text(a, "capability"),
                    success: text(a, "status") == "success",
                    error: a.get("error").or_else(|| a.get("message")).and_then(Value::as_str).map(String::from),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Report {
        api_version: text(&root, "api_version"),
        devices: root
            .get("devices")
            .and_then(Value::as_array)
            .map(|d| d.iter().filter_map(device).collect())
            .unwrap_or_default(),
        actions,
    })
}

/// Earmuff speaks HeadsetControl's JSON API 1.x
pub fn api_supported(version: &str) -> bool {
    version.split('.').next() == Some("1")
}

/// What a HeadsetControl capability lets Earmuff do
pub fn capabilities(names: &[String]) -> Vec<Capability> {
    let mut capabilities = Vec::new();
    for name in names {
        capabilities.extend_from_slice(match name.as_str() {
            "CAP_SIDETONE" => &[Capability::Sidetone, Capability::SidetoneLevel][..],
            "CAP_BATTERY_STATUS" => &[Capability::Battery],
            "CAP_NOTIFICATION_SOUND" => &[Capability::NotificationSound],
            "CAP_LIGHTS" => &[Capability::Lights],
            "CAP_INACTIVE_TIME" => &[Capability::AutoOff],
            "CAP_CHATMIX_STATUS" => &[Capability::ChatMix],
            "CAP_VOICE_PROMPTS" => &[Capability::VoicePrompts],
            "CAP_ROTATE_TO_MUTE" => &[Capability::RotateToMute],
            "CAP_EQUALIZER_PRESET" => &[Capability::EqualizerPreset],
            "CAP_EQUALIZER" => &[Capability::Equalizer],
            "CAP_MICROPHONE_MUTE_LED_BRIGHTNESS" => &[Capability::MicMuteLedBrightness],
            "CAP_MICROPHONE_VOLUME" => &[Capability::MicVolume],
            "CAP_VOLUME_LIMITER" => &[Capability::VolumeLimiter],
            "CAP_BT_WHEN_POWERED_ON" => &[Capability::BtWhenPoweredOn],
            "CAP_BT_CALL_VOLUME" => &[Capability::BtCallVolume],
            "CAP_NOISE_FILTER" => &[Capability::NoiseFilter],
            _ => &[], // e.g. CAP_SIDETONE_STATUS: read through the "sidetone" field instead
        });
    }
    capabilities
}

pub fn device_info(report: &DeviceReport) -> DeviceInfo {
    let capabilities = capabilities(&report.capabilities);
    DeviceInfo {
        id: DeviceId { backend: BackendKind::HeadsetControl, usb: report.usb, instance: report.usb.to_string() },
        name: report.name.clone(),
        vendor: report.vendor.clone(),
        equalizer: if capabilities.contains(&Capability::Equalizer)
            || capabilities.contains(&Capability::EqualizerPreset)
        {
            report.equalizer.clone()
        } else {
            None
        },
        capabilities,
    }
}

/// What a status report says about the headset
pub fn events(report: &DeviceReport) -> Vec<Event> {
    let mut events = Vec::new();
    match &report.battery {
        Some(battery) if battery.status == "BATTERY_UNAVAILABLE" => events.push(Event::Connected(false)),
        Some(battery) => {
            events.push(Event::Connected(true));
            if battery.level >= 0 {
                events.push(Event::Battery(Percent::saturating(battery.level)));
            }
            events.push(Event::Charge(match battery.status.as_str() {
                "BATTERY_CHARGING" => ChargeState::Charging,
                "BATTERY_AVAILABLE" => ChargeState::Discharging,
                _ => ChargeState::Unknown,
            }));
        }
        // wired headsets have no battery: answering at all means connected
        None => events.push(Event::Connected(report.status != "failure")),
    }
    if let Some(chatmix) = report.chatmix {
        events.push(Event::ChatMix(chatmix));
    }
    if let Some(level) = report.sidetone_level {
        events.push(Event::Sidetone(level > 0));
        events.push(Event::SidetoneLevel(Percent::from_range(level, 0, LEVEL_MAX)));
    }
    events
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Captured from `headsetcontrol --test-device -o json` (HeadsetControl 4.1.0, API 1.5),
    /// plus a HyperX Cloud Alpha Wireless that was switched off
    pub(crate) const SAMPLE: &str = r#"{
  "name": "HeadsetControl", "version": "4.1.0", "api_version": "1.5", "hidapi_version": "0.15.0",
  "device_count": 2,
  "devices": [
    {
      "status": "success", "device": "HeadsetControl Test device", "vendor": "HeadsetControl",
      "product": "Test Device", "id_vendor": "0xf00b", "id_product": "0xa00c",
      "capabilities": ["CAP_SIDETONE", "CAP_BATTERY_STATUS", "CAP_LIGHTS", "CAP_CHATMIX_STATUS",
                       "CAP_EQUALIZER_PRESET", "CAP_EQUALIZER", "CAP_NOISE_FILTER", "CAP_SIDETONE_STATUS"],
      "battery": { "status": "BATTERY_AVAILABLE", "level": 42, "voltage_mv": 3650, "time_to_empty_min": 302 },
      "equalizer": { "bands": 3, "baseline": 0, "step": 0.5, "min": -12, "max": 12 },
      "equalizer_presets_count": 2,
      "equalizer_presets": { "flat": [0.0, 0.0, 0.0], "bass boost": [6.0, 4.0, 2.0] },
      "chatmix": 64,
      "sidetone": { "level": 85, "device_level": 2, "name": "Medium" }
    },
    {
      "status": "partial", "device": "HyperX Cloud Alpha Wireless", "vendor": "HP, Inc",
      "product": "HyperX Cloud Alpha Wireless", "id_vendor": "0x03f0", "id_product": "0x098d",
      "capabilities": ["CAP_SIDETONE", "CAP_BATTERY_STATUS", "CAP_INACTIVE_TIME", "CAP_VOICE_PROMPTS"],
      "battery": { "status": "BATTERY_UNAVAILABLE", "level": -1 },
      "errors": { "battery": "Device is offline or not responding" }
    }
  ]
}"#;

    #[test]
    fn parses_devices() {
        let report = parse(SAMPLE).unwrap();
        assert_eq!(report.api_version, "1.5");
        assert_eq!(report.devices.len(), 2);
        let test = &report.devices[0];
        assert_eq!(test.usb, UsbId::new(0xF00B, 0xA00C));
        assert_eq!((test.name.as_str(), test.vendor.as_str()), ("HeadsetControl Test device", "HeadsetControl"));
        assert_eq!(test.battery, Some(BatteryReport { status: "BATTERY_AVAILABLE".into(), level: 42 }));
        assert_eq!((test.chatmix, test.sidetone_level), (Some(64), Some(85)));
        let alpha = &report.devices[1];
        assert_eq!(alpha.errors, [("battery".to_string(), "Device is offline or not responding".to_string())]);
    }

    #[test]
    fn parses_the_equalizer_with_presets_in_device_order() {
        let spec = parse(SAMPLE).unwrap().devices[0].equalizer.clone().unwrap();
        assert_eq!(
            (spec.bands, spec.min, spec.max, spec.step),
            (3, Gain::tenths(-120), Gain::tenths(120), Gain::tenths(5))
        );
        let names: Vec<&str> = spec.presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["flat", "bass boost"]); // preset numbers follow this order
        assert_eq!(spec.presets[1].curve, [Gain::tenths(60), Gain::tenths(40), Gain::tenths(20)]);
    }

    #[test]
    fn parses_action_results() {
        let report = parse(
            r#"{"api_version": "1.5", "devices": [], "actions": [
                {"capability": "CAP_LIGHTS", "device": "X", "status": "success", "value": 1},
                {"capability": "CAP_SIDETONE", "device": "X", "status": "failure", "error": "Device is offline"}]}"#,
        )
        .unwrap();
        assert_eq!(
            report.actions,
            [
                ActionReport { capability: "CAP_LIGHTS".into(), success: true, error: None },
                ActionReport {
                    capability: "CAP_SIDETONE".into(),
                    success: false,
                    error: Some("Device is offline".into())
                },
            ]
        );
    }

    #[test]
    fn rejects_output_that_is_not_json() {
        assert!(parse("Error: sidetone: value 999 out of range [0, 128]").is_err());
    }

    #[test]
    fn speaks_api_1() {
        assert!(api_supported("1.5"));
        assert!(api_supported("1.9"));
        assert!(!api_supported("2.0"));
        assert!(!api_supported(""));
    }

    #[test]
    fn maps_capabilities() {
        let names: Vec<String> = ["CAP_SIDETONE", "CAP_LIGHTS", "CAP_SIDETONE_STATUS", "CAP_SOMETHING_NEW"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(capabilities(&names), [Capability::Sidetone, Capability::SidetoneLevel, Capability::Lights]);
    }

    #[test]
    fn describes_a_device() {
        let report = parse(SAMPLE).unwrap();
        let info = device_info(&report.devices[0]);
        assert_eq!(info.id.backend, BackendKind::HeadsetControl);
        assert_eq!(info.id.instance, "f00b:a00c");
        assert!(info.supports(Capability::ChatMix) && info.supports(Capability::NoiseFilter));
        assert_eq!(info.equalizer.as_ref().map(|eq| eq.bands), Some(3));
        assert_eq!(device_info(&report.devices[1]).equalizer, None);
    }

    #[test]
    fn status_events() {
        let report = parse(SAMPLE).unwrap();
        assert_eq!(
            events(&report.devices[0]),
            [
                Event::Connected(true),
                Event::Battery(Percent::saturating(42)),
                Event::Charge(ChargeState::Discharging),
                Event::ChatMix(64),
                Event::Sidetone(true),
                Event::SidetoneLevel(Percent::saturating(66)),
            ]
        );
        assert_eq!(events(&report.devices[1]), [Event::Connected(false)]);
    }

    #[test]
    fn wired_headsets_without_a_battery_are_connected() {
        let mut report = parse(SAMPLE).unwrap().devices.remove(0);
        report.battery = None;
        report.chatmix = None;
        report.sidetone_level = None;
        assert_eq!(events(&report), [Event::Connected(true)]);
        report.status = "failure".into();
        assert_eq!(events(&report), [Event::Connected(false)]);
    }
}
