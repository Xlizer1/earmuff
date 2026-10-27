//! How the command line prints things. Pure functions, so they are unit-tested.

use earmuff_domain::{DeviceInfo, Event, Status};
use serde_json::{Value, json};

fn known<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "?".into(), |v| v.to_string())
}

fn yes_no(value: Option<bool>, yes: &str, no: &str) -> String {
    known(value.map(|v| if v { yes } else { no }))
}

/// The `status` command's text: one line per topic
pub fn status_text(info: &DeviceInfo, s: &Status) -> String {
    if !s.is_connected() {
        return format!("Headset      {}: off (dongle plugged in)\n", info.name);
    }
    let auto_off = s.auto_off.map(|a| if a.is_never() { "never".into() } else { format!("{} min", a.get()) });
    format!(
        "Headset      {}, connected ({})\n\
         Battery      {}, {}\n\
         Mic          {}, {}\n\
         Monitoring   {}, level {}\n\
         Auto-off     {}\n\
         Voice        {}\n",
        info.name,
        known(s.color.as_ref()),
        known(s.battery),
        known(s.charge),
        yes_no(s.mic_attached, "attached", "detached"),
        yes_no(s.mic_muted, "muted", "not muted"),
        yes_no(s.sidetone, "on", "off"),
        known(s.sidetone_level),
        known(auto_off),
        yes_no(s.voice_prompts, "on", "off"),
    )
}

/// The `status --json` output. Unknown values are null.
pub fn status_json(info: &DeviceInfo, s: &Status) -> Value {
    json!({
        "device": info.name,
        "usb_id": info.id.usb.to_string(),
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
        Event::Raw(report) => {
            let hex: Vec<String> = report.iter().map(|b| format!("{b:02x}")).collect();
            format!("raw  {}", hex.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{AutoOff, BackendKind, ChargeState, DeviceId, Percent, UsbId};

    fn info() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(0x03F0, 0x098D), instance: "demo-1".into() },
            name: "Cloud Alpha Wireless".into(),
            vendor: "HyperX".into(),
            capabilities: vec![],
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
        }
    }

    #[test]
    fn status_text_lists_every_setting() {
        assert_eq!(
            status_text(&info(), &full()),
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
        let text = status_text(&info(), &status);
        assert!(text.contains("Battery      ?, ?"));
        assert!(text.contains("Auto-off     never"));
    }

    #[test]
    fn status_text_for_a_headset_that_is_off() {
        let status = Status { connected: Some(false), ..Status::default() };
        assert_eq!(status_text(&info(), &status), "Headset      Cloud Alpha Wireless: off (dongle plugged in)\n");
    }

    #[test]
    fn status_json_uses_plain_values_and_null() {
        let value = status_json(&info(), &full());
        assert_eq!(value["battery"], 99);
        assert_eq!(value["sidetone_level"], 0);
        assert_eq!(value["auto_off"], 20);
        assert_eq!(value["charging"], "discharging");
        assert_eq!(value["usb_id"], "03f0:098d");
        assert_eq!(status_json(&info(), &Status::default())["battery"], Value::Null);
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
        assert_eq!(event_line(&Event::Raw(vec![0x21, 0xBB, 0x0B])), "raw  21 bb 0b");
    }
}
