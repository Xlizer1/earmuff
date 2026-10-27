//! The native session, codec and use cases working together against the simulated
//! Cloud Alpha Wireless, through the same Backend port the app uses.

use std::time::Duration;

use earmuff_application::{Backend, Session, SessionError, apply_settings, read_status, settle};
use earmuff_domain::{AutoOff, ChargeState, Event, Percent, Query, Setting, Status};
use earmuff_infrastructure::demo::{DemoBackend, DemoHeadset};

const TIMEOUT: Duration = Duration::from_secs(2);

fn open() -> (Box<dyn Session>, DemoHeadset) {
    let backend = DemoBackend::new();
    let headset = backend.headsets()[0].clone();
    let device = backend.discover().unwrap().remove(0);
    (backend.open(&device).unwrap(), headset)
}

fn percent(value: u8) -> Percent {
    Percent::new(value).unwrap()
}

#[test]
fn reads_the_whole_status() {
    let (mut session, _headset) = open();
    let status = read_status(session.as_mut(), TIMEOUT).unwrap();
    assert_eq!(
        status,
        Status {
            connected: Some(true),
            battery: Some(percent(80)),
            charge: Some(ChargeState::Discharging),
            mic_muted: Some(false),
            mic_attached: Some(true),
            sidetone: Some(false),
            sidetone_level: Some(percent(0)),
            auto_off: Some(AutoOff::minutes(20)),
            voice_prompts: Some(true),
            color: Some("Black/Red".into()),
            ..Status::default()
        }
    );
}

#[test]
fn every_query_is_answered_although_the_dongle_drops_bursts() {
    let (mut session, headset) = open();
    read_status(session.as_mut(), TIMEOUT).unwrap();
    // the session waits for each answer, so the dongle never had to drop anything
    assert_eq!(headset.with(|sim, _| sim.dropped), 0);
    assert_eq!(headset.with(|sim, _| sim.received.len()), 11);
}

#[test]
fn reports_only_the_connection_when_the_headset_is_off() {
    let (mut session, headset) = open();
    headset.with(|sim, now| sim.set_powered(false, now));
    let status = read_status(session.as_mut(), TIMEOUT).unwrap();
    assert_eq!(status.connected, Some(false));
    assert_eq!(status.battery, None);
}

#[test]
fn settings_are_applied_and_confirmed() {
    let (mut session, headset) = open();
    let status = apply_settings(
        session.as_mut(),
        &[
            Setting::Sidetone(true),
            Setting::SidetoneLevel(percent(50)),
            Setting::AutoOff(AutoOff::minutes(45)),
            Setting::VoicePrompts(false),
            Setting::MicMuted(true),
        ],
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(status.sidetone, Some(true));
    assert_eq!(status.sidetone_level, Some(percent(50)));
    assert_eq!(status.auto_off, Some(AutoOff::minutes(45)));
    assert_eq!(status.voice_prompts, Some(false));
    assert_eq!(status.mic_muted, Some(true));
    headset.with(|sim, _| {
        assert!(sim.sidetone && sim.muted && !sim.voice_prompts);
        assert_eq!((sim.sidetone_level, sim.auto_off), (7, 45));
    });
}

#[test]
fn the_kept_level_is_reported_not_the_echo() {
    let (mut session, headset) = open();
    let status = apply_settings(session.as_mut(), &[Setting::SidetoneLevel(percent(100))], TIMEOUT).unwrap();
    assert_eq!(headset.with(|sim, _| sim.sidetone_level), 18);
    assert_eq!(status.sidetone_level, Some(percent(100)));
}

#[test]
fn buttons_on_the_headset_arrive_as_events() {
    let (mut session, headset) = open();
    headset.with(|sim, now| sim.press_mute_button(now));
    assert_eq!(session.poll(TIMEOUT).unwrap(), [Event::MicMuted(true)]);
    headset.with(|sim, now| sim.set_battery(42, now));
    assert_eq!(session.poll(TIMEOUT).unwrap(), [Event::Battery(percent(42))]);
    headset.with(|sim, now| sim.set_charging(1, now));
    assert_eq!(session.poll(TIMEOUT).unwrap(), [Event::Charge(ChargeState::Charging)]);
}

#[test]
fn the_waker_interrupts_a_long_poll() {
    let (mut session, _headset) = open();
    let waker = session.waker();
    let started = std::time::Instant::now();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        waker.wake();
    });
    assert_eq!(session.poll(Duration::from_secs(30)).unwrap(), []);
    assert!(started.elapsed() < Duration::from_secs(5));
    thread.join().unwrap();
}

#[test]
fn unplugging_the_dongle_ends_the_session() {
    let (mut session, headset) = open();
    headset.unplug();
    assert!(matches!(session.poll(TIMEOUT), Err(SessionError::Disconnected(_))));
    assert!(matches!(session.query(Query::All), Err(SessionError::Disconnected(_))));
}

#[test]
fn raw_mode_shows_every_report() {
    let (mut session, _headset) = open();
    session.set_raw(true);
    session.query(Query::Connection).unwrap();
    let mut status = Status::default();
    let mut events = Vec::new();
    while !session.is_idle() {
        events.extend(session.poll(TIMEOUT).unwrap());
    }
    settle(session.as_mut(), &mut status, TIMEOUT).unwrap();
    assert!(events.contains(&Event::Connected(true)));
    assert!(events.iter().any(|e| matches!(e, Event::Raw(report) if report[..4] == [0x21, 0xBB, 0x03, 0x02])));
}

#[test]
fn unplugged_headsets_are_not_discovered() {
    let backend = DemoBackend::new();
    let device = backend.discover().unwrap().remove(0);
    assert_eq!(device.name, "Cloud Alpha Wireless (demo)");
    backend.headsets()[0].unplug();
    assert_eq!(backend.discover().unwrap(), []);
    assert!(backend.open(&device).is_err());
}
