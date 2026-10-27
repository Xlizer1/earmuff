//! GTK4/libadwaita window.

use std::cell::{Cell, RefCell};
use std::io;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::gio;
use gtk::glib::{self, ControlFlow, IOCondition, SourceId};
use ksni::TrayMethods;

use crate::device::{Headset, OpenError};
use crate::protocol::{self as p, cmd, ChargeState, Update};
use crate::tray::{HeadsetTray, TrayEvent};

const APP_ID: &str = "io.github.xlizer1.Earmuff";
const AUTO_OFF_MINUTES: [u8; 6] = [0, 10, 20, 30, 60, 90];

pub fn run() -> ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| match app.active_window() {
        Some(window) => window.present(),
        None => Ui::new(app).window.present(),
    });
    // clap already parsed the command line, so GTK gets no arguments
    app.run_with_args::<&str>(&[]).into()
}

fn minutes_text(minutes: u8) -> String {
    match minutes {
        0 => "Never".into(),
        m if m % 60 != 0 => format!("{m} minutes"),
        60 => "1 hour".into(),
        m => format!("{} hours", m / 60),
    }
}

fn group(title: &str, description: Option<&str>, rows: &[&gtk::Widget]) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    group.set_description(description);
    for row in rows {
        group.add(*row);
    }
    group
}

struct Ui {
    app: adw::Application,
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    stack: gtk::Stack,
    missing_page: adw::StatusPage,
    battery_row: adw::ActionRow,
    battery_bar: gtk::LevelBar,
    battery_label: gtk::Label,
    mute_row: adw::SwitchRow,
    mic_row: adw::ActionRow,
    sidetone_row: adw::SwitchRow,
    level_row: adw::ActionRow,
    level_scale: gtk::Scale,
    auto_off_row: adw::ComboRow,
    auto_off_model: gtk::StringList,
    auto_off_values: RefCell<Vec<u8>>,
    voice_row: adw::SwitchRow,

    headset: RefCell<Option<Headset>>,
    /// True while widgets are being updated from the headset
    syncing: Cell<bool>,
    fd_watch: RefCell<Option<SourceId>>,
    pump_timer: RefCell<Option<SourceId>>,
    retry_timer: RefCell<Option<SourceId>>,
    level_timer: RefCell<Option<SourceId>>,

    tray: RefCell<Option<ksni::Handle<HeadsetTray>>>,
    /// While a tray shows the icon, closing the window only hides it
    tray_online: Cell<bool>,
    /// Keeps the app running while the window is hidden
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

impl Ui {
    fn new(app: &adw::Application) -> Rc<Ui> {
        let title = adw::WindowTitle::new("Cloud Alpha Wireless", "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();

        let missing_page = adw::StatusPage::builder()
            .icon_name("audio-headset-symbolic")
            .title("Plug in the USB dongle")
            .build();
        let off_page = adw::StatusPage::builder()
            .icon_name("audio-headset-symbolic")
            .title("Headset is off")
            .description("Turn it on with the power button.")
            .build();

        let battery_bar = gtk::LevelBar::builder()
            .min_value(0.0)
            .max_value(100.0)
            .valign(gtk::Align::Center)
            .width_request(120)
            .build();
        let battery_label = gtk::Label::builder().width_chars(4).xalign(1.0).build();
        let battery_row = adw::ActionRow::builder().title("Battery").build();
        battery_row.add_suffix(&battery_bar);
        battery_row.add_suffix(&battery_label);
        let battery = adw::PreferencesGroup::new();
        battery.add(&battery_row);

        let mute_row = adw::SwitchRow::builder()
            .title("Mute")
            .subtitle("Same as the mute button on the headset")
            .build();
        let mic_row = adw::ActionRow::builder().title("Mic boom").build();
        let mic = group("Microphone", None, &[mute_row.upcast_ref(), mic_row.upcast_ref()]);

        let sidetone_row = adw::SwitchRow::builder().title("Mic monitoring").build();
        let (min, max) = (p::SIDETONE_MIN.into(), p::SIDETONE_MAX.into());
        let level_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 1.0);
        level_scale.set_hexpand(true);
        level_scale.set_round_digits(0);
        level_scale.add_mark(min, gtk::PositionType::Bottom, Some("Quiet"));
        level_scale.add_mark(max, gtk::PositionType::Bottom, Some("Loud"));
        let level_row = adw::ActionRow::builder().title("Level").build();
        level_row.add_suffix(&level_scale);
        let monitoring = group(
            "Mic monitoring",
            Some("Hear your own voice in the headset while you talk"),
            &[sidetone_row.upcast_ref(), level_row.upcast_ref()],
        );

        let auto_off_model = gtk::StringList::new(&[]);
        for minutes in AUTO_OFF_MINUTES {
            auto_off_model.append(&minutes_text(minutes));
        }
        let auto_off_row = adw::ComboRow::builder()
            .title("Turn off when not in use")
            .model(&auto_off_model)
            .build();
        let voice_row = adw::SwitchRow::builder()
            .title("Voice prompts")
            .subtitle("Spoken alerts from the headset")
            .build();
        let power = group("Power and sounds", None, &[auto_off_row.upcast_ref(), voice_row.upcast_ref()]);

        let page = adw::PreferencesPage::new();
        for g in [&battery, &mic, &monitoring, &power] {
            page.add(g);
        }

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&missing_page, Some("missing"));
        stack.add_named(&off_page, Some("off"));
        stack.add_named(&page, Some("settings"));

        let view = adw::ToolbarView::builder().content(&stack).build();
        view.add_top_bar(&header);
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Earmuff")
            .default_width(440)
            .default_height(620)
            .content(&view)
            .build();

        let ui = Rc::new(Ui {
            app: app.clone(),
            window,
            title,
            stack,
            missing_page,
            battery_row,
            battery_bar,
            battery_label,
            mute_row,
            mic_row,
            sidetone_row,
            level_row,
            level_scale,
            auto_off_row,
            auto_off_model,
            auto_off_values: RefCell::new(AUTO_OFF_MINUTES.to_vec()),
            voice_row,
            headset: RefCell::new(None),
            syncing: Cell::new(false),
            fd_watch: RefCell::new(None),
            pump_timer: RefCell::new(None),
            retry_timer: RefCell::new(None),
            level_timer: RefCell::new(None),
            tray: RefCell::new(None),
            tray_online: Cell::new(false),
            hold: RefCell::new(None),
        });
        ui.connect_signals();
        ui.start_tray();

        // this handler's strong reference keeps Ui alive as long as the window
        let keep = ui.clone();
        ui.window.connect_close_request(move |window| {
            if keep.tray_online.get() {
                // like Discord: X hides the window, Quit in the tray menu exits
                window.set_visible(false);
                return glib::Propagation::Stop;
            }
            keep.quit();
            glib::Propagation::Proceed
        });

        let weak = Rc::downgrade(&ui);
        glib::timeout_add_seconds_local(60, move || {
            let Some(ui) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            ui.send(|h| h.request([p::get(cmd::GET_BATTERY), p::get(cmd::GET_CHARGING)]));
            ControlFlow::Continue
        });

        if !ui.try_connect() {
            ui.start_retry();
        }
        ui
    }

    // --- widgets -> headset ----------------------------------------------

    fn connect_signals(self: &Rc<Self>) {
        self.on_switch(&self.mute_row, Headset::set_mute);
        self.on_switch(&self.sidetone_row, Headset::set_sidetone);
        self.on_switch(&self.voice_row, Headset::set_voice_prompts);

        let weak = Rc::downgrade(self);
        self.auto_off_row.connect_selected_notify(move |row| {
            let Some(ui) = weak.upgrade() else { return };
            if ui.syncing.get() {
                return;
            }
            let minutes = ui.auto_off_values.borrow().get(row.selected() as usize).copied();
            if let Some(minutes) = minutes {
                ui.send(|h| h.set_auto_off(minutes));
            }
        });

        let weak = Rc::downgrade(self);
        self.level_scale.connect_value_changed(move |_| {
            if let Some(ui) = weak.upgrade()
                && !ui.syncing.get()
            {
                ui.schedule_level();
            }
        });
    }

    fn on_switch(self: &Rc<Self>, row: &adw::SwitchRow, action: fn(&mut Headset, bool) -> io::Result<()>) {
        let weak = Rc::downgrade(self);
        row.connect_active_notify(move |row| {
            if let Some(ui) = weak.upgrade()
                && !ui.syncing.get()
            {
                let on = row.is_active();
                ui.send(|h| action(h, on));
            }
        });
    }

    /// Waits until the slider stops for a moment instead of sending every step of a drag
    fn schedule_level(self: &Rc<Self>) {
        if let Some(id) = self.level_timer.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(150), move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.level_timer.take();
            let level = ui.level_scale.value().round() as i16;
            ui.send(|h| h.set_sidetone_level(level));
        });
        self.level_timer.replace(Some(id));
    }

    // --- tray ------------------------------------------------------------

    fn start_tray(self: &Rc<Self>) {
        let (sender, receiver) = async_channel::unbounded();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let handle = match HeadsetTray::new(sender).spawn().await {
                Ok(handle) => handle,
                Err(e) => {
                    // no tray on this desktop: the window's X quits the app instead
                    eprintln!("earmuff: no system tray ({e})");
                    return;
                }
            };
            if let Some(ui) = weak.upgrade() {
                ui.hold.replace(Some(ui.app.hold()));
                ui.tray_online.set(true);
                ui.tray.replace(Some(handle));
                ui.update_tray();
            }
            while let Ok(event) = receiver.recv().await {
                let Some(ui) = weak.upgrade() else { break };
                match event {
                    TrayEvent::Show => ui.window.present(),
                    TrayEvent::SetMute(on) => ui.send(|h| h.set_mute(on)),
                    TrayEvent::Online(online) => ui.tray_online.set(online),
                    TrayEvent::Quit => ui.quit(),
                }
            }
        });
    }

    /// Shows the current battery and mute state in the tray
    fn update_tray(&self) {
        let Some(handle) = self.tray.borrow().clone() else {
            return;
        };
        let state = self.headset.borrow().as_ref().map(|h| h.state);
        glib::spawn_future_local(async move {
            handle.update(move |tray| tray.state = state).await;
        });
    }

    fn quit(&self) {
        self.disconnect_headset();
        self.hold.take();
        self.app.quit();
    }

    // --- connection ------------------------------------------------------

    /// Opens the dongle; returns false when it isn't there
    fn try_connect(self: &Rc<Self>) -> bool {
        let headset = match Headset::open() {
            Ok(headset) => headset,
            Err(OpenError::NotFound) => {
                self.missing_page.set_description(Some("Looking for the HyperX Cloud Alpha Wireless…"));
                return false;
            }
            Err(e) => {
                self.missing_page.set_description(Some(&e.to_string()));
                return false;
            }
        };
        let weak = Rc::downgrade(self);
        let condition = IOCondition::IN | IOCondition::HUP | IOCondition::ERR;
        let watch = glib_unix::unix_fd_add_local(headset.fd(), condition, move |_, condition| {
            match weak.upgrade() {
                Some(ui) => ui.on_readable(condition),
                None => ControlFlow::Break,
            }
        });
        self.headset.replace(Some(headset));
        self.fd_watch.replace(Some(watch));
        self.missing_page.set_description(Some("Waiting for the headset…"));
        self.send(Headset::refresh);
        self.update_tray();
        true
    }

    fn start_retry(self: &Rc<Self>) {
        if self.retry_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_seconds_local(2, move || {
            let Some(ui) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            if ui.try_connect() {
                ui.retry_timer.take();
                ControlFlow::Break
            } else {
                ControlFlow::Continue
            }
        });
        self.retry_timer.replace(Some(id));
    }

    fn lost(self: &Rc<Self>) {
        self.disconnect_headset();
        self.stack.set_visible_child_name("missing");
        self.update_tray();
        self.start_retry();
    }

    fn disconnect_headset(&self) {
        for source in [&self.fd_watch, &self.pump_timer, &self.level_timer] {
            if let Some(id) = source.take() {
                id.remove();
            }
        }
        self.headset.take(); // closes the dongle
    }

    /// Runs `action` on the open headset; an error means the dongle is gone
    fn with_headset<T>(&self, action: impl FnOnce(&mut Headset) -> io::Result<T>) -> io::Result<T> {
        match self.headset.borrow_mut().as_mut() {
            Some(headset) => action(headset),
            None => Err(io::ErrorKind::NotConnected.into()),
        }
    }

    /// Runs a Headset method that queues commands, and keeps the queue moving
    fn send(self: &Rc<Self>, action: impl FnOnce(&mut Headset) -> io::Result<()>) {
        if self.headset.borrow().is_none() {
            return;
        }
        if self.with_headset(action).is_err() {
            self.lost();
            return;
        }
        if self.pump_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(ui) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            match ui.with_headset(Headset::pump) {
                Ok(true) => ControlFlow::Continue,
                Ok(false) => {
                    ui.pump_timer.take();
                    ControlFlow::Break
                }
                Err(_) => {
                    ui.pump_timer.take();
                    ui.lost();
                    ControlFlow::Break
                }
            }
        });
        self.pump_timer.replace(Some(id));
    }

    fn on_readable(self: &Rc<Self>, condition: IOCondition) -> ControlFlow {
        let result = if condition.intersects(IOCondition::HUP | IOCondition::ERR) {
            Err(io::ErrorKind::NotConnected.into())
        } else {
            self.with_headset(Headset::read)
        };
        let Ok(changed) = result else {
            self.fd_watch.take();
            self.lost();
            return ControlFlow::Break;
        };
        if changed.contains(&Update::Connected(true)) {
            self.send(Headset::refresh); // settings may have changed while it was off
        }
        if !changed.is_empty() {
            self.render();
            self.update_tray();
        }
        ControlFlow::Continue
    }

    // --- headset -> widgets ----------------------------------------------

    fn render(&self) {
        let Some(s) = self.headset.borrow().as_ref().map(|h| h.state) else {
            return;
        };
        let Some(connected) = s.connected else { return };
        if !connected {
            self.stack.set_visible_child_name("off");
            return;
        }
        self.stack.set_visible_child_name("settings");
        self.syncing.set(true);
        self.title.set_subtitle(&s.color.map(|c| c.to_string()).unwrap_or_default());
        if let Some(level) = s.battery {
            self.battery_bar.set_value(level.into());
            self.battery_label.set_label(&format!("{level}%"));
        }
        self.battery_row.set_subtitle(match s.charging {
            Some(ChargeState::Discharging) => "On battery",
            Some(ChargeState::Charging) => "Charging",
            Some(ChargeState::Full) => "Fully charged",
            _ => "",
        });
        self.mute_row.set_active(s.muted.unwrap_or(false));
        self.mic_row.set_subtitle(match s.mic_attached {
            Some(true) => "Attached",
            Some(false) => "Not attached",
            None => "",
        });
        let sidetone = s.sidetone.unwrap_or(false);
        self.sidetone_row.set_active(sidetone);
        self.level_row.set_sensitive(sidetone);
        if let Some(level) = s.sidetone_level
            && self.level_timer.borrow().is_none()
        {
            self.level_scale.set_value(level.into());
        }
        if let Some(minutes) = s.auto_off {
            self.select_auto_off(minutes);
        }
        self.voice_row.set_active(s.voice_prompts.unwrap_or(false));
        self.syncing.set(false);
    }

    fn select_auto_off(&self, minutes: u8) {
        let position = self.auto_off_values.borrow().iter().position(|&m| m == minutes);
        let index = position.unwrap_or_else(|| {
            // a value set elsewhere (e.g. from the command line): add it to the list
            let index = {
                let mut values = self.auto_off_values.borrow_mut();
                let index = values.partition_point(|&m| m < minutes);
                values.insert(index, minutes);
                index
            };
            self.auto_off_model.splice(index as u32, 0, &[&minutes_text(minutes)]);
            index
        });
        self.auto_off_row.set_selected(index as u32);
    }
}
