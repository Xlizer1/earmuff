//! Tests against a real HyperX Cloud Alpha Wireless. They only run on request:
//!
//!     cargo test -p earmuff-infrastructure --test hardware -- --ignored --test-threads=1
//!
//! They read the headset's settings first and put every one back afterwards, even
//! when a check fails. The headset must be on.

use std::sync::Arc;
use std::time::Duration;

use earmuff_application::{Backend, Session, apply_settings, open_first, read_status};
use earmuff_domain::{AutoOff, Percent, Setting, Status};
use earmuff_infrastructure::native::NativeBackend;

const TIMEOUT: Duration = Duration::from_secs(3);

/// Opens the real headset and restores its settings when dropped
struct RealHeadset {
    session: Box<dyn Session>,
    original: Status,
}

impl RealHeadset {
    fn open() -> Self {
        let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(NativeBackend::new())];
        let mut session = open_first(&backends).expect("plug in the Cloud Alpha Wireless dongle");
        let original = read_status(session.as_mut(), TIMEOUT).unwrap();
        assert!(original.is_connected(), "turn the headset on");
        RealHeadset { session, original }
    }

    fn apply(&mut self, settings: &[Setting]) -> Status {
        apply_settings(self.session.as_mut(), settings, TIMEOUT).unwrap()
    }
}

impl Drop for RealHeadset {
    fn drop(&mut self) {
        let o = &self.original;
        let restore: Vec<Setting> = [
            o.sidetone_level.map(Setting::SidetoneLevel),
            o.sidetone.map(Setting::Sidetone),
            o.auto_off.map(Setting::AutoOff),
            o.mic_muted.map(Setting::MicMuted),
            o.voice_prompts.map(Setting::VoicePrompts),
        ]
        .into_iter()
        .flatten()
        .collect();
        let _ = apply_settings(self.session.as_mut(), &restore, TIMEOUT);
    }
}

#[test]
#[ignore = "needs a HyperX Cloud Alpha Wireless"]
fn reads_every_value() {
    let headset = RealHeadset::open();
    let s = &headset.original;
    assert!(s.battery.is_some() && s.charge.is_some() && s.color.is_some());
    assert!(s.mic_muted.is_some() && s.mic_attached.is_some() && s.voice_prompts.is_some());
    assert!(s.sidetone.is_some() && s.sidetone_level.is_some() && s.auto_off.is_some());
}

#[test]
#[ignore = "needs a HyperX Cloud Alpha Wireless"]
fn sidetone_level_round_trips_through_percent() {
    let mut headset = RealHeadset::open();
    // monitoring stays off, so nothing is heard while the level changes
    headset.apply(&[Setting::Sidetone(false)]);
    for level in [0, 50, 100] {
        let status = headset.apply(&[Setting::SidetoneLevel(Percent::new(level).unwrap())]);
        assert_eq!(status.sidetone_level, Some(Percent::new(level).unwrap()), "level {level}%");
    }
}

#[test]
#[ignore = "needs a HyperX Cloud Alpha Wireless"]
fn auto_off_is_stored() {
    let mut headset = RealHeadset::open();
    let status = headset.apply(&[Setting::AutoOff(AutoOff::minutes(45))]);
    assert_eq!(status.auto_off, Some(AutoOff::minutes(45)));
}

#[test]
#[ignore = "needs a HyperX Cloud Alpha Wireless"]
fn mute_is_confirmed_by_the_headset() {
    let mut headset = RealHeadset::open();
    let muted = headset.original.mic_muted.unwrap_or(false);
    let status = headset.apply(&[Setting::MicMuted(!muted)]);
    assert_eq!(status.mic_muted, Some(!muted));
}
