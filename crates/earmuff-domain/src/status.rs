//! What a headset reports and what can be asked of it.

use crate::device::Capability;
use crate::values::{AutoOff, ChargeState, Percent};

/// What to ask a headset for
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Query {
    /// Whether the headset is on and in range of its dongle
    Connection,
    /// Battery level and charging state
    Battery,
    /// Everything the headset can report
    All,
}

/// A change the user asks for
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Setting {
    MicMuted(bool),
    Sidetone(bool),
    SidetoneLevel(Percent),
    AutoOff(AutoOff),
    VoicePrompts(bool),
}

impl Setting {
    /// What a headset must support to accept this setting
    pub fn capability(&self) -> Capability {
        match self {
            Setting::MicMuted(_) => Capability::MicMute,
            Setting::Sidetone(_) => Capability::Sidetone,
            Setting::SidetoneLevel(_) => Capability::SidetoneLevel,
            Setting::AutoOff(_) => Capability::AutoOff,
            Setting::VoicePrompts(_) => Capability::VoicePrompts,
        }
    }
}

/// News from a headset: an answer to a query, or something that changed on its own
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Whether the headset is on and in range of its dongle
    Connected(bool),
    Battery(Percent),
    Charge(ChargeState),
    MicMuted(bool),
    MicAttached(bool),
    Sidetone(bool),
    SidetoneLevel(Percent),
    AutoOff(AutoOff),
    VoicePrompts(bool),
    /// The headset's color variant, e.g. "Black/Red"
    Color(String),
    /// A report as the backend received it, for diagnostics
    Raw(Vec<u8>),
}

/// Everything known about a headset. A field stays `None` until the headset reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub connected: Option<bool>,
    pub battery: Option<Percent>,
    pub charge: Option<ChargeState>,
    pub mic_muted: Option<bool>,
    pub mic_attached: Option<bool>,
    pub sidetone: Option<bool>,
    pub sidetone_level: Option<Percent>,
    pub auto_off: Option<AutoOff>,
    pub voice_prompts: Option<bool>,
    pub color: Option<String>,
}

impl Status {
    /// Records an event; returns true if it changed anything
    pub fn apply(&mut self, event: &Event) -> bool {
        fn set<T: PartialEq + Clone>(field: &mut Option<T>, value: &T) -> bool {
            let changed = field.as_ref() != Some(value);
            *field = Some(value.clone());
            changed
        }
        match event {
            Event::Connected(v) => set(&mut self.connected, v),
            Event::Battery(v) => set(&mut self.battery, v),
            Event::Charge(v) => set(&mut self.charge, v),
            Event::MicMuted(v) => set(&mut self.mic_muted, v),
            Event::MicAttached(v) => set(&mut self.mic_attached, v),
            Event::Sidetone(v) => set(&mut self.sidetone, v),
            Event::SidetoneLevel(v) => set(&mut self.sidetone_level, v),
            Event::AutoOff(v) => set(&mut self.auto_off, v),
            Event::VoicePrompts(v) => set(&mut self.voice_prompts, v),
            Event::Color(v) => set(&mut self.color, v),
            Event::Raw(_) => false,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected == Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_reports_only_real_changes() {
        let mut status = Status::default();
        assert!(status.apply(&Event::Battery(Percent::saturating(99))));
        assert!(!status.apply(&Event::Battery(Percent::saturating(99))));
        assert!(status.apply(&Event::Battery(Percent::saturating(98))));
        assert_eq!(status.battery, Some(Percent::saturating(98)));
    }

    #[test]
    fn apply_covers_every_field() {
        let mut status = Status::default();
        let events = [
            Event::Connected(true),
            Event::Battery(Percent::saturating(50)),
            Event::Charge(ChargeState::Charging),
            Event::MicMuted(true),
            Event::MicAttached(true),
            Event::Sidetone(true),
            Event::SidetoneLevel(Percent::saturating(30)),
            Event::AutoOff(AutoOff::minutes(20)),
            Event::VoicePrompts(false),
            Event::Color("Black/Red".into()),
        ];
        for event in &events {
            assert!(status.apply(event), "{event:?}");
        }
        assert_eq!(
            status,
            Status {
                connected: Some(true),
                battery: Some(Percent::saturating(50)),
                charge: Some(ChargeState::Charging),
                mic_muted: Some(true),
                mic_attached: Some(true),
                sidetone: Some(true),
                sidetone_level: Some(Percent::saturating(30)),
                auto_off: Some(AutoOff::minutes(20)),
                voice_prompts: Some(false),
                color: Some("Black/Red".into()),
            }
        );
    }

    #[test]
    fn raw_reports_change_nothing() {
        let mut status = Status::default();
        assert!(!status.apply(&Event::Raw(vec![0x21, 0xBB])));
        assert_eq!(status, Status::default());
    }

    #[test]
    fn connected_only_when_the_headset_said_so() {
        let mut status = Status::default();
        assert!(!status.is_connected());
        status.apply(&Event::Connected(false));
        assert!(!status.is_connected());
        status.apply(&Event::Connected(true));
        assert!(status.is_connected());
    }

    #[test]
    fn every_setting_names_its_capability() {
        assert_eq!(Setting::MicMuted(true).capability(), Capability::MicMute);
        assert_eq!(Setting::Sidetone(true).capability(), Capability::Sidetone);
        assert_eq!(Setting::SidetoneLevel(Percent::ZERO).capability(), Capability::SidetoneLevel);
        assert_eq!(Setting::AutoOff(AutoOff::NEVER).capability(), Capability::AutoOff);
        assert_eq!(Setting::VoicePrompts(false).capability(), Capability::VoicePrompts);
    }
}
