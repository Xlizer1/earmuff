//! Icon in the system tray (StatusNotifierItem): Waybar's tray, KDE, and GNOME with
//! the AppIndicator extension. ksni runs it on its own thread; clicks come back to
//! the GTK thread as [`TrayEvent`]s.

use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, OfflineReason, ToolTip, Tray};

use crate::ui::view_model::Model;
use earmuff_domain::ChargeState;

pub const TRAY_ID: &str = "earmuff";

/// What the user did in the tray, handled on the GTK thread
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    Show,
    SetMute(bool),
    /// The tray host left (e.g. Waybar restarting) or came back
    Online(bool),
    Quit,
}

/// What the tray shows
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayState {
    pub title: String,
    pub status_line: String,
    /// `None` when there's no headset whose mic can be muted right now
    pub muted: Option<bool>,
}

impl TrayState {
    pub fn from_model(model: &Model) -> Self {
        let Some((info, s)) = model.current() else {
            return TrayState { title: "Earmuff".into(), status_line: "No headset found".into(), muted: None };
        };
        let status_line = match (s.connected, s.battery) {
            (None, _) => "Connecting…".into(),
            (Some(false), _) => "Headset is off".into(),
            (Some(true), None) => "Battery …".into(),
            (Some(true), Some(level)) => match s.charge {
                Some(ChargeState::Charging) => format!("Battery {level}, charging"),
                Some(ChargeState::Full) => format!("Battery {level}, full"),
                _ => format!("Battery {level}"),
            },
        };
        let muted = if s.is_connected() { s.mic_muted } else { None };
        TrayState { title: info.name.clone(), status_line, muted }
    }
}

impl Default for TrayState {
    fn default() -> Self {
        TrayState::from_model(&Model::default())
    }
}

#[derive(Debug)]
pub struct HeadsetTray {
    pub state: TrayState,
    events: async_channel::Sender<TrayEvent>,
    /// The icon as pixels, for trays that can't find it in the icon theme
    pixmap: Vec<ksni::Icon>,
}

impl HeadsetTray {
    pub fn new(events: async_channel::Sender<TrayEvent>, pixmap: Vec<ksni::Icon>) -> Self {
        HeadsetTray { state: TrayState::default(), events, pixmap }
    }

    fn send(&self, event: TrayEvent) {
        // only fails once the app is quitting
        let _ = self.events.try_send(event);
    }
}

impl Tray for HeadsetTray {
    fn id(&self) -> String {
        TRAY_ID.into()
    }

    fn title(&self) -> String {
        "Earmuff".into()
    }

    fn icon_name(&self) -> String {
        crate::ui::APP_ID.into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmap.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: self.state.title.clone(), description: self.state.status_line.clone(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let muted = self.state.muted;
        vec![
            StandardItem { label: self.state.status_line.clone(), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Mute microphone".into(),
                checked: muted.unwrap_or(false),
                enabled: muted.is_some(),
                activate: Box::new(move |tray: &mut Self| tray.send(TrayEvent::SetMute(!muted.unwrap_or(false)))),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open Earmuff".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayEvent::Show)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayEvent::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_online(&self) {
        self.send(TrayEvent::Online(true));
    }

    fn watcher_offline(&self, _reason: OfflineReason) -> bool {
        self.send(TrayEvent::Online(false));
        true // keep running: ksni registers again when the host comes back
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_application::AppEvent;
    use earmuff_domain::{BackendKind, DeviceId, DeviceInfo, Event, Percent, UsbId};

    fn model_with(events: &[Event]) -> Model {
        let id = DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "a".into() };
        let mut model = Model::default();
        model.apply(&AppEvent::Added(DeviceInfo {
            id: id.clone(),
            name: "Cloud Alpha Wireless".into(),
            vendor: "HyperX".into(),
            capabilities: vec![],
            equalizer: None,
        }));
        for event in events {
            model.apply(&AppEvent::Changed(id.clone(), event.clone()));
        }
        model
    }

    #[test]
    fn says_when_there_is_no_headset() {
        let state = TrayState::default();
        assert_eq!(state.status_line, "No headset found");
        assert_eq!(state.muted, None);
    }

    #[test]
    fn status_line_follows_the_headset() {
        assert_eq!(TrayState::from_model(&model_with(&[])).status_line, "Connecting…");
        assert_eq!(TrayState::from_model(&model_with(&[Event::Connected(false)])).status_line, "Headset is off");
        let battery = Event::Battery(Percent::saturating(80));
        assert_eq!(
            TrayState::from_model(&model_with(&[Event::Connected(true), battery.clone()])).status_line,
            "Battery 80%"
        );
        let charging = Event::Charge(ChargeState::Charging);
        assert_eq!(
            TrayState::from_model(&model_with(&[Event::Connected(true), battery, charging])).status_line,
            "Battery 80%, charging"
        );
    }

    #[test]
    fn mute_is_offered_only_while_the_headset_is_on() {
        let on = model_with(&[Event::Connected(true), Event::MicMuted(true)]);
        assert_eq!(TrayState::from_model(&on).muted, Some(true));
        let off = model_with(&[Event::Connected(false), Event::MicMuted(true)]);
        assert_eq!(TrayState::from_model(&off).muted, None);
        assert_eq!(TrayState::from_model(&on).title, "Cloud Alpha Wireless");
    }

    #[test]
    fn menu_mutes_and_quits() {
        let (sender, receiver) = async_channel::unbounded();
        let mut tray = HeadsetTray::new(sender, Vec::new());
        tray.state = TrayState::from_model(&model_with(&[Event::Connected(true), Event::MicMuted(false)]));
        let menu = tray.menu();
        let MenuItem::Checkmark(mute) = &menu[2] else { panic!("third item is the mute toggle") };
        assert!(mute.enabled && !mute.checked);
        (mute.activate)(&mut tray);
        let MenuItem::Standard(quit) = &menu[5] else { panic!("last item is Quit") };
        (quit.activate)(&mut tray);
        tray.activate(0, 0);
        assert_eq!(receiver.try_recv(), Ok(TrayEvent::SetMute(true)));
        assert_eq!(receiver.try_recv(), Ok(TrayEvent::Quit));
        assert_eq!(receiver.try_recv(), Ok(TrayEvent::Show));
    }
}
