//! HyperX Cloud Alpha Wireless, USB dongle 03f0:098d.
//!
//! Every message is a 31-byte HID report on the dongle's vendor interface:
//! report ID 0x21, then 0xBB, a command byte and its value. The headset answers
//! each GET with the same command byte, and sends the `EVT_*` reports on its own
//! when something changes (mute button, battery, power). See
//! docs/protocols/hyperx-cloud-alpha-wireless.md.
//!
//! Command bytes come from HeadsetControl (GPL-3.0) and HyperHeadset (MIT).
//! The sidetone level encoding and the value ranges were measured on the headset.

use earmuff_domain::{AutoOff, Capability, ChargeState, Event, Percent, Query, Setting, UsbId};

use crate::codec::{Codec, Exchange, Model};

pub const USB_ID: UsbId = UsbId::new(0x03F0, 0x098D);
pub const REPORT_ID: u8 = 0x21;
pub const MAGIC: u8 = 0xBB;
/// Report ID + 30 bytes, from the dongle's HID report descriptor
pub const REPORT_LEN: usize = 31;

/// Mic monitoring level: a signed 16-bit value in bytes 3-4, clamped by the headset to this range
pub const SIDETONE_MIN: i32 = -4;
pub const SIDETONE_MAX: i32 = 18;

pub mod cmd {
    pub const GET_WIRELESS: u8 = 0x03; // 1 = headset off, 2 = connected
    pub const GET_PAIRING: u8 = 0x04;
    pub const GET_SIDETONE: u8 = 0x05;
    pub const GET_SIDETONE_LEVEL: u8 = 0x06;
    pub const GET_AUTO_OFF: u8 = 0x07; // minutes
    pub const GET_MIC_ATTACHED: u8 = 0x08;
    pub const GET_VOICE_PROMPTS: u8 = 0x09;
    pub const GET_MUTE: u8 = 0x0A;
    pub const GET_BATTERY: u8 = 0x0B; // percent
    pub const GET_CHARGING: u8 = 0x0C;
    pub const GET_COLOR: u8 = 0x0E;

    pub const SET_SIDETONE: u8 = 0x10;
    pub const SET_SIDETONE_LEVEL: u8 = 0x11;
    pub const SET_AUTO_OFF: u8 = 0x12;
    pub const SET_VOICE_PROMPTS: u8 = 0x13;
    pub const SET_MUTE: u8 = 0x15;

    pub const EVT_SIDETONE: u8 = 0x22;
    pub const EVT_MUTE: u8 = 0x23;
    pub const EVT_WIRELESS: u8 = 0x24;
    pub const EVT_BATTERY: u8 = 0x25;
    pub const EVT_CHARGING: u8 = 0x26;
}

/// Wireless first: when the headset is off, the other answers mean nothing
const QUERY_ALL: [u8; 10] = [
    cmd::GET_WIRELESS,
    cmd::GET_BATTERY,
    cmd::GET_CHARGING,
    cmd::GET_MUTE,
    cmd::GET_MIC_ATTACHED,
    cmd::GET_SIDETONE,
    cmd::GET_SIDETONE_LEVEL,
    cmd::GET_AUTO_OFF,
    cmd::GET_VOICE_PROMPTS,
    cmd::GET_COLOR,
];

static MODEL: Model = Model {
    name: "Cloud Alpha Wireless",
    vendor: "HyperX",
    ids: &[USB_ID],
    capabilities: &[
        Capability::Battery,
        Capability::MicMute,
        Capability::MicAttached,
        Capability::Sidetone,
        Capability::SidetoneLevel,
        Capability::AutoOff,
        Capability::VoicePrompts,
    ],
};

pub fn packet(cmd: u8, value: &[u8]) -> Vec<u8> {
    let mut report = vec![0; REPORT_LEN];
    report[..3].copy_from_slice(&[REPORT_ID, MAGIC, cmd]);
    report[3..3 + value.len()].copy_from_slice(value);
    report
}

fn get(cmd: u8) -> Exchange {
    Exchange { report: packet(cmd, &[]), reply_prefix: Some(vec![REPORT_ID, MAGIC, cmd]) }
}

fn set(cmd: u8, value: &[u8]) -> Exchange {
    // a mute change is answered with the mute event; every other SET echoes its command
    let reply = if cmd == cmd::SET_MUTE { cmd::EVT_MUTE } else { cmd };
    Exchange { report: packet(cmd, value), reply_prefix: Some(vec![REPORT_ID, MAGIC, reply]) }
}

pub fn color_name(value: u8) -> String {
    match value {
        0 => "Black".into(),
        1 => "White".into(),
        2 => "Black/Red".into(),
        v => format!("Unknown ({v})"),
    }
}

/// The codec for the HyperX Cloud Alpha Wireless
#[derive(Debug)]
pub struct CloudAlphaWireless;

impl Codec for CloudAlphaWireless {
    fn model(&self) -> &Model {
        &MODEL
    }

    fn query(&self, query: Query) -> Vec<Exchange> {
        match query {
            Query::Connection => vec![get(cmd::GET_WIRELESS)],
            Query::Battery => vec![get(cmd::GET_BATTERY), get(cmd::GET_CHARGING)],
            Query::Mute => vec![get(cmd::GET_MUTE)],
            Query::All => QUERY_ALL.map(get).to_vec(),
        }
    }

    fn apply(&self, setting: Setting) -> Option<Vec<Exchange>> {
        // every SET is followed by its GET: the echo carries the value sent, not the value kept
        Some(match setting {
            Setting::MicMuted(on) => vec![set(cmd::SET_MUTE, &[u8::from(on)])],
            Setting::Sidetone(on) => vec![set(cmd::SET_SIDETONE, &[u8::from(on)]), get(cmd::GET_SIDETONE)],
            Setting::SidetoneLevel(level) => {
                let raw = level.to_range(SIDETONE_MIN, SIDETONE_MAX) as i16;
                vec![set(cmd::SET_SIDETONE_LEVEL, &raw.to_be_bytes()), get(cmd::GET_SIDETONE_LEVEL)]
            }
            Setting::AutoOff(minutes) => vec![set(cmd::SET_AUTO_OFF, &[minutes.get()]), get(cmd::GET_AUTO_OFF)],
            Setting::VoicePrompts(on) => {
                vec![set(cmd::SET_VOICE_PROMPTS, &[u8::from(on)]), get(cmd::GET_VOICE_PROMPTS)]
            }
            _ => return None,
        })
    }

    fn decode(&self, report: &[u8]) -> Option<Event> {
        if report.len() < 5 || report[0] != REPORT_ID || report[1] != MAGIC {
            return None;
        }
        let value = report[3];
        Some(match report[2] {
            cmd::GET_WIRELESS | cmd::EVT_WIRELESS => Event::Connected(value == 2),
            cmd::GET_BATTERY | cmd::EVT_BATTERY => Event::Battery(Percent::saturating(i64::from(value))),
            cmd::GET_CHARGING | cmd::EVT_CHARGING => Event::Charge(match value {
                0 => ChargeState::Discharging,
                1 => ChargeState::Charging,
                2 => ChargeState::Full,
                _ => ChargeState::Unknown,
            }),
            cmd::GET_MUTE | cmd::EVT_MUTE => Event::MicMuted(value == 1),
            cmd::GET_MIC_ATTACHED => Event::MicAttached(value == 1),
            cmd::GET_SIDETONE | cmd::EVT_SIDETONE => Event::Sidetone(value == 1),
            cmd::GET_SIDETONE_LEVEL => {
                let raw = i16::from_be_bytes([report[3], report[4]]);
                Event::SidetoneLevel(Percent::from_range(raw.into(), SIDETONE_MIN, SIDETONE_MAX))
            }
            cmd::GET_AUTO_OFF => Event::AutoOff(AutoOff::minutes(value)),
            cmd::GET_VOICE_PROMPTS => Event::VoicePrompts(value == 1),
            cmd::GET_COLOR => Event::Color(color_name(value)),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    //! The byte strings are real replies captured from the headset.
    use super::*;

    fn reply(bytes: &[u8]) -> Vec<u8> {
        let mut report = vec![0; REPORT_LEN];
        report[..bytes.len()].copy_from_slice(bytes);
        report
    }

    fn percent(value: u8) -> Percent {
        Percent::new(value).unwrap()
    }

    #[test]
    fn get_packet_is_a_full_report() {
        let report = packet(cmd::GET_BATTERY, &[]);
        assert_eq!(report.len(), REPORT_LEN);
        assert_eq!(report[..4], [0x21, 0xBB, 0x0B, 0x00]);
    }

    #[test]
    fn query_all_asks_whether_the_headset_is_on_first() {
        let reports = CloudAlphaWireless.query(Query::All);
        assert_eq!(reports.len(), 10);
        assert_eq!(reports[0].report[2], cmd::GET_WIRELESS);
        assert!(reports.iter().all(|e| e.reply_prefix.as_deref() == Some(&e.report[..3])));
    }

    #[test]
    fn query_battery_asks_level_and_charging() {
        let mute: Vec<u8> = CloudAlphaWireless.query(Query::Mute).iter().map(|e| e.report[2]).collect();
        assert_eq!(mute, [cmd::GET_MUTE]);
        let cmds: Vec<u8> = CloudAlphaWireless.query(Query::Battery).iter().map(|e| e.report[2]).collect();
        assert_eq!(cmds, [cmd::GET_BATTERY, cmd::GET_CHARGING]);
    }

    #[test]
    fn sidetone_level_is_signed_16_bit() {
        let level = |p: u8| CloudAlphaWireless.apply(Setting::SidetoneLevel(percent(p))).unwrap();
        assert_eq!(level(0)[0].report[2..5], [0x11, 0xFF, 0xFC]); // -4
        assert_eq!(level(50)[0].report[2..5], [0x11, 0x00, 0x07]);
        assert_eq!(level(100)[0].report[2..5], [0x11, 0x00, 0x12]); // 18
        assert_eq!(level(50)[1].report[2], cmd::GET_SIDETONE_LEVEL);
    }

    #[test]
    fn every_set_is_followed_by_its_get() {
        let pairs = [
            (Setting::Sidetone(true), cmd::SET_SIDETONE, cmd::GET_SIDETONE),
            (Setting::AutoOff(AutoOff::minutes(30)), cmd::SET_AUTO_OFF, cmd::GET_AUTO_OFF),
            (Setting::VoicePrompts(false), cmd::SET_VOICE_PROMPTS, cmd::GET_VOICE_PROMPTS),
        ];
        for (setting, set_cmd, get_cmd) in pairs {
            let exchanges = CloudAlphaWireless.apply(setting.clone()).unwrap();
            assert_eq!(exchanges.iter().map(|e| e.report[2]).collect::<Vec<_>>(), [set_cmd, get_cmd], "{setting:?}");
        }
        assert_eq!(CloudAlphaWireless.apply(Setting::AutoOff(AutoOff::minutes(30))).unwrap()[0].report[3], 30);
    }

    #[test]
    fn mute_is_answered_by_the_mute_event() {
        let exchanges = CloudAlphaWireless.apply(Setting::MicMuted(true)).unwrap();
        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0].report[2..4], [cmd::SET_MUTE, 1]);
        assert!(exchanges[0].answered_by(&reply(&[0x21, 0xBB, 0x23, 0x01])));
        assert!(!exchanges[0].answered_by(&reply(&[0x21, 0xBB, 0x15, 0x01])));
    }

    #[test]
    fn decodes_captured_replies() {
        let cases: &[(&[u8], Event)] = &[
            (&[0x21, 0xBB, 0x03, 0x02], Event::Connected(true)),
            (&[0x21, 0xBB, 0x03, 0x01], Event::Connected(false)),
            (&[0x21, 0xBB, 0x24, 0x01], Event::Connected(false)),
            (&[0x21, 0xBB, 0x0B, 0x63, 0x10, 0x3A, 0x01], Event::Battery(percent(99))),
            (&[0x21, 0xBB, 0x25, 0x50], Event::Battery(percent(80))),
            (&[0x21, 0xBB, 0x0C, 0x00], Event::Charge(ChargeState::Discharging)),
            (&[0x21, 0xBB, 0x26, 0x01], Event::Charge(ChargeState::Charging)),
            (&[0x21, 0xBB, 0x0C, 0x02], Event::Charge(ChargeState::Full)),
            (&[0x21, 0xBB, 0x0C, 0x09], Event::Charge(ChargeState::Unknown)),
            (&[0x21, 0xBB, 0x05, 0x00], Event::Sidetone(false)),
            (&[0x21, 0xBB, 0x06, 0xFF, 0xFC], Event::SidetoneLevel(percent(0))),
            (&[0x21, 0xBB, 0x06, 0x00, 0x12], Event::SidetoneLevel(percent(100))),
            (&[0x21, 0xBB, 0x07, 0x14], Event::AutoOff(AutoOff::minutes(20))),
            (&[0x21, 0xBB, 0x08, 0x01], Event::MicAttached(true)),
            (&[0x21, 0xBB, 0x09, 0x01], Event::VoicePrompts(true)),
            (&[0x21, 0xBB, 0x0A, 0x00], Event::MicMuted(false)),
            (&[0x21, 0xBB, 0x23, 0x01], Event::MicMuted(true)),
            (&[0x21, 0xBB, 0x0E, 0x02], Event::Color("Black/Red".into())),
        ];
        for (bytes, expected) in cases {
            assert_eq!(CloudAlphaWireless.decode(&reply(bytes)), Some(expected.clone()), "{bytes:02x?}");
        }
    }

    #[test]
    fn set_echoes_are_ignored() {
        // the headset echoes 40, but keeps 18; the GET that follows reports what it kept
        assert_eq!(CloudAlphaWireless.decode(&reply(&[0x21, 0xBB, 0x11, 0x00, 0x28, 0x01])), None);
        assert_eq!(CloudAlphaWireless.decode(&reply(&[0x21, 0xBB, 0x12, 0x0A, 0x01])), None);
    }

    #[test]
    fn other_reports_are_ignored() {
        assert_eq!(CloudAlphaWireless.decode(&[0x01, 0x04]), None); // volume wheel (consumer control report)
        assert_eq!(CloudAlphaWireless.decode(&reply(&[0x21, 0xBB, 0x04, 0xC8, 0x5A])), None); // pairing info
        assert_eq!(CloudAlphaWireless.decode(&reply(&[0x21, 0xAA, 0x0B, 0x63])), None);
    }

    #[test]
    fn color_names() {
        assert_eq!(color_name(0), "Black");
        assert_eq!(color_name(1), "White");
        assert_eq!(color_name(7), "Unknown (7)");
    }

    #[test]
    fn model_supports_everything_it_decodes() {
        let model = CloudAlphaWireless.model();
        assert_eq!(model.ids, [USB_ID]);
        for capability in [Capability::Battery, Capability::MicMute, Capability::Sidetone, Capability::AutoOff] {
            assert!(model.capabilities.contains(&capability));
        }
    }
}
