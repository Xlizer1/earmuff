//! Making a headset the computer's sound device when it turns on.

use std::collections::HashMap;
use std::time::Duration;

use earmuff_domain::{DeviceId, Event, UsbId};

use crate::manager::AppEvent;

/// What came of asking the sound system to use a headset
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Switch {
    /// The headset is now the default output, under this name
    Switched(String),
    /// It already was
    Already,
    /// The sound system doesn't know the headset (yet)
    NotFound,
}

/// The computer's sound system (PipeWire on Linux)
pub trait AudioSystem: Send + Sync {
    /// Makes the headset's speakers the default output, and its mic the default input
    fn use_headset(&self, headset: UsbId) -> Result<Switch, String>;
}

/// Asks the sound system to use a headset, giving it a few tries: a headset's sound
/// card can show up a moment after the headset itself
pub fn use_headset_when_ready(
    audio: &dyn AudioSystem,
    headset: UsbId,
    tries: u32,
    wait: Duration,
) -> Result<Switch, String> {
    for _ in 1..tries {
        match audio.use_headset(headset)? {
            Switch::NotFound => std::thread::sleep(wait),
            done => return Ok(done),
        }
    }
    audio.use_headset(headset)
}

/// Says when to switch the sound over: whenever a headset turns on, if the user wants that
pub struct AudioSwitch {
    enabled: bool,
    /// Which headsets are on
    on: HashMap<DeviceId, bool>,
}

impl AudioSwitch {
    pub fn new(enabled: bool) -> Self {
        AudioSwitch { enabled, on: HashMap::new() }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Takes every device manager event; returns the headset to switch to now
    pub fn on_event(&mut self, event: &AppEvent) -> Option<UsbId> {
        match event {
            AppEvent::Added(info) => {
                self.on.insert(info.id.clone(), false);
                None
            }
            AppEvent::Removed(id) => {
                self.on.remove(id);
                None
            }
            AppEvent::Changed(id, Event::Connected(now_on)) => {
                let was_on = self.on.get_mut(id)?;
                let turned_on = *now_on && !*was_on;
                *was_on = *now_on;
                (turned_on && self.enabled).then_some(id.usb)
            }
            AppEvent::Changed(..) | AppEvent::Failed(..) | AppEvent::Problem(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{BackendKind, DeviceInfo};
    use std::sync::Mutex;

    fn device() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "a".into() },
            name: "Headset".into(),
            vendor: "V".into(),
            capabilities: vec![],
            equalizer: None,
        }
    }

    fn connected(on: bool) -> AppEvent {
        AppEvent::Changed(device().id, Event::Connected(on))
    }

    #[test]
    fn switches_each_time_the_headset_turns_on() {
        let mut switch = AudioSwitch::new(true);
        assert_eq!(switch.on_event(&AppEvent::Added(device())), None);
        assert_eq!(switch.on_event(&connected(true)), Some(UsbId::new(1, 2)));
        // still on: no second switch
        assert_eq!(switch.on_event(&connected(true)), None);
        assert_eq!(switch.on_event(&connected(false)), None);
        assert_eq!(switch.on_event(&connected(true)), Some(UsbId::new(1, 2)));
    }

    #[test]
    fn only_when_the_user_wants_it() {
        let mut switch = AudioSwitch::new(false);
        switch.on_event(&AppEvent::Added(device()));
        assert_eq!(switch.on_event(&connected(true)), None);
        switch.set_enabled(true);
        switch.on_event(&connected(false));
        assert_eq!(switch.on_event(&connected(true)), Some(UsbId::new(1, 2)));
    }

    #[test]
    fn ignores_headsets_it_was_not_told_about() {
        let mut switch = AudioSwitch::new(true);
        assert_eq!(switch.on_event(&connected(true)), None);
    }

    /// Doesn't know the headset for the first `missing` asks
    struct SlowAudio {
        missing: Mutex<u32>,
    }

    impl AudioSystem for SlowAudio {
        fn use_headset(&self, _: UsbId) -> Result<Switch, String> {
            let mut missing = self.missing.lock().unwrap();
            if *missing == 0 {
                return Ok(Switch::Switched("Headset".into()));
            }
            *missing -= 1;
            Ok(Switch::NotFound)
        }
    }

    #[test]
    fn waits_for_the_sound_card_to_show_up() {
        let audio = SlowAudio { missing: Mutex::new(2) };
        let result = use_headset_when_ready(&audio, UsbId::new(1, 2), 5, Duration::ZERO);
        assert_eq!(result, Ok(Switch::Switched("Headset".into())));
        let audio = SlowAudio { missing: Mutex::new(9) };
        assert_eq!(use_headset_when_ready(&audio, UsbId::new(1, 2), 5, Duration::ZERO), Ok(Switch::NotFound));
    }
}
