//! The HeadsetControl backend against the real `headsetcontrol` program and its
//! built-in mock headset. Settings only ever go to the mock (f00b:a00c); real
//! headsets are only read during discovery. Skipped when HeadsetControl isn't installed.

use std::sync::Arc;
use std::time::Duration;

use earmuff_application::{Backend, Session, SessionError, apply_settings, read_status};
use earmuff_domain::{
    AutoOff, BackendKind, Capability, ChargeState, Event, Gain, NoiseFilter, Percent, Setting, UsbId,
};
use earmuff_infrastructure::headsetcontrol::{HeadsetControlBackend, Program};

const MOCK: UsbId = UsbId::new(0xF00B, 0xA00C);
const TIMEOUT: Duration = Duration::from_secs(5);

/// The mock headset with HeadsetControl's behavior `profile`, or None without HeadsetControl
fn mock(profile: u8) -> Option<Box<dyn Session>> {
    let Some(program) = Program::find() else {
        eprintln!("skipped: headsetcontrol is not installed");
        return None;
    };
    let backend = HeadsetControlBackend::new(Arc::new(program.with_test_device(profile)));
    let device = backend.discover().unwrap().into_iter().find(|d| d.id.usb == MOCK).expect("the mock is listed");
    Some(backend.open(&device).unwrap())
}

#[test]
fn describes_the_mock_headset() {
    let Some(session) = mock(0) else { return };
    let info = session.info();
    assert_eq!(info.id.backend, BackendKind::HeadsetControl);
    assert_eq!(info.name, "HeadsetControl Test device");
    for capability in [Capability::Battery, Capability::Lights, Capability::ChatMix, Capability::Equalizer] {
        assert!(info.supports(capability), "{capability:?}");
    }
    let eq = info.equalizer.as_ref().expect("the mock has an equalizer");
    assert_eq!(eq.bands, 10);
    assert_eq!(eq.presets.first().map(|p| p.name.as_str()), Some("flat"));
}

#[test]
fn reads_battery_chatmix_and_sidetone() {
    let Some(mut session) = mock(0) else { return };
    let status = read_status(session.as_mut(), TIMEOUT).unwrap();
    assert_eq!(status.connected, Some(true));
    assert_eq!(status.battery, Some(Percent::saturating(42)));
    assert_eq!(status.charge, Some(ChargeState::Discharging));
    assert_eq!(status.chatmix, Some(64));
    assert_eq!(status.sidetone, Some(true));
}

#[test]
fn follows_the_battery_profiles() {
    let Some(mut charging) = mock(2) else { return };
    let status = read_status(charging.as_mut(), TIMEOUT).unwrap();
    assert_eq!((status.battery, status.charge), (Some(Percent::saturating(50)), Some(ChargeState::Charging)));
    let Some(mut offline) = mock(4) else { return };
    assert_eq!(read_status(offline.as_mut(), TIMEOUT).unwrap().connected, Some(false));
}

#[test]
fn applies_settings_and_reports_them_set() {
    let Some(mut session) = mock(0) else { return };
    // like the app: read the headset first, then change it (the mock doesn't keep
    // what it's told, so a later status read would undo the confirmations)
    read_status(session.as_mut(), TIMEOUT).unwrap();
    let curve: Vec<Gain> = (0..10).map(|band| Gain::tenths(band * 5 - 20)).collect();
    let status = apply_settings(
        session.as_mut(),
        &[
            Setting::Lights(true),
            Setting::NoiseFilter(NoiseFilter::High),
            Setting::SidetoneLevel(Percent::saturating(50)),
            Setting::AutoOff(AutoOff::minutes(30)),
            Setting::Equalizer(curve.clone()),
        ],
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(status.lights, Some(true));
    assert_eq!(status.noise_filter, Some(NoiseFilter::High));
    assert_eq!((status.sidetone, status.sidetone_level), (Some(true), Some(Percent::saturating(50))));
    assert_eq!(status.auto_off, Some(AutoOff::minutes(30)));
    assert_eq!(status.equalizer, Some(curve));
}

#[test]
fn a_preset_reports_its_curve() {
    let Some(mut session) = mock(0) else { return };
    session.apply(Setting::EqualizerPreset(1)).unwrap();
    let events = session.poll(TIMEOUT).unwrap();
    assert!(events.contains(&Event::EqualizerPreset(1)));
    let bass_boost = session.info().equalizer.as_ref().unwrap().presets[1].curve.clone();
    assert!(events.contains(&Event::Equalizer(bass_boost)));
}

#[test]
fn out_of_range_values_are_refused_with_a_reason() {
    let Some(mut session) = mock(0) else { return };
    assert!(matches!(session.apply(Setting::AutoOff(AutoOff::minutes(120))), Err(SessionError::Rejected(_))));
    let bands = Setting::Equalizer(vec![Gain::tenths(0); 3]);
    assert_eq!(session.apply(bands), Err(SessionError::Rejected("this equalizer has 10 bands, not 3".into())));
}
