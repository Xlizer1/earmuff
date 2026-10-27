//! USB protocol of the HyperX Cloud Alpha Wireless.
//!
//! Every message is a 31-byte HID report on the dongle's vendor interface:
//! report ID 0x21, then 0xBB, a command byte and its value. The headset answers
//! each GET with the same command byte, and sends the `EVT_*` reports on its own
//! when something changes (mute button, battery, power).
//!
//! Command bytes come from HeadsetControl and HyperHeadset (both MIT licensed).
//! The sidetone level encoding and the value ranges were measured on the headset.

use std::fmt;

pub const VENDOR_ID: u16 = 0x03F0;
pub const PRODUCT_ID: u16 = 0x098D;
pub const REPORT_ID: u8 = 0x21;
pub const MAGIC: u8 = 0xBB;
/// Report ID + 30 bytes, from the dongle's HID report descriptor
pub const REPORT_LEN: usize = 31;

/// Signed 16-bit value in bytes 3-4; the headset clamps anything outside this range
pub const SIDETONE_MIN: i16 = -4;
pub const SIDETONE_MAX: i16 = 18;

pub type Report = [u8; REPORT_LEN];

pub mod cmd {
    pub const GET_WIRELESS: u8 = 0x03; // 1 = headset off, 2 = connected
    #[allow(dead_code)] // answers with a pairing ID; not shown anywhere yet
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
pub const GET_ALL: [u8; 10] = [
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChargeState {
    Discharging,
    Charging,
    Full,
    Unknown(u8),
}

impl fmt::Display for ChargeState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ChargeState::Discharging => f.write_str("discharging"),
            ChargeState::Charging => f.write_str("charging"),
            ChargeState::Full => f.write_str("full"),
            ChargeState::Unknown(v) => write!(f, "unknown ({v})"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    Black,
    White,
    BlackRed,
    Unknown(u8),
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Color::Black => f.write_str("Black"),
            Color::White => f.write_str("White"),
            Color::BlackRed => f.write_str("Black/Red"),
            Color::Unknown(v) => write!(f, "unknown ({v})"),
        }
    }
}

/// One piece of news from the headset
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Update {
    Connected(bool),
    Battery(u8),
    Charging(ChargeState),
    Muted(bool),
    MicAttached(bool),
    Sidetone(bool),
    SidetoneLevel(i16),
    AutoOff(u8),
    VoicePrompts(bool),
    Color(Color),
}

impl fmt::Display for Update {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Update::Connected(v) => write!(f, "connected = {v}"),
            Update::Battery(v) => write!(f, "battery = {v}"),
            Update::Charging(v) => write!(f, "charging = {v}"),
            Update::Muted(v) => write!(f, "muted = {v}"),
            Update::MicAttached(v) => write!(f, "mic_attached = {v}"),
            Update::Sidetone(v) => write!(f, "sidetone = {v}"),
            Update::SidetoneLevel(v) => write!(f, "sidetone_level = {v}"),
            Update::AutoOff(v) => write!(f, "auto_off = {v}"),
            Update::VoicePrompts(v) => write!(f, "voice_prompts = {v}"),
            Update::Color(v) => write!(f, "color = {v}"),
        }
    }
}

/// The last value the headset reported for each setting; None until it has answered
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub connected: Option<bool>,
    pub battery: Option<u8>,
    pub charging: Option<ChargeState>,
    pub muted: Option<bool>,
    pub mic_attached: Option<bool>,
    pub sidetone: Option<bool>,
    pub sidetone_level: Option<i16>,
    pub auto_off: Option<u8>,
    pub voice_prompts: Option<bool>,
    pub color: Option<Color>,
}

impl State {
    /// Stores an update; returns true if it changed anything
    pub fn apply(&mut self, update: Update) -> bool {
        fn set<T: PartialEq>(field: &mut Option<T>, value: T) -> bool {
            let changed = field.as_ref() != Some(&value);
            *field = Some(value);
            changed
        }
        match update {
            Update::Connected(v) => set(&mut self.connected, v),
            Update::Battery(v) => set(&mut self.battery, v),
            Update::Charging(v) => set(&mut self.charging, v),
            Update::Muted(v) => set(&mut self.muted, v),
            Update::MicAttached(v) => set(&mut self.mic_attached, v),
            Update::Sidetone(v) => set(&mut self.sidetone, v),
            Update::SidetoneLevel(v) => set(&mut self.sidetone_level, v),
            Update::AutoOff(v) => set(&mut self.auto_off, v),
            Update::VoicePrompts(v) => set(&mut self.voice_prompts, v),
            Update::Color(v) => set(&mut self.color, v),
        }
    }
}

pub fn packet(cmd: u8, value: &[u8]) -> Report {
    let mut report = [0; REPORT_LEN];
    report[..3].copy_from_slice(&[REPORT_ID, MAGIC, cmd]);
    report[3..3 + value.len()].copy_from_slice(value);
    report
}

pub fn get(cmd: u8) -> Report {
    packet(cmd, &[])
}

pub fn flag(cmd: u8, on: bool) -> Report {
    packet(cmd, &[on as u8])
}

pub fn sidetone_level(level: i16) -> Report {
    let level = level.clamp(SIDETONE_MIN, SIDETONE_MAX);
    packet(cmd::SET_SIDETONE_LEVEL, &level.to_be_bytes())
}

pub fn auto_off(minutes: u8) -> Report {
    packet(cmd::SET_AUTO_OFF, &[minutes])
}

/// The command byte of the headset's answer to a report we sent
pub fn reply_code(report: &Report) -> u8 {
    match report[2] {
        cmd::SET_MUTE => cmd::EVT_MUTE,
        code => code,
    }
}

/// Turns a report from the headset into an update.
///
/// Answers to SET commands are ignored: they echo the value that was sent,
/// not the value the headset kept, so every SET is followed by its GET.
pub fn parse(report: &[u8]) -> Option<Update> {
    if report.len() < 5 || report[0] != REPORT_ID || report[1] != MAGIC {
        return None;
    }
    let value = report[3];
    Some(match report[2] {
        cmd::GET_WIRELESS | cmd::EVT_WIRELESS => Update::Connected(value == 2),
        cmd::GET_BATTERY | cmd::EVT_BATTERY => Update::Battery(value),
        cmd::GET_CHARGING | cmd::EVT_CHARGING => Update::Charging(match value {
            0 => ChargeState::Discharging,
            1 => ChargeState::Charging,
            2 => ChargeState::Full,
            v => ChargeState::Unknown(v),
        }),
        cmd::GET_MUTE | cmd::EVT_MUTE => Update::Muted(value == 1),
        cmd::GET_MIC_ATTACHED => Update::MicAttached(value == 1),
        cmd::GET_SIDETONE | cmd::EVT_SIDETONE => Update::Sidetone(value == 1),
        cmd::GET_SIDETONE_LEVEL => Update::SidetoneLevel(i16::from_be_bytes([report[3], report[4]])),
        cmd::GET_AUTO_OFF => Update::AutoOff(value),
        cmd::GET_VOICE_PROMPTS => Update::VoicePrompts(value == 1),
        cmd::GET_COLOR => Update::Color(match value {
            0 => Color::Black,
            1 => Color::White,
            2 => Color::BlackRed,
            v => Color::Unknown(v),
        }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    //! The byte strings are real replies captured from the headset.
    use super::*;

    fn reply(bytes: &[u8]) -> Report {
        let mut report = [0; REPORT_LEN];
        report[..bytes.len()].copy_from_slice(bytes);
        report
    }

    #[test]
    fn get_packet() {
        let report = get(cmd::GET_BATTERY);
        assert_eq!(report.len(), REPORT_LEN);
        assert_eq!(report[..4], [0x21, 0xBB, 0x0B, 0x00]);
    }

    #[test]
    fn sidetone_level_is_signed_16_bit() {
        assert_eq!(sidetone_level(10)[2..5], [0x11, 0x00, 0x0A]);
        assert_eq!(sidetone_level(-4)[2..5], [0x11, 0xFF, 0xFC]);
    }

    #[test]
    fn sidetone_level_is_clamped() {
        assert_eq!(sidetone_level(100), sidetone_level(SIDETONE_MAX));
        assert_eq!(sidetone_level(-100), sidetone_level(SIDETONE_MIN));
    }

    #[test]
    fn mute_is_answered_by_mute_event() {
        assert_eq!(reply_code(&flag(cmd::SET_MUTE, true)), cmd::EVT_MUTE);
        assert_eq!(reply_code(&get(cmd::GET_BATTERY)), cmd::GET_BATTERY);
    }

    #[test]
    fn replies() {
        let cases: &[(&[u8], Update)] = &[
            (&[0x21, 0xBB, 0x03, 0x02], Update::Connected(true)),
            (&[0x21, 0xBB, 0x03, 0x01], Update::Connected(false)),
            (&[0x21, 0xBB, 0x0B, 0x63, 0x10, 0x3A, 0x01], Update::Battery(99)),
            (&[0x21, 0xBB, 0x0C, 0x00], Update::Charging(ChargeState::Discharging)),
            (&[0x21, 0xBB, 0x05, 0x00], Update::Sidetone(false)),
            (&[0x21, 0xBB, 0x06, 0xFF, 0xFC], Update::SidetoneLevel(-4)),
            (&[0x21, 0xBB, 0x06, 0x00, 0x12], Update::SidetoneLevel(18)),
            (&[0x21, 0xBB, 0x07, 0x14], Update::AutoOff(20)),
            (&[0x21, 0xBB, 0x08, 0x01], Update::MicAttached(true)),
            (&[0x21, 0xBB, 0x09, 0x01], Update::VoicePrompts(true)),
            (&[0x21, 0xBB, 0x0A, 0x00], Update::Muted(false)),
            (&[0x21, 0xBB, 0x23, 0x01], Update::Muted(true)),
            (&[0x21, 0xBB, 0x0E, 0x02], Update::Color(Color::BlackRed)),
        ];
        for (bytes, expected) in cases {
            assert_eq!(parse(&reply(bytes)), Some(*expected), "{bytes:02x?}");
        }
    }

    #[test]
    fn set_echo_is_ignored() {
        // the headset echoes 40, but keeps 18
        assert_eq!(parse(&reply(&[0x21, 0xBB, 0x11, 0x00, 0x28, 0x01])), None);
    }

    #[test]
    fn other_reports_are_ignored() {
        assert_eq!(parse(&[0x01, 0x04]), None); // volume wheel (consumer control report)
        assert_eq!(parse(&reply(&[0x21, 0xBB, 0x04, 0xC8, 0x5A])), None); // pairing info: not used
    }

    #[test]
    fn apply_reports_changes() {
        let mut state = State::default();
        assert!(state.apply(Update::Battery(99)));
        assert!(!state.apply(Update::Battery(99)));
        assert!(state.apply(Update::Battery(98)));
        assert_eq!(state.battery, Some(98));
    }
}
