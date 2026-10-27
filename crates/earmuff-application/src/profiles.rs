//! Remembering each headset's settings, and putting them back when it reconnects.
//! Some headsets forget their settings when turned off, and HeadsetControl can't
//! read most settings back at all.

use std::collections::HashMap;
use std::sync::Arc;

use earmuff_domain::{Capability, DeviceId, DeviceInfo, Event, Setting, Status, UsbId};

use crate::manager::AppEvent;

/// Keeps the remembered settings of each headset model, by USB ID
pub trait ProfileStore: Send + Sync {
    fn load(&self, headset: UsbId) -> Vec<Setting>;

    fn save(&self, headset: UsbId, settings: &[Setting]) -> Result<(), String>;
}

/// The setting an event reports, if it's one worth putting back: lasting settings
/// the user chose, not the mic mute or the battery
pub fn remembered(event: &Event) -> Option<Setting> {
    Some(match event.clone() {
        Event::Sidetone(v) => Setting::Sidetone(v),
        Event::SidetoneLevel(v) => Setting::SidetoneLevel(v),
        Event::AutoOff(v) => Setting::AutoOff(v),
        Event::VoicePrompts(v) => Setting::VoicePrompts(v),
        Event::Lights(v) => Setting::Lights(v),
        Event::RotateToMute(v) => Setting::RotateToMute(v),
        Event::MicVolume(v) => Setting::MicVolume(v),
        Event::MicMuteLedBrightness(v) => Setting::MicMuteLedBrightness(v),
        Event::NoiseFilter(v) => Setting::NoiseFilter(v),
        Event::VolumeLimiter(v) => Setting::VolumeLimiter(v),
        Event::BtWhenPoweredOn(v) => Setting::BtWhenPoweredOn(v),
        Event::BtCallVolume(v) => Setting::BtCallVolume(v),
        Event::EqualizerPreset(v) => Setting::EqualizerPreset(v),
        Event::Equalizer(v) => Setting::Equalizer(v),
        _ => return None,
    })
}

/// The order settings go back in: a preset before the curve, so a custom curve wins
fn restore_order(setting: &Setting) -> u8 {
    match setting.capability() {
        Capability::Sidetone => 0,
        Capability::EqualizerPreset => 1,
        Capability::Equalizer => 2,
        _ => 3,
    }
}

struct Tracked {
    info: DeviceInfo,
    status: Status,
    /// One remembered setting per capability
    profile: HashMap<Capability, Setting>,
}

/// Records each headset's settings as it reports them, and says what to put back
/// when it comes back on
pub struct Profiles {
    store: Arc<dyn ProfileStore>,
    enabled: bool,
    devices: HashMap<DeviceId, Tracked>,
}

impl Profiles {
    pub fn new(store: Arc<dyn ProfileStore>, enabled: bool) -> Self {
        Profiles { store, enabled, devices: HashMap::new() }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Takes every device manager event; returns the settings to put back now
    pub fn on_event(&mut self, event: &AppEvent) -> Vec<(DeviceId, Setting)> {
        match event {
            AppEvent::Added(info) => {
                let profile = self.store.load(info.id.usb).into_iter().map(|s| (s.capability(), s)).collect();
                self.devices
                    .insert(info.id.clone(), Tracked { info: info.clone(), status: Status::default(), profile });
                Vec::new()
            }
            AppEvent::Removed(id) => {
                self.devices.remove(id);
                Vec::new()
            }
            AppEvent::Changed(id, change) => {
                let Some(tracked) = self.devices.get_mut(id) else { return Vec::new() };
                let was_on = tracked.status.is_connected();
                tracked.status.apply(change);
                if !self.enabled {
                    return Vec::new();
                }
                if let Some(setting) = remembered(change) {
                    tracked.profile.insert(setting.capability(), setting);
                    let settings: Vec<Setting> = tracked.profile.values().cloned().collect();
                    // a lost save only means less is restored next time
                    let _ = self.store.save(tracked.info.id.usb, &settings);
                }
                if !was_on && tracked.status.is_connected() {
                    let mut restore: Vec<Setting> = tracked
                        .profile
                        .values()
                        .filter(|setting| tracked.info.check(setting).is_ok())
                        .cloned()
                        .collect();
                    restore.sort_by_key(restore_order);
                    return restore.into_iter().map(|setting| (id.clone(), setting)).collect();
                }
                Vec::new()
            }
            AppEvent::Failed(..) | AppEvent::Problem(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{AutoOff, BackendKind, EqualizerSpec, Gain, Percent};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryStore(Mutex<HashMap<UsbId, Vec<Setting>>>);

    impl ProfileStore for MemoryStore {
        fn load(&self, headset: UsbId) -> Vec<Setting> {
            self.0.lock().unwrap().get(&headset).cloned().unwrap_or_default()
        }

        fn save(&self, headset: UsbId, settings: &[Setting]) -> Result<(), String> {
            self.0.lock().unwrap().insert(headset, settings.to_vec());
            Ok(())
        }
    }

    fn device() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "a".into() },
            name: "Headset".into(),
            vendor: "V".into(),
            capabilities: vec![
                Capability::Lights,
                Capability::AutoOff,
                Capability::Sidetone,
                Capability::SidetoneLevel,
                Capability::EqualizerPreset,
                Capability::Equalizer,
                Capability::MicMute,
            ],
            equalizer: Some(EqualizerSpec {
                bands: 2,
                min: Gain::tenths(-60),
                max: Gain::tenths(60),
                step: Gain::tenths(10),
                baseline: Gain::tenths(0),
                presets: vec![],
            }),
        }
    }

    fn feed(profiles: &mut Profiles, events: &[Event]) -> Vec<Setting> {
        events
            .iter()
            .flat_map(|event| profiles.on_event(&AppEvent::Changed(device().id, event.clone())))
            .map(|(_, setting)| setting)
            .collect()
    }

    fn start(store: &Arc<MemoryStore>, enabled: bool) -> Profiles {
        let mut profiles = Profiles::new(store.clone(), enabled);
        profiles.on_event(&AppEvent::Added(device()));
        profiles
    }

    #[test]
    fn records_lasting_settings_but_not_the_mute() {
        let store = Arc::new(MemoryStore::default());
        let mut profiles = start(&store, true);
        feed(
            &mut profiles,
            &[Event::Connected(true), Event::Lights(false), Event::MicMuted(true), Event::Lights(true)],
        );
        assert_eq!(store.load(UsbId::new(1, 2)), [Setting::Lights(true)]);
    }

    #[test]
    fn puts_settings_back_when_the_headset_comes_back_on() {
        let store = Arc::new(MemoryStore::default());
        let mut profiles = start(&store, true);
        feed(&mut profiles, &[Event::Connected(true), Event::Lights(false), Event::AutoOff(AutoOff::minutes(45))]);
        assert_eq!(feed(&mut profiles, &[Event::Connected(false)]), []);
        let mut restored = feed(&mut profiles, &[Event::Connected(true)]);
        restored.sort_by_key(|s| format!("{s:?}"));
        assert_eq!(restored, [Setting::AutoOff(AutoOff::minutes(45)), Setting::Lights(false)]);
    }

    #[test]
    fn remembers_across_runs() {
        let store = Arc::new(MemoryStore::default());
        let mut first = start(&store, true);
        feed(&mut first, &[Event::Connected(true), Event::Lights(false)]);
        let mut second = start(&store, true);
        assert_eq!(feed(&mut second, &[Event::Connected(true)]), [Setting::Lights(false)]);
    }

    #[test]
    fn a_preset_goes_back_before_the_curve() {
        let store = Arc::new(MemoryStore::default());
        let mut profiles = start(&store, true);
        let curve = vec![Gain::tenths(10), Gain::tenths(20)];
        feed(&mut profiles, &[Event::Connected(true), Event::Equalizer(curve.clone()), Event::EqualizerPreset(1)]);
        feed(&mut profiles, &[Event::SidetoneLevel(Percent::saturating(40)), Event::Sidetone(true)]);
        feed(&mut profiles, &[Event::Connected(false)]);
        let restored = feed(&mut profiles, &[Event::Connected(true)]);
        let order: Vec<Capability> = restored.iter().map(Setting::capability).collect();
        assert_eq!(&order[..3], [Capability::Sidetone, Capability::EqualizerPreset, Capability::Equalizer]);
    }

    #[test]
    fn does_nothing_when_turned_off() {
        let store = Arc::new(MemoryStore::default());
        store.save(UsbId::new(1, 2), &[Setting::Lights(false)]).unwrap();
        let mut profiles = start(&store, false);
        assert_eq!(feed(&mut profiles, &[Event::Connected(true), Event::Lights(true)]), []);
        assert_eq!(store.load(UsbId::new(1, 2)), [Setting::Lights(false)]);
    }

    #[test]
    fn skips_settings_the_headset_no_longer_takes() {
        let store = Arc::new(MemoryStore::default());
        store.save(UsbId::new(1, 2), &[Setting::VolumeLimiter(true), Setting::Lights(true)]).unwrap();
        let mut profiles = start(&store, true);
        assert_eq!(feed(&mut profiles, &[Event::Connected(true)]), [Setting::Lights(true)]);
    }
}
