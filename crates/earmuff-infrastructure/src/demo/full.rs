//! A simulated headset with every capability Earmuff knows, so the whole app can be
//! tried (and tested) without owning one. It is simulated at the level of domain
//! values, not bytes.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use earmuff_application::{Session, SessionError, Waker};
use earmuff_domain::{
    AutoOff, BackendKind, Capability, ChargeState, DeviceId, DeviceInfo, EqPreset, EqualizerSpec, Event, Gain,
    NoiseFilter, Percent, Query, Setting, Status, UsbId,
};

/// A made-up USB ID for the demo headset
pub const FULL_DEMO_USB: UsbId = UsbId::new(0xF00B, 0xA00D);

const CAPABILITIES: [Capability; 19] = [
    Capability::Battery,
    Capability::MicMute,
    Capability::MicAttached,
    Capability::Sidetone,
    Capability::SidetoneLevel,
    Capability::AutoOff,
    Capability::VoicePrompts,
    Capability::Lights,
    Capability::ChatMix,
    Capability::RotateToMute,
    Capability::MicVolume,
    Capability::MicMuteLedBrightness,
    Capability::NoiseFilter,
    Capability::VolumeLimiter,
    Capability::BtWhenPoweredOn,
    Capability::BtCallVolume,
    Capability::EqualizerPreset,
    Capability::Equalizer,
    Capability::NotificationSound,
];

fn curve(db: [i16; 10]) -> Vec<Gain> {
    db.iter().map(|d| Gain::tenths(d * 10)).collect()
}

pub fn equalizer() -> EqualizerSpec {
    EqualizerSpec {
        bands: 10,
        min: Gain::tenths(-120),
        max: Gain::tenths(120),
        step: Gain::tenths(5),
        baseline: Gain::tenths(0),
        presets: vec![
            EqPreset { name: "Flat".into(), curve: curve([0; 10]) },
            EqPreset { name: "Bass boost".into(), curve: curve([6, 4, 2, 0, 0, 0, 0, 0, 0, 0]) },
            EqPreset { name: "Footsteps".into(), curve: curve([-4, -3, -2, 0, 2, 4, 6, 4, 2, 0]) },
            EqPreset { name: "Vocal".into(), curve: curve([-2, -1, 0, 2, 4, 4, 2, 0, -1, -2]) },
        ],
    }
}

fn starting_status() -> Status {
    Status {
        connected: Some(true),
        battery: Some(Percent::saturating(64)),
        charge: Some(ChargeState::Discharging),
        mic_muted: Some(false),
        mic_attached: Some(true),
        sidetone: Some(false),
        sidetone_level: Some(Percent::saturating(50)),
        auto_off: Some(AutoOff::minutes(30)),
        voice_prompts: Some(true),
        color: Some("Graphite".into()),
        lights: Some(true),
        chatmix: Some(64),
        rotate_to_mute: Some(true),
        mic_volume: Some(Percent::saturating(80)),
        mic_mute_led_brightness: Some(2),
        noise_filter: Some(NoiseFilter::Low),
        volume_limiter: Some(false),
        bt_when_powered_on: Some(true),
        bt_call_volume: Some(Percent::saturating(70)),
        equalizer_preset: Some(0),
        equalizer: Some(curve([0; 10])),
    }
}

/// Every value in `status` as events, for answering a query
fn report(status: &Status, query: Query) -> Vec<Event> {
    let connection = status.connected.map(Event::Connected);
    let battery = [status.battery.map(Event::Battery), status.charge.map(Event::Charge)];
    if query == Query::Connection {
        return connection.into_iter().collect();
    }
    if query == Query::Battery || status.connected != Some(true) {
        return connection.into_iter().chain(battery.into_iter().flatten()).collect();
    }
    let s = status.clone();
    [
        connection,
        battery[0].clone(),
        battery[1].clone(),
        s.mic_muted.map(Event::MicMuted),
        s.mic_attached.map(Event::MicAttached),
        s.sidetone.map(Event::Sidetone),
        s.sidetone_level.map(Event::SidetoneLevel),
        s.auto_off.map(Event::AutoOff),
        s.voice_prompts.map(Event::VoicePrompts),
        s.color.map(Event::Color),
        s.lights.map(Event::Lights),
        s.chatmix.map(Event::ChatMix),
        s.rotate_to_mute.map(Event::RotateToMute),
        s.mic_volume.map(Event::MicVolume),
        s.mic_mute_led_brightness.map(Event::MicMuteLedBrightness),
        s.noise_filter.map(Event::NoiseFilter),
        s.volume_limiter.map(Event::VolumeLimiter),
        s.bt_when_powered_on.map(Event::BtWhenPoweredOn),
        s.bt_call_volume.map(Event::BtCallVolume),
        s.equalizer_preset.map(Event::EqualizerPreset),
        s.equalizer.map(Event::Equalizer),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[derive(Debug)]
struct State {
    status: Status,
    plugged: bool,
    woken: bool,
    outbox: VecDeque<Event>,
    /// Notification sounds played, in order
    played: Vec<u8>,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// The demo headset with every feature. Clones control the same headset.
#[derive(Clone, Debug)]
pub struct FullHeadset {
    shared: Arc<Shared>,
    instance: String,
}

impl FullHeadset {
    pub fn new(instance: impl Into<String>) -> Self {
        FullHeadset {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    status: starting_status(),
                    plugged: true,
                    woken: false,
                    outbox: VecDeque::new(),
                    played: Vec::new(),
                }),
                changed: Condvar::new(),
            }),
            instance: instance.into(),
        }
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let result = change(&mut self.lock());
        self.shared.changed.notify_all();
        result
    }

    /// The headset changes on its own (a dial, a button, the battery) and says so
    fn happen(&self, event: Event) {
        self.update(|state| {
            state.status.apply(&event);
            state.outbox.push_back(event);
        });
    }

    pub fn status(&self) -> Status {
        self.lock().status.clone()
    }

    pub fn played_sounds(&self) -> Vec<u8> {
        self.lock().played.clone()
    }

    pub fn set_battery(&self, level: u8) {
        self.happen(Event::Battery(Percent::saturating(level.into())));
    }

    pub fn set_charge(&self, charge: ChargeState) {
        self.happen(Event::Charge(charge));
    }

    /// Turns the game/chat dial
    pub fn set_chatmix(&self, value: u8) {
        self.happen(Event::ChatMix(value));
    }

    pub fn press_mute_button(&self) {
        let muted = self.lock().status.mic_muted == Some(true);
        self.happen(Event::MicMuted(!muted));
    }

    pub fn set_powered(&self, on: bool) {
        self.happen(Event::Connected(on));
    }

    pub fn unplug(&self) {
        self.update(|state| state.plugged = false);
    }

    pub fn plug(&self) {
        self.update(|state| state.plugged = true);
    }

    pub fn is_plugged(&self) -> bool {
        self.lock().plugged
    }

    pub fn info(&self) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: FULL_DEMO_USB, instance: self.instance.clone() },
            name: "Earmuff Demo Headset".into(),
            vendor: "Earmuff".into(),
            capabilities: CAPABILITIES.to_vec(),
            equalizer: Some(equalizer()),
        }
    }

    pub fn session(&self) -> FullSession {
        FullSession { info: self.info(), headset: self.clone() }
    }
}

/// A session with the demo headset
#[derive(Debug)]
pub struct FullSession {
    info: DeviceInfo,
    headset: FullHeadset,
}

fn unplugged() -> SessionError {
    SessionError::Disconnected("demo headset unplugged".into())
}

impl Session for FullSession {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn query(&mut self, query: Query) -> Result<(), SessionError> {
        self.headset.update(|state| {
            if !state.plugged {
                return Err(unplugged());
            }
            let events = report(&state.status, query);
            state.outbox.extend(events);
            Ok(())
        })
    }

    fn apply(&mut self, setting: Setting) -> Result<(), SessionError> {
        self.info.check(&setting)?;
        let preset_curve = match &setting {
            Setting::EqualizerPreset(index) => equalizer().presets.get(usize::from(*index)).map(|p| p.curve.clone()),
            _ => None,
        };
        self.headset.update(|state| {
            if !state.plugged {
                return Err(unplugged());
            }
            if state.status.connected != Some(true) {
                return Err(SessionError::Rejected("the headset is off".into()));
            }
            if let Setting::PlayNotificationSound(sound) = setting {
                state.played.push(sound);
            }
            let events = setting.confirmation().into_iter().chain(preset_curve.map(Event::Equalizer));
            for event in events {
                state.status.apply(&event);
                state.outbox.push_back(event);
            }
            Ok(())
        })
    }

    fn poll(&mut self, timeout: Duration) -> Result<Vec<Event>, SessionError> {
        let end = Instant::now() + timeout;
        let mut state = self.headset.lock();
        loop {
            if !state.plugged {
                return Err(unplugged());
            }
            if !state.outbox.is_empty() {
                return Ok(state.outbox.drain(..).collect());
            }
            if state.woken {
                state.woken = false;
                return Ok(Vec::new());
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(Vec::new());
            }
            state = self.headset.shared.changed.wait_timeout(state, left).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    fn is_idle(&self) -> bool {
        self.headset.lock().outbox.is_empty()
    }

    fn waker(&self) -> Waker {
        let headset = self.headset.clone();
        Waker::new(move || headset.update(|state| state.woken = true))
    }

    fn set_raw(&mut self, _raw: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(session: &mut FullSession) -> Vec<Event> {
        session.poll(Duration::ZERO).unwrap()
    }

    #[test]
    fn supports_everything_with_a_valid_equalizer() {
        let info = FullHeadset::new("x").info();
        assert_eq!(info.capabilities.len(), 19);
        let eq = info.equalizer.unwrap();
        assert!(eq.presets.iter().all(|preset| eq.check(&preset.curve).is_ok()));
    }

    #[test]
    fn reports_everything_when_asked() {
        let headset = FullHeadset::new("x");
        let mut session = headset.session();
        session.query(Query::All).unwrap();
        let mut status = Status::default();
        for event in drain(&mut session) {
            status.apply(&event);
        }
        assert_eq!(status, headset.status());
        session.query(Query::Battery).unwrap();
        assert_eq!(drain(&mut session).len(), 3);
    }

    #[test]
    fn keeps_settings_and_confirms_them() {
        let headset = FullHeadset::new("x");
        let mut session = headset.session();
        session.apply(Setting::NoiseFilter(NoiseFilter::High)).unwrap();
        session.apply(Setting::EqualizerPreset(1)).unwrap();
        let events = drain(&mut session);
        assert_eq!(events[0], Event::NoiseFilter(NoiseFilter::High));
        assert_eq!(events[1], Event::EqualizerPreset(1));
        assert_eq!(events[2], Event::Equalizer(equalizer().presets[1].curve.clone()));
        assert_eq!(headset.status().equalizer, Some(equalizer().presets[1].curve.clone()));
    }

    #[test]
    fn plays_notification_sounds() {
        let headset = FullHeadset::new("x");
        headset.session().apply(Setting::PlayNotificationSound(1)).unwrap();
        assert_eq!(headset.played_sounds(), [1]);
    }

    #[test]
    fn refuses_settings_while_off_and_invalid_values() {
        let headset = FullHeadset::new("x");
        let mut session = headset.session();
        assert!(session.apply(Setting::Equalizer(vec![Gain::tenths(0); 3])).is_err());
        headset.set_powered(false);
        assert_eq!(session.apply(Setting::Lights(false)), Err(SessionError::Rejected("the headset is off".into())));
    }

    #[test]
    fn dials_and_buttons_arrive_as_events() {
        let headset = FullHeadset::new("x");
        let mut session = headset.session();
        headset.set_chatmix(20);
        headset.press_mute_button();
        assert_eq!(drain(&mut session), [Event::ChatMix(20), Event::MicMuted(true)]);
        headset.unplug();
        assert!(session.poll(Duration::ZERO).is_err());
    }
}
