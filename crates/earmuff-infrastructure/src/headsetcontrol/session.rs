//! One headset reached through HeadsetControl.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use earmuff_application::{Session, SessionError, Waker};
use earmuff_domain::{DeviceInfo, Event, Percent, Query, Setting};

use super::json::{self, LEVEL_MAX};
use super::runner::Runner;

/// HeadsetControl's inactive time goes from 0 to 90 minutes
const AUTO_OFF_MAX: u8 = 90;

#[derive(Default)]
struct Wake {
    woken: Mutex<bool>,
    signal: Condvar,
}

pub struct HeadsetControlSession {
    info: DeviceInfo,
    runner: Arc<dyn Runner>,
    /// `-d` argument selecting this headset, e.g. 0x1038:0x12ad
    selector: String,
    poll_every: Duration,
    next_poll: Instant,
    refresh: bool,
    outbox: VecDeque<Event>,
    /// Mic monitoring level (0-128) to restore when monitoring is switched back on
    sidetone_level: i32,
    wake: Arc<Wake>,
}

/// The HeadsetControl arguments that apply a setting
pub fn arguments(setting: &Setting, sidetone_level: i32) -> Result<Vec<String>, SessionError> {
    let flag = |on: bool| if on { "1" } else { "0" }.to_string();
    let level = |p: Percent| p.to_range(0, LEVEL_MAX).to_string();
    Ok(match setting {
        Setting::Sidetone(on) => vec!["-s".into(), if *on { sidetone_level.to_string() } else { "0".into() }],
        Setting::SidetoneLevel(p) => vec!["-s".into(), level(*p)],
        Setting::AutoOff(minutes) if minutes.get() > AUTO_OFF_MAX => {
            return Err(SessionError::Rejected(format!("auto-off goes up to {AUTO_OFF_MAX} minutes on this headset")));
        }
        Setting::AutoOff(minutes) => vec!["-i".into(), minutes.get().to_string()],
        Setting::VoicePrompts(on) => vec!["-v".into(), flag(*on)],
        Setting::Lights(on) => vec!["-l".into(), flag(*on)],
        Setting::RotateToMute(on) => vec!["-r".into(), flag(*on)],
        Setting::MicVolume(p) => vec!["--microphone-volume".into(), level(*p)],
        Setting::MicMuteLedBrightness(n) => vec!["--microphone-mute-led-brightness".into(), n.to_string()],
        Setting::NoiseFilter(filter) => vec!["--noise-filter".into(), filter.level().to_string()],
        Setting::VolumeLimiter(on) => vec!["--volume-limiter".into(), flag(*on)],
        Setting::BtWhenPoweredOn(on) => vec!["--bt-when-powered-on".into(), flag(*on)],
        Setting::BtCallVolume(p) => vec!["--bt-call-volume".into(), p.get().to_string()],
        Setting::EqualizerPreset(index) => vec!["-p".into(), index.to_string()],
        Setting::Equalizer(curve) => {
            vec!["-e".into(), curve.iter().map(|g| g.db().to_string()).collect::<Vec<_>>().join(",")]
        }
        Setting::PlayNotificationSound(sound) => vec!["-n".into(), sound.to_string()],
        Setting::MicMuted(_) => return Err(SessionError::Unsupported(setting.capability())),
    })
}

impl HeadsetControlSession {
    pub fn new(info: DeviceInfo, runner: Arc<dyn Runner>, poll_every: Duration) -> Self {
        let selector = format!("0x{:04x}:0x{:04x}", info.id.usb.vendor, info.id.usb.product);
        HeadsetControlSession {
            info,
            runner,
            selector,
            poll_every,
            next_poll: Instant::now(),
            refresh: true,
            outbox: VecDeque::new(),
            sidetone_level: LEVEL_MAX / 2,
            wake: Arc::default(),
        }
    }

    fn run(&self, args: Vec<String>) -> Result<super::RunOutput, SessionError> {
        let mut full = vec!["-d".to_string(), self.selector.clone()];
        full.extend(args);
        full.extend(["-o".to_string(), "json".to_string()]);
        self.runner.run(&full).map_err(|e| SessionError::Disconnected(format!("headsetcontrol: {e}")))
    }

    /// Asks HeadsetControl for the battery, connection and other readable values
    fn read_status(&mut self) -> Result<(), SessionError> {
        let output = self.run(Vec::new())?;
        let report = json::parse(&output.stdout).map_err(SessionError::Disconnected)?;
        let device = report
            .devices
            .iter()
            .find(|d| d.usb == self.info.id.usb)
            .ok_or_else(|| SessionError::Disconnected("headset unplugged".into()))?;
        if let Some(level) = device.sidetone_level.filter(|level| *level > 0) {
            self.sidetone_level = level;
        }
        self.outbox.extend(json::events(device));
        Ok(())
    }

    /// The events that report `setting` as applied
    fn confirmations(&mut self, setting: &Setting) -> Vec<Event> {
        match setting {
            Setting::Sidetone(true) => {
                vec![
                    Event::Sidetone(true),
                    Event::SidetoneLevel(Percent::from_range(self.sidetone_level, 0, LEVEL_MAX)),
                ]
            }
            Setting::SidetoneLevel(p) => {
                let raw = p.to_range(0, LEVEL_MAX);
                if raw > 0 {
                    self.sidetone_level = raw;
                }
                vec![Event::Sidetone(raw > 0), Event::SidetoneLevel(*p)]
            }
            Setting::EqualizerPreset(index) => {
                let curve = self.info.equalizer.as_ref().and_then(|eq| eq.presets.get(usize::from(*index)));
                let mut events = vec![Event::EqualizerPreset(*index)];
                events.extend(curve.map(|preset| Event::Equalizer(preset.curve.clone())));
                events
            }
            other => other.confirmation().into_iter().collect(),
        }
    }
}

impl Session for HeadsetControlSession {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn query(&mut self, _query: Query) -> Result<(), SessionError> {
        // HeadsetControl reports everything it can read in one run
        self.refresh = true;
        Ok(())
    }

    fn apply(&mut self, setting: Setting) -> Result<(), SessionError> {
        self.info.check(&setting)?;
        let output = self.run(arguments(&setting, self.sidetone_level)?)?;
        let report = json::parse(&output.stdout).map_err(|_| {
            // invalid values make HeadsetControl print an error instead of JSON
            let printed = if output.stderr.trim().is_empty() { &output.stdout } else { &output.stderr };
            let reason = printed.trim().trim_start_matches("Error: ");
            SessionError::Rejected(if reason.is_empty() { "HeadsetControl failed".into() } else { reason.into() })
        })?;
        if report.actions.is_empty() {
            return Err(SessionError::Rejected("the headset didn't answer".into()));
        }
        if let Some(action) = report.actions.iter().find(|action| !action.success) {
            let reason = action
                .error
                .clone()
                .unwrap_or_else(|| format!("the headset refused the {} change", setting.capability().name()));
            return Err(SessionError::Rejected(reason));
        }
        let confirmations = self.confirmations(&setting);
        self.outbox.extend(confirmations);
        Ok(())
    }

    fn poll(&mut self, timeout: Duration) -> Result<Vec<Event>, SessionError> {
        let end = Instant::now() + timeout;
        loop {
            if !self.outbox.is_empty() {
                return Ok(self.outbox.drain(..).collect());
            }
            let now = Instant::now();
            if self.refresh || now >= self.next_poll {
                self.refresh = false;
                self.next_poll = now + self.poll_every;
                self.read_status()?;
                continue;
            }
            if now >= end {
                return Ok(Vec::new());
            }
            let woken = self.wake.woken.lock().unwrap_or_else(PoisonError::into_inner);
            let until = self.next_poll.min(end);
            let (mut woken, _) = self
                .wake
                .signal
                .wait_timeout_while(woken, until.saturating_duration_since(now), |woken| !*woken)
                .unwrap_or_else(PoisonError::into_inner);
            if *woken {
                *woken = false;
                return Ok(Vec::new());
            }
        }
    }

    fn is_idle(&self) -> bool {
        self.outbox.is_empty() && !self.refresh
    }

    fn waker(&self) -> Waker {
        let wake = self.wake.clone();
        Waker::new(move || {
            *wake.woken.lock().unwrap_or_else(PoisonError::into_inner) = true;
            wake.signal.notify_all();
        })
    }

    fn set_raw(&mut self, _raw: bool) {
        // HeadsetControl hides the raw reports
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headsetcontrol::{RunOutput, json::tests::SAMPLE};
    use earmuff_domain::{AutoOff, ChargeState, Gain, NoiseFilter};

    /// Answers status runs with SAMPLE and setting runs with `action`, recording every run
    struct Scripted {
        action: String,
        runs: Mutex<Vec<Vec<String>>>,
    }

    impl Runner for Scripted {
        fn run(&self, args: &[String]) -> std::io::Result<RunOutput> {
            self.runs.lock().unwrap().push(args.to_vec());
            let setting = args.len() > 4;
            let stdout = if setting { self.action.clone() } else { SAMPLE.into() };
            Ok(RunOutput { success: !stdout.starts_with("Error"), stdout, stderr: String::new() })
        }
    }

    fn open(action: &str) -> (HeadsetControlSession, Arc<Scripted>) {
        let runner = Arc::new(Scripted { action: action.into(), runs: Mutex::default() });
        let report = json::parse(SAMPLE).unwrap();
        let info = json::device_info(&report.devices[0]);
        (HeadsetControlSession::new(info, runner.clone(), Duration::from_secs(60)), runner)
    }

    const OK: &str =
        r#"{"api_version": "1.5", "devices": [], "actions": [{"capability": "CAP_X", "status": "success"}]}"#;

    #[test]
    fn translates_settings_into_arguments() {
        let args = |setting: Setting| arguments(&setting, 64).unwrap().join(" ");
        assert_eq!(args(Setting::Sidetone(false)), "-s 0");
        assert_eq!(args(Setting::Sidetone(true)), "-s 64");
        assert_eq!(args(Setting::SidetoneLevel(Percent::FULL)), "-s 128");
        assert_eq!(args(Setting::AutoOff(AutoOff::minutes(30))), "-i 30");
        assert_eq!(args(Setting::Lights(true)), "-l 1");
        assert_eq!(args(Setting::NoiseFilter(NoiseFilter::High)), "--noise-filter 2");
        assert_eq!(args(Setting::MicVolume(Percent::saturating(50))), "--microphone-volume 64");
        assert_eq!(args(Setting::BtCallVolume(Percent::saturating(50))), "--bt-call-volume 50");
        assert_eq!(args(Setting::EqualizerPreset(2)), "-p 2");
        assert_eq!(args(Setting::Equalizer(vec![Gain::tenths(25), Gain::tenths(-30), Gain::tenths(0)])), "-e 2.5,-3,0");
        assert_eq!(args(Setting::PlayNotificationSound(1)), "-n 1");
        assert!(matches!(arguments(&Setting::AutoOff(AutoOff::minutes(120)), 64), Err(SessionError::Rejected(_))));
        assert!(matches!(arguments(&Setting::MicMuted(true), 64), Err(SessionError::Unsupported(_))));
    }

    #[test]
    fn reads_the_status_on_the_first_poll() {
        let (mut session, runner) = open(OK);
        let events = session.poll(Duration::from_secs(1)).unwrap();
        assert!(events.contains(&Event::Battery(Percent::saturating(42))));
        assert!(events.contains(&Event::Charge(ChargeState::Discharging)));
        assert!(events.contains(&Event::ChatMix(64)));
        assert_eq!(runner.runs.lock().unwrap()[0], ["-d", "0xf00b:0xa00c", "-o", "json"]);
        assert!(session.is_idle());
    }

    #[test]
    fn applies_a_setting_and_reports_it_set() {
        let (mut session, runner) = open(OK);
        session.apply(Setting::Lights(true)).unwrap();
        assert_eq!(runner.runs.lock().unwrap()[0], ["-d", "0xf00b:0xa00c", "-l", "1", "-o", "json"]);
        assert_eq!(session.poll(Duration::ZERO).unwrap(), [Event::Lights(true)]);
    }

    #[test]
    fn a_preset_also_reports_its_curve() {
        let (mut session, _) = open(OK);
        session.apply(Setting::EqualizerPreset(1)).unwrap();
        assert_eq!(
            session.poll(Duration::ZERO).unwrap(),
            [Event::EqualizerPreset(1), Event::Equalizer(vec![Gain::tenths(60), Gain::tenths(40), Gain::tenths(20)])]
        );
    }

    #[test]
    fn switching_monitoring_back_on_restores_its_level() {
        let (mut session, runner) = open(OK);
        session.poll(Duration::from_secs(1)).unwrap(); // the headset reports level 85
        session.apply(Setting::Sidetone(false)).unwrap();
        session.apply(Setting::Sidetone(true)).unwrap();
        assert_eq!(runner.runs.lock().unwrap().last().unwrap()[2..4], ["-s", "85"]);
    }

    #[test]
    fn refused_settings_are_rejected_with_the_reason() {
        let refused = r#"{"api_version": "1.5", "devices": [], "actions": [
            {"capability": "CAP_LIGHTS", "status": "failure", "error": "Device is offline or not responding"}]}"#;
        let (mut session, _) = open(refused);
        assert_eq!(
            session.apply(Setting::Lights(true)),
            Err(SessionError::Rejected("Device is offline or not responding".into()))
        );
        let (mut session, _) = open("Error: lights: value out of range");
        assert_eq!(
            session.apply(Setting::Lights(true)),
            Err(SessionError::Rejected("lights: value out of range".into()))
        );
    }

    #[test]
    fn unsupported_settings_never_run_headsetcontrol() {
        let (mut session, runner) = open(OK);
        assert!(matches!(session.apply(Setting::RotateToMute(true)), Err(SessionError::Unsupported(_))));
        assert!(runner.runs.lock().unwrap().is_empty());
    }

    #[test]
    fn the_waker_interrupts_waiting() {
        let (mut session, _) = open(OK);
        session.poll(Duration::from_secs(1)).unwrap();
        session.waker().wake();
        let started = Instant::now();
        assert_eq!(session.poll(Duration::from_secs(30)).unwrap(), []);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
