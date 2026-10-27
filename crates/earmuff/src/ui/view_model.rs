//! What the window shows, worked out from the device manager's events.
//! Plain Rust with no GTK, so all of the window's logic is unit-tested here.

use earmuff_application::AppEvent;
use earmuff_domain::{ChargeState, DeviceInfo, Status};

pub const LOOKING: &str = "Looking for a supported headset…";
pub const WAITING: &str = "Waiting for the headset…";
pub const AUTO_OFF_MINUTES: [u8; 6] = [0, 10, 20, 30, 60, 90];

/// The headsets the device manager reported, and what's stopping one from working
#[derive(Debug, Default)]
pub struct Model {
    devices: Vec<(DeviceInfo, Status)>,
    problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    /// No headset to show; the description says why
    Missing {
        description: String,
    },
    /// The dongle is there, the headset is off
    Off,
    Settings,
}

/// Everything the settings page shows, ready for the widgets
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceView {
    pub title: String,
    pub subtitle: String,
    /// Battery percentage for the level bar
    pub battery: Option<u8>,
    pub battery_text: String,
    pub charge_text: String,
    pub muted: bool,
    pub mic_text: String,
    pub sidetone: bool,
    /// Mic monitoring level in percent
    pub level: Option<u8>,
    pub auto_off: Option<u8>,
    pub voice_prompts: bool,
}

impl Model {
    /// Records an event; returns true if the window needs redrawing
    pub fn apply(&mut self, event: &AppEvent) -> bool {
        match event {
            AppEvent::Added(info) => {
                self.devices.retain(|(known, _)| known.id != info.id);
                self.devices.push((info.clone(), Status::default()));
                true
            }
            AppEvent::Changed(id, event) => {
                self.devices.iter_mut().find(|(info, _)| info.id == *id).is_some_and(|(_, status)| status.apply(event))
            }
            AppEvent::Removed(id) => {
                let before = self.devices.len();
                self.devices.retain(|(info, _)| info.id != *id);
                self.devices.len() != before
            }
            // shown as a toast by the window, nothing to redraw
            AppEvent::Failed(..) => false,
            AppEvent::Problem(problem) => {
                let changed = self.problem != *problem;
                self.problem = problem.clone();
                changed
            }
        }
    }

    /// The headset the window shows: the first one found
    pub fn current(&self) -> Option<&(DeviceInfo, Status)> {
        self.devices.first()
    }

    pub fn page(&self) -> Page {
        match self.current() {
            None => Page::Missing { description: self.problem.clone().unwrap_or_else(|| LOOKING.into()) },
            Some((_, status)) => match status.connected {
                None => Page::Missing { description: WAITING.into() },
                Some(false) => Page::Off,
                Some(true) => Page::Settings,
            },
        }
    }

    pub fn title(&self) -> String {
        self.current().map_or_else(|| "Earmuff".into(), |(info, _)| info.name.clone())
    }

    pub fn view(&self) -> Option<DeviceView> {
        let (info, s) = self.current()?;
        Some(DeviceView {
            title: info.name.clone(),
            subtitle: s.color.clone().unwrap_or_default(),
            battery: s.battery.map(|b| b.get()),
            battery_text: s.battery.map(|b| b.to_string()).unwrap_or_default(),
            charge_text: match s.charge {
                Some(ChargeState::Discharging) => "On battery",
                Some(ChargeState::Charging) => "Charging",
                Some(ChargeState::Full) => "Fully charged",
                Some(ChargeState::Unknown) | None => "",
            }
            .into(),
            muted: s.mic_muted.unwrap_or(false),
            mic_text: match s.mic_attached {
                Some(true) => "Attached",
                Some(false) => "Not attached",
                None => "",
            }
            .into(),
            sidetone: s.sidetone.unwrap_or(false),
            level: s.sidetone_level.map(|l| l.get()),
            auto_off: s.auto_off.map(|a| a.get()),
            voice_prompts: s.voice_prompts.unwrap_or(false),
        })
    }
}

pub fn minutes_text(minutes: u8) -> String {
    match minutes {
        0 => "Never".into(),
        m if m % 60 != 0 => format!("{m} minutes"),
        60 => "1 hour".into(),
        m => format!("{} hours", m / 60),
    }
}

/// The auto-off choices: the usual ones, plus the headset's current value if it's
/// something else (e.g. set from the command line)
pub fn auto_off_choices(current: Option<u8>) -> Vec<u8> {
    let mut choices = AUTO_OFF_MINUTES.to_vec();
    if let Some(minutes) = current
        && !choices.contains(&minutes)
    {
        choices.push(minutes);
        choices.sort_unstable();
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{AutoOff, BackendKind, Capability, DeviceId, Event, Percent, UsbId};

    fn info(instance: &str) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(0x03F0, 0x098D), instance: instance.into() },
            name: format!("Headset {instance}"),
            vendor: "HyperX".into(),
            capabilities: vec![Capability::Battery],
            equalizer: None,
        }
    }

    fn changed(instance: &str, event: Event) -> AppEvent {
        AppEvent::Changed(info(instance).id, event)
    }

    #[test]
    fn looks_for_a_headset_until_one_is_added() {
        let model = Model::default();
        assert_eq!(model.page(), Page::Missing { description: LOOKING.into() });
        assert_eq!(model.title(), "Earmuff");
        assert_eq!(model.view(), None);
    }

    #[test]
    fn explains_a_problem_while_no_headset_works() {
        let mut model = Model::default();
        assert!(model.apply(&AppEvent::Problem(Some("no permission".into()))));
        assert!(!model.apply(&AppEvent::Problem(Some("no permission".into()))));
        assert_eq!(model.page(), Page::Missing { description: "no permission".into() });
        model.apply(&AppEvent::Problem(None));
        assert_eq!(model.page(), Page::Missing { description: LOOKING.into() });
    }

    #[test]
    fn follows_the_connection() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(info("a")));
        assert_eq!(model.page(), Page::Missing { description: WAITING.into() });
        assert_eq!(model.title(), "Headset a");
        model.apply(&changed("a", Event::Connected(false)));
        assert_eq!(model.page(), Page::Off);
        model.apply(&changed("a", Event::Connected(true)));
        assert_eq!(model.page(), Page::Settings);
        assert!(model.apply(&AppEvent::Removed(info("a").id)));
        assert_eq!(model.page(), Page::Missing { description: LOOKING.into() });
    }

    #[test]
    fn redraws_only_on_real_changes() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(info("a")));
        assert!(model.apply(&changed("a", Event::MicMuted(true))));
        assert!(!model.apply(&changed("a", Event::MicMuted(true))));
        assert!(!model.apply(&changed("other", Event::MicMuted(false))));
        assert!(!model.apply(&AppEvent::Removed(info("other").id)));
    }

    #[test]
    fn re_adding_a_headset_starts_it_fresh() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(info("a")));
        model.apply(&changed("a", Event::Connected(true)));
        model.apply(&AppEvent::Added(info("a")));
        assert_eq!(model.page(), Page::Missing { description: WAITING.into() });
    }

    #[test]
    fn shows_the_first_headset() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(info("a")));
        model.apply(&AppEvent::Added(info("b")));
        assert_eq!(model.title(), "Headset a");
    }

    #[test]
    fn builds_the_settings_view() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(info("a")));
        for event in [
            Event::Connected(true),
            Event::Battery(Percent::saturating(99)),
            Event::Charge(ChargeState::Charging),
            Event::MicMuted(true),
            Event::MicAttached(false),
            Event::Sidetone(true),
            Event::SidetoneLevel(Percent::saturating(50)),
            Event::AutoOff(AutoOff::minutes(20)),
            Event::VoicePrompts(true),
            Event::Color("Black/Red".into()),
        ] {
            model.apply(&changed("a", event));
        }
        assert_eq!(
            model.view(),
            Some(DeviceView {
                title: "Headset a".into(),
                subtitle: "Black/Red".into(),
                battery: Some(99),
                battery_text: "99%".into(),
                charge_text: "Charging".into(),
                muted: true,
                mic_text: "Not attached".into(),
                sidetone: true,
                level: Some(50),
                auto_off: Some(20),
                voice_prompts: true,
            })
        );
    }

    #[test]
    fn unknown_values_show_as_empty_or_off() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(info("a")));
        let view = model.view().unwrap();
        assert_eq!((view.battery, view.battery_text.as_str(), view.charge_text.as_str()), (None, "", ""));
        assert!(!view.muted && !view.sidetone && !view.voice_prompts);
    }

    #[test]
    fn minutes_read_naturally() {
        assert_eq!(minutes_text(0), "Never");
        assert_eq!(minutes_text(20), "20 minutes");
        assert_eq!(minutes_text(60), "1 hour");
        assert_eq!(minutes_text(90), "90 minutes");
        assert_eq!(minutes_text(120), "2 hours");
    }

    #[test]
    fn auto_off_choices_include_the_current_value() {
        assert_eq!(auto_off_choices(None), AUTO_OFF_MINUTES);
        assert_eq!(auto_off_choices(Some(20)), AUTO_OFF_MINUTES);
        assert_eq!(auto_off_choices(Some(45)), [0, 10, 20, 30, 45, 60, 90]);
    }
}
