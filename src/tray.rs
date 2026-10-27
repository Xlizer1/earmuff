//! Icon in the system tray (StatusNotifierItem): Waybar's tray, KDE, and GNOME
//! with the AppIndicator extension. ksni runs it on its own thread; clicks come
//! back to the GTK thread as `TrayEvent`s.

use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, OfflineReason, ToolTip, Tray};

use crate::protocol::{ChargeState, State};

/// What the user did in the tray, handled on the GTK thread
pub enum TrayEvent {
    Show,
    SetMute(bool),
    /// The tray host left (e.g. Waybar restarting) or came back
    Online(bool),
    Quit,
}

pub struct HeadsetTray {
    /// None while the USB dongle isn't plugged in
    pub state: Option<State>,
    events: async_channel::Sender<TrayEvent>,
}

impl HeadsetTray {
    pub fn new(events: async_channel::Sender<TrayEvent>) -> Self {
        HeadsetTray { state: None, events }
    }

    fn send(&self, event: TrayEvent) {
        // only fails once the app is quitting
        let _ = self.events.try_send(event);
    }

    fn connected(&self) -> bool {
        matches!(self.state, Some(State { connected: Some(true), .. }))
    }

    fn status_line(&self) -> String {
        let Some(s) = self.state else {
            return "USB dongle not plugged in".into();
        };
        match (s.connected, s.battery) {
            (None, _) => "Connecting…".into(),
            (Some(false), _) => "Headset is off".into(),
            (Some(true), None) => "Battery …".into(),
            (Some(true), Some(level)) => match s.charging {
                Some(ChargeState::Charging) => format!("Battery {level}%, charging"),
                Some(ChargeState::Full) => format!("Battery {level}%, full"),
                _ => format!("Battery {level}%"),
            },
        }
    }
}

impl Tray for HeadsetTray {
    fn id(&self) -> String {
        "earmuff".into()
    }

    fn title(&self) -> String {
        "Earmuff".into()
    }

    fn icon_name(&self) -> String {
        "audio-headset".into()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Cloud Alpha Wireless".into(),
            description: self.status_line(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let muted = self.state.and_then(|s| s.muted).unwrap_or(false);
        vec![
            StandardItem { label: self.status_line(), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Mute microphone".into(),
                checked: muted,
                enabled: self.connected(),
                activate: Box::new(move |tray: &mut Self| tray.send(TrayEvent::SetMute(!muted))),
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
