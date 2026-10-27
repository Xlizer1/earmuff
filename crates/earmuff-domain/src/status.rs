//! What a headset reports and what can be asked of it.

use crate::device::Capability;
use crate::equalizer::Gain;
use crate::values::{AutoOff, ChargeState, NoiseFilter, Percent};

/// What to ask a headset for
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Query {
    /// Whether the headset is on and in range of its dongle
    Connection,
    /// Battery level and charging state
    Battery,
    /// Whether the mic is muted
    Mute,
    /// Everything the headset can report
    All,
}

/// A change the user asks for
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Setting {
    MicMuted(bool),
    Sidetone(bool),
    SidetoneLevel(Percent),
    AutoOff(AutoOff),
    VoicePrompts(bool),
    Lights(bool),
    RotateToMute(bool),
    MicVolume(Percent),
    /// 0 (off) to 3
    MicMuteLedBrightness(u8),
    NoiseFilter(NoiseFilter),
    VolumeLimiter(bool),
    BtWhenPoweredOn(bool),
    BtCallVolume(Percent),
    /// Index into the equalizer's presets
    EqualizerPreset(u8),
    Equalizer(Vec<Gain>),
    /// Plays one of the headset's notification sounds (not a lasting setting)
    PlayNotificationSound(u8),
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
            Setting::Lights(_) => Capability::Lights,
            Setting::RotateToMute(_) => Capability::RotateToMute,
            Setting::MicVolume(_) => Capability::MicVolume,
            Setting::MicMuteLedBrightness(_) => Capability::MicMuteLedBrightness,
            Setting::NoiseFilter(_) => Capability::NoiseFilter,
            Setting::VolumeLimiter(_) => Capability::VolumeLimiter,
            Setting::BtWhenPoweredOn(_) => Capability::BtWhenPoweredOn,
            Setting::BtCallVolume(_) => Capability::BtCallVolume,
            Setting::EqualizerPreset(_) => Capability::EqualizerPreset,
            Setting::Equalizer(_) => Capability::Equalizer,
            Setting::PlayNotificationSound(_) => Capability::NotificationSound,
        }
    }

    /// The event that reports this setting as applied. Backends that can't read a
    /// setting back report it with this once the headset accepted it.
    pub fn confirmation(&self) -> Option<Event> {
        Some(match self.clone() {
            Setting::MicMuted(v) => Event::MicMuted(v),
            Setting::Sidetone(v) => Event::Sidetone(v),
            Setting::SidetoneLevel(v) => Event::SidetoneLevel(v),
            Setting::AutoOff(v) => Event::AutoOff(v),
            Setting::VoicePrompts(v) => Event::VoicePrompts(v),
            Setting::Lights(v) => Event::Lights(v),
            Setting::RotateToMute(v) => Event::RotateToMute(v),
            Setting::MicVolume(v) => Event::MicVolume(v),
            Setting::MicMuteLedBrightness(v) => Event::MicMuteLedBrightness(v),
            Setting::NoiseFilter(v) => Event::NoiseFilter(v),
            Setting::VolumeLimiter(v) => Event::VolumeLimiter(v),
            Setting::BtWhenPoweredOn(v) => Event::BtWhenPoweredOn(v),
            Setting::BtCallVolume(v) => Event::BtCallVolume(v),
            Setting::EqualizerPreset(v) => Event::EqualizerPreset(v),
            Setting::Equalizer(v) => Event::Equalizer(v),
            Setting::PlayNotificationSound(_) => return None,
        })
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
    Lights(bool),
    /// Game/chat balance: 0 is all game, 128 all chat, 64 even
    ChatMix(u8),
    RotateToMute(bool),
    MicVolume(Percent),
    MicMuteLedBrightness(u8),
    NoiseFilter(NoiseFilter),
    VolumeLimiter(bool),
    BtWhenPoweredOn(bool),
    BtCallVolume(Percent),
    EqualizerPreset(u8),
    Equalizer(Vec<Gain>),
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
    pub lights: Option<bool>,
    pub chatmix: Option<u8>,
    pub rotate_to_mute: Option<bool>,
    pub mic_volume: Option<Percent>,
    pub mic_mute_led_brightness: Option<u8>,
    pub noise_filter: Option<NoiseFilter>,
    pub volume_limiter: Option<bool>,
    pub bt_when_powered_on: Option<bool>,
    pub bt_call_volume: Option<Percent>,
    pub equalizer_preset: Option<u8>,
    pub equalizer: Option<Vec<Gain>>,
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
            Event::Lights(v) => set(&mut self.lights, v),
            Event::ChatMix(v) => set(&mut self.chatmix, v),
            Event::RotateToMute(v) => set(&mut self.rotate_to_mute, v),
            Event::MicVolume(v) => set(&mut self.mic_volume, v),
            Event::MicMuteLedBrightness(v) => set(&mut self.mic_mute_led_brightness, v),
            Event::NoiseFilter(v) => set(&mut self.noise_filter, v),
            Event::VolumeLimiter(v) => set(&mut self.volume_limiter, v),
            Event::BtWhenPoweredOn(v) => set(&mut self.bt_when_powered_on, v),
            Event::BtCallVolume(v) => set(&mut self.bt_call_volume, v),
            Event::EqualizerPreset(v) => set(&mut self.equalizer_preset, v),
            Event::Equalizer(v) => set(&mut self.equalizer, v),
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

    fn every_setting() -> Vec<Setting> {
        vec![
            Setting::MicMuted(true),
            Setting::Sidetone(true),
            Setting::SidetoneLevel(Percent::saturating(30)),
            Setting::AutoOff(AutoOff::minutes(20)),
            Setting::VoicePrompts(false),
            Setting::Lights(true),
            Setting::RotateToMute(true),
            Setting::MicVolume(Percent::saturating(70)),
            Setting::MicMuteLedBrightness(2),
            Setting::NoiseFilter(NoiseFilter::High),
            Setting::VolumeLimiter(true),
            Setting::BtWhenPoweredOn(false),
            Setting::BtCallVolume(Percent::saturating(40)),
            Setting::EqualizerPreset(1),
            Setting::Equalizer(vec![Gain::tenths(20), Gain::tenths(-10)]),
        ]
    }

    #[test]
    fn apply_reports_only_real_changes() {
        let mut status = Status::default();
        assert!(status.apply(&Event::Battery(Percent::saturating(99))));
        assert!(!status.apply(&Event::Battery(Percent::saturating(99))));
        assert!(status.apply(&Event::Battery(Percent::saturating(98))));
        assert_eq!(status.battery, Some(Percent::saturating(98)));
    }

    #[test]
    fn every_setting_is_confirmed_by_an_event_that_fills_the_status() {
        let mut status = Status::default();
        for setting in every_setting() {
            let event = setting.confirmation().expect("lasting settings have a confirmation");
            assert!(status.apply(&event), "{setting:?}");
        }
        assert_eq!(status.mic_muted, Some(true));
        assert_eq!(status.noise_filter, Some(NoiseFilter::High));
        assert_eq!(status.equalizer, Some(vec![Gain::tenths(20), Gain::tenths(-10)]));
        assert_eq!(status.bt_call_volume, Some(Percent::saturating(40)));
    }

    #[test]
    fn notification_sounds_are_not_lasting_settings() {
        assert_eq!(Setting::PlayNotificationSound(1).confirmation(), None);
        assert_eq!(Setting::PlayNotificationSound(1).capability(), Capability::NotificationSound);
    }

    #[test]
    fn apply_covers_read_only_values() {
        let mut status = Status::default();
        for event in [
            Event::Connected(true),
            Event::Charge(ChargeState::Charging),
            Event::MicAttached(true),
            Event::Color("Black/Red".into()),
            Event::ChatMix(64),
        ] {
            assert!(status.apply(&event), "{event:?}");
        }
        assert_eq!(status.chatmix, Some(64));
        assert_eq!(status.color.as_deref(), Some("Black/Red"));
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
        let capabilities: Vec<Capability> = every_setting().iter().map(Setting::capability).collect();
        assert_eq!(capabilities.len(), 15);
        assert_eq!(capabilities[0], Capability::MicMute);
        assert_eq!(capabilities[9], Capability::NoiseFilter);
        assert_eq!(capabilities[14], Capability::Equalizer);
    }
}
