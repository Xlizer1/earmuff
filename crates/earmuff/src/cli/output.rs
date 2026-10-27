//! How the command line prints things. Pure functions, so they are unit-tested.

use earmuff_domain::{Capability, DeviceInfo, Event, Gain, Status};
use serde_json::{Value, json};

fn known<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "?".into(), |v| v.to_string())
}

fn yes_no(value: Option<bool>, yes: &str, no: &str) -> String {
    known(value.map(|v| if v { yes } else { no }))
}

fn curve_text(curve: &[Gain]) -> String {
    curve.iter().map(Gain::to_string).collect::<Vec<_>>().join(", ")
}

/// "64 (even)", "30 (more game)", "100 (more chat)"
pub fn chatmix_text(value: u8) -> String {
    let side = match value {
        0..=59 => "more game",
        60..=68 => "even",
        _ => "more chat",
    };
    format!("{value} ({side})")
}

/// The name of an equalizer preset, or its number when the headset doesn't name them
pub fn preset_name(info: &DeviceInfo, index: u8) -> String {
    info.equalizer
        .as_ref()
        .and_then(|spec| spec.presets.get(usize::from(index)))
        .map_or_else(|| index.to_string(), |preset| preset.name.clone())
}

/// The `status` command's text: one line per thing the headset supports
pub fn status_text(info: &DeviceInfo, s: &Status) -> String {
    if !s.is_connected() {
        return format!("Headset      {}: off (dongle plugged in)\n", info.name);
    }
    let has = |capability| info.supports(capability);
    let mut lines = vec![match &s.color {
        Some(color) => format!("{}, connected ({color})", info.name),
        None if has(Capability::MicAttached) => format!("{}, connected (?)", info.name),
        None => format!("{}, connected", info.name),
    }];
    let mut line = |label: &str, value: String| lines.push(format!("{label:<13}{value}"));
    if has(Capability::Battery) {
        line("Battery", format!("{}, {}", known(s.battery), known(s.charge)));
    }
    let mut mic = Vec::new();
    if has(Capability::MicAttached) {
        mic.push(yes_no(s.mic_attached, "attached", "detached"));
    }
    if has(Capability::MicMute) {
        mic.push(yes_no(s.mic_muted, "muted", "not muted"));
    }
    if !mic.is_empty() {
        line("Mic", mic.join(", "));
    }
    if has(Capability::Sidetone) || has(Capability::SidetoneLevel) {
        line("Monitoring", format!("{}, level {}", yes_no(s.sidetone, "on", "off"), known(s.sidetone_level)));
    }
    if has(Capability::AutoOff) {
        let auto_off = s.auto_off.map(|a| if a.is_never() { "never".into() } else { format!("{} min", a.get()) });
        line("Auto-off", known(auto_off));
    }
    if has(Capability::VoicePrompts) {
        line("Voice", yes_no(s.voice_prompts, "on", "off"));
    }
    if has(Capability::Lights) {
        line("Lights", yes_no(s.lights, "on", "off"));
    }
    if has(Capability::ChatMix) {
        line("ChatMix", known(s.chatmix.map(chatmix_text)));
    }
    if has(Capability::RotateToMute) {
        line("Rotate mute", yes_no(s.rotate_to_mute, "on", "off"));
    }
    if has(Capability::MicVolume) {
        line("Mic volume", known(s.mic_volume));
    }
    if has(Capability::MicMuteLedBrightness) {
        line("Mute LED", known(s.mic_mute_led_brightness.map(|level| format!("{level} of 3"))));
    }
    if has(Capability::NoiseFilter) {
        line("Noise filter", known(s.noise_filter));
    }
    if has(Capability::VolumeLimiter) {
        line("Volume limit", yes_no(s.volume_limiter, "on", "off"));
    }
    if has(Capability::BtWhenPoweredOn) {
        line("Bluetooth", yes_no(s.bt_when_powered_on, "on at power-on", "off at power-on"));
    }
    if has(Capability::BtCallVolume) {
        line("Call volume", known(s.bt_call_volume));
    }
    if has(Capability::EqualizerPreset) {
        line("EQ preset", known(s.equalizer_preset.map(|index| preset_name(info, index))));
    }
    if has(Capability::Equalizer) {
        line("EQ", known(s.equalizer.as_deref().map(curve_text)));
    }
    let mut text = format!("{:<13}{}\n", "Headset", lines[0]);
    for rest in &lines[1..] {
        text.push_str(rest);
        text.push('\n');
    }
    text
}

/// The `status --json` output. Unknown values are null.
pub fn status_json(info: &DeviceInfo, s: &Status) -> Value {
    json!({
        "device": info.name,
        "vendor": info.vendor,
        "usb_id": info.id.usb.to_string(),
        "backend": info.id.backend.to_string(),
        "connected": s.connected,
        "battery": s.battery.map(|b| b.get()),
        "charging": s.charge.map(|c| c.to_string()),
        "muted": s.mic_muted,
        "mic_attached": s.mic_attached,
        "sidetone": s.sidetone,
        "sidetone_level": s.sidetone_level.map(|l| l.get()),
        "auto_off": s.auto_off.map(|a| a.get()),
        "voice_prompts": s.voice_prompts,
        "color": s.color,
        "lights": s.lights,
        "chatmix": s.chatmix,
        "rotate_to_mute": s.rotate_to_mute,
        "mic_volume": s.mic_volume.map(|v| v.get()),
        "mic_mute_led_brightness": s.mic_mute_led_brightness,
        "noise_filter": s.noise_filter.map(|f| f.to_string()),
        "volume_limiter": s.volume_limiter,
        "bluetooth_when_powered_on": s.bt_when_powered_on,
        "bluetooth_call_volume": s.bt_call_volume.map(|v| v.get()),
        "equalizer_preset": s.equalizer_preset,
        "equalizer": s.equalizer.as_ref().map(|curve| curve.iter().map(|g| g.db()).collect::<Vec<_>>()),
    })
}

/// One line of JSON for a Waybar custom module. `None` (no headset) hides the module.
pub fn waybar_json(status: Option<&Status>) -> Value {
    let Some(s) = status else {
        return json!({ "text": "" });
    };
    if !s.is_connected() {
        return json!({ "text": "off", "tooltip": "Headset is off", "class": "off" });
    }
    let level = s.battery.map_or(0, |b| b.get());
    let charging = s.charge.is_some_and(|c| c.is_plugged_in());
    let class = if charging {
        "charging"
    } else if level <= 15 {
        "low"
    } else {
        "ok"
    };
    json!({
        "text": format!("{level}%"),
        "tooltip": format!("Headset battery: {level}%, {}", known(s.charge)),
        "class": class,
        "percentage": level,
    })
}

/// One line of `monitor` output
pub fn event_line(event: &Event) -> String {
    match event {
        Event::Connected(v) => format!("connected = {v}"),
        Event::Battery(v) => format!("battery = {v}"),
        Event::Charge(v) => format!("charge = {v}"),
        Event::MicMuted(v) => format!("mic_muted = {v}"),
        Event::MicAttached(v) => format!("mic_attached = {v}"),
        Event::Sidetone(v) => format!("sidetone = {v}"),
        Event::SidetoneLevel(v) => format!("sidetone_level = {v}"),
        Event::AutoOff(v) => format!("auto_off = {v}"),
        Event::VoicePrompts(v) => format!("voice_prompts = {v}"),
        Event::Color(v) => format!("color = {v}"),
        Event::Lights(v) => format!("lights = {v}"),
        Event::ChatMix(v) => format!("chatmix = {}", chatmix_text(*v)),
        Event::RotateToMute(v) => format!("rotate_to_mute = {v}"),
        Event::MicVolume(v) => format!("mic_volume = {v}"),
        Event::MicMuteLedBrightness(v) => format!("mute_led = {v}"),
        Event::NoiseFilter(v) => format!("noise_filter = {v}"),
        Event::VolumeLimiter(v) => format!("volume_limiter = {v}"),
        Event::BtWhenPoweredOn(v) => format!("bluetooth_when_powered_on = {v}"),
        Event::BtCallVolume(v) => format!("bluetooth_call_volume = {v}"),
        Event::EqualizerPreset(v) => format!("eq_preset = {v}"),
        Event::Equalizer(curve) => format!("eq = {}", curve_text(curve)),
        Event::Raw(report) => {
            let hex: Vec<String> = report.iter().map(|b| format!("{b:02x}")).collect();
            format!("raw  {}", hex.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{
        AutoOff, BackendKind, ChargeState, DeviceId, EqPreset, EqualizerSpec, NoiseFilter, Percent, UsbId,
    };

    const CLOUD_ALPHA: [Capability; 7] = [
        Capability::Battery,
        Capability::MicMute,
        Capability::MicAttached,
        Capability::Sidetone,
        Capability::SidetoneLevel,
        Capability::AutoOff,
        Capability::VoicePrompts,
    ];

    fn info(capabilities: &[Capability]) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(0x03F0, 0x098D), instance: "demo-1".into() },
            name: "Cloud Alpha Wireless".into(),
            vendor: "HyperX".into(),
            capabilities: capabilities.to_vec(),
            equalizer: Some(EqualizerSpec {
                bands: 2,
                min: Gain::tenths(-120),
                max: Gain::tenths(120),
                step: Gain::tenths(5),
                baseline: Gain::tenths(0),
                presets: vec![
                    EqPreset { name: "flat".into(), curve: vec![Gain::tenths(0); 2] },
                    EqPreset { name: "bass boost".into(), curve: vec![Gain::tenths(60), Gain::tenths(0)] },
                ],
            }),
        }
    }

    fn full() -> Status {
        Status {
            connected: Some(true),
            battery: Some(Percent::saturating(99)),
            charge: Some(ChargeState::Discharging),
            mic_muted: Some(false),
            mic_attached: Some(true),
            sidetone: Some(false),
            sidetone_level: Some(Percent::ZERO),
            auto_off: Some(AutoOff::minutes(20)),
            voice_prompts: Some(true),
            color: Some("Black/Red".into()),
            ..Status::default()
        }
    }

    #[test]
    fn status_text_lists_every_setting() {
        assert_eq!(
            status_text(&info(&CLOUD_ALPHA), &full()),
            "Headset      Cloud Alpha Wireless, connected (Black/Red)\n\
             Battery      99%, discharging\n\
             Mic          attached, not muted\n\
             Monitoring   off, level 0%\n\
             Auto-off     20 min\n\
             Voice        on\n"
        );
    }

    #[test]
    fn status_text_marks_unknown_values() {
        let status = Status { connected: Some(true), auto_off: Some(AutoOff::NEVER), ..Status::default() };
        let text = status_text(&info(&CLOUD_ALPHA), &status);
        assert!(text.contains("Battery      ?, ?"));
        assert!(text.contains("Auto-off     never"));
    }

    #[test]
    fn status_text_shows_only_what_the_headset_has() {
        let status = Status {
            connected: Some(true),
            battery: Some(Percent::saturating(42)),
            charge: Some(ChargeState::Charging),
            lights: Some(true),
            chatmix: Some(64),
            noise_filter: Some(NoiseFilter::Low),
            equalizer_preset: Some(1),
            equalizer: Some(vec![Gain::tenths(25), Gain::tenths(-10)]),
            ..Status::default()
        };
        let capabilities = [
            Capability::Battery,
            Capability::Lights,
            Capability::ChatMix,
            Capability::NoiseFilter,
            Capability::EqualizerPreset,
            Capability::Equalizer,
        ];
        assert_eq!(
            status_text(&info(&capabilities), &status),
            "Headset      Cloud Alpha Wireless, connected\n\
             Battery      42%, charging\n\
             Lights       on\n\
             ChatMix      64 (even)\n\
             Noise filter low\n\
             EQ preset    bass boost\n\
             EQ           2.5 dB, -1 dB\n"
        );
    }

    #[test]
    fn status_text_for_a_headset_that_is_off() {
        let status = Status { connected: Some(false), ..Status::default() };
        assert_eq!(
            status_text(&info(&CLOUD_ALPHA), &status),
            "Headset      Cloud Alpha Wireless: off (dongle plugged in)\n"
        );
    }

    #[test]
    fn status_json_uses_plain_values_and_null() {
        let value = status_json(&info(&CLOUD_ALPHA), &full());
        assert_eq!(value["battery"], 99);
        assert_eq!(value["sidetone_level"], 0);
        assert_eq!(value["auto_off"], 20);
        assert_eq!(value["charging"], "discharging");
        assert_eq!(value["usb_id"], "03f0:098d");
        assert_eq!(value["backend"], "demo");
        assert_eq!(value["lights"], Value::Null);
        let eq = Status { equalizer: Some(vec![Gain::tenths(25)]), ..Status::default() };
        assert_eq!(status_json(&info(&[]), &eq)["equalizer"], json!([2.5]));
    }

    #[test]
    fn chatmix_reads_as_a_balance() {
        assert_eq!(chatmix_text(64), "64 (even)");
        assert_eq!(chatmix_text(0), "0 (more game)");
        assert_eq!(chatmix_text(128), "128 (more chat)");
    }

    #[test]
    fn presets_are_named_when_the_headset_names_them() {
        assert_eq!(preset_name(&info(&[]), 1), "bass boost");
        assert_eq!(preset_name(&info(&[]), 7), "7");
    }

    #[test]
    fn waybar_hides_without_a_headset_and_shows_states() {
        assert_eq!(waybar_json(None), json!({ "text": "" }));
        let off = Status { connected: Some(false), ..Status::default() };
        assert_eq!(waybar_json(Some(&off))["class"], "off");
        let mut status = full();
        assert_eq!(
            waybar_json(Some(&status)),
            json!({
                "text": "99%", "tooltip": "Headset battery: 99%, discharging", "class": "ok", "percentage": 99
            })
        );
        status.battery = Some(Percent::saturating(10));
        assert_eq!(waybar_json(Some(&status))["class"], "low");
        status.charge = Some(ChargeState::Charging);
        assert_eq!(waybar_json(Some(&status))["class"], "charging");
    }

    #[test]
    fn monitor_lines() {
        assert_eq!(event_line(&Event::Battery(Percent::saturating(99))), "battery = 99%");
        assert_eq!(event_line(&Event::AutoOff(AutoOff::minutes(20))), "auto_off = 20 min");
        assert_eq!(event_line(&Event::Charge(ChargeState::Full)), "charge = full");
        assert_eq!(event_line(&Event::ChatMix(30)), "chatmix = 30 (more game)");
        assert_eq!(event_line(&Event::Equalizer(vec![Gain::tenths(-5)])), "eq = -0.5 dB");
        assert_eq!(event_line(&Event::Raw(vec![0x21, 0xBB, 0x0B])), "raw  21 bb 0b");
    }
}
