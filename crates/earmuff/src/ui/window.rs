//! The main window. It shows what the [`Model`] says and sends the user's changes to
//! the device manager, which talks to the headset on its own thread.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use earmuff_application::{AppEvent, Backend, DeviceManager};
use earmuff_domain::{AutoOff, Percent, Setting};
use gtk::gio;
use gtk::glib::{self, SourceId};
use ksni::TrayMethods;

use super::UiOptions;
use super::view_model::{Model, Page, auto_off_choices, minutes_text};
use crate::tray::{HeadsetTray, TrayEvent, TrayState};

/// Builds the window, starts looking for headsets and shows it
pub(super) fn open(app: &adw::Application, backends: Vec<Arc<dyn Backend>>, options: &UiOptions) {
    let ui = Ui::new(app);
    ui.connect_signals();
    if options.tray {
        ui.start_tray();
    }
    ui.start_manager(backends, options);
    ui.render();
    ui.window.present();
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

    model: RefCell<Model>,
    manager: RefCell<Option<DeviceManager>>,
    /// True while widgets are being updated from the headset, so their signals are ignored
    syncing: Cell<bool>,
    level_timer: RefCell<Option<SourceId>>,
    tray: RefCell<Option<ksni::Handle<HeadsetTray>>>,
    /// While a tray shows the icon, closing the window only hides it
    tray_online: Cell<bool>,
    /// Keeps the app running while the window is hidden
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

impl Ui {
    fn new(app: &adw::Application) -> Rc<Ui> {
        let title = adw::WindowTitle::new("Earmuff", "");
        title.set_widget_name("title");
        let header = adw::HeaderBar::builder().title_widget(&title).build();

        let missing_page = adw::StatusPage::builder()
            .name("missing-page")
            .icon_name("audio-headset-symbolic")
            .title("Plug in your headset's USB dongle")
            .build();
        let off_page = adw::StatusPage::builder()
            .name("off-page")
            .icon_name("audio-headset-symbolic")
            .title("Headset is off")
            .description("Turn it on with the power button.")
            .build();

        let battery_bar = gtk::LevelBar::builder()
            .name("battery-bar")
            .min_value(0.0)
            .max_value(100.0)
            .valign(gtk::Align::Center)
            .width_request(120)
            .build();
        let battery_label = gtk::Label::builder().name("battery-label").width_chars(4).xalign(1.0).build();
        let battery_row = adw::ActionRow::builder().name("battery").title("Battery").build();
        battery_row.add_suffix(&battery_bar);
        battery_row.add_suffix(&battery_label);
        let battery = adw::PreferencesGroup::new();
        battery.add(&battery_row);

        let mute_row = adw::SwitchRow::builder()
            .name("mute")
            .title("Mute")
            .subtitle("Same as the mute button on the headset")
            .build();
        let mic_row = adw::ActionRow::builder().name("mic").title("Mic boom").build();
        let mic = group("Microphone", None, &[mute_row.upcast_ref(), mic_row.upcast_ref()]);

        let sidetone_row = adw::SwitchRow::builder().name("sidetone").title("Mic monitoring").build();
        let level_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        level_scale.set_widget_name("level");
        level_scale.set_hexpand(true);
        level_scale.set_round_digits(0);
        level_scale.add_mark(0.0, gtk::PositionType::Bottom, Some("Quiet"));
        level_scale.add_mark(100.0, gtk::PositionType::Bottom, Some("Loud"));
        let level_row = adw::ActionRow::builder().name("level-row").title("Level").build();
        level_row.add_suffix(&level_scale);
        let monitoring = group(
            "Mic monitoring",
            Some("Hear your own voice in the headset while you talk"),
            &[sidetone_row.upcast_ref(), level_row.upcast_ref()],
        );

        let auto_off_values = auto_off_choices(None);
        let auto_off_model = gtk::StringList::new(&[]);
        for minutes in &auto_off_values {
            auto_off_model.append(&minutes_text(*minutes));
        }
        let auto_off_row =
            adw::ComboRow::builder().name("auto-off").title("Turn off when not in use").model(&auto_off_model).build();
        let voice_row = adw::SwitchRow::builder()
            .name("voice-prompts")
            .title("Voice prompts")
            .subtitle("Spoken alerts from the headset")
            .build();
        let power = group("Power and sounds", None, &[auto_off_row.upcast_ref(), voice_row.upcast_ref()]);

        let page = adw::PreferencesPage::builder().name("settings-page").build();
        for g in [&battery, &mic, &monitoring, &power] {
            page.add(g);
        }

        let stack = gtk::Stack::builder().name("pages").transition_type(gtk::StackTransitionType::Crossfade).build();
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

        Rc::new(Ui {
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
            auto_off_values: RefCell::new(auto_off_values),
            voice_row,
            model: RefCell::new(Model::default()),
            manager: RefCell::new(None),
            syncing: Cell::new(false),
            level_timer: RefCell::new(None),
            tray: RefCell::new(None),
            tray_online: Cell::new(false),
            hold: RefCell::new(None),
        })
    }

    // --- widgets -> headset ----------------------------------------------

    fn connect_signals(self: &Rc<Self>) {
        self.on_switch(&self.mute_row, Setting::MicMuted);
        self.on_switch(&self.sidetone_row, Setting::Sidetone);
        self.on_switch(&self.voice_row, Setting::VoicePrompts);

        let weak = Rc::downgrade(self);
        self.auto_off_row.connect_selected_notify(move |row| {
            let Some(ui) = weak.upgrade() else { return };
            if ui.syncing.get() {
                return;
            }
            let minutes = ui.auto_off_values.borrow().get(row.selected() as usize).copied();
            if let Some(minutes) = minutes {
                ui.send(Setting::AutoOff(AutoOff::minutes(minutes)));
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

        // this handler's strong reference keeps Ui alive as long as the window
        let keep = self.clone();
        self.window.connect_close_request(move |window| {
            if keep.tray_online.get() {
                // like Discord: X hides the window, Quit in the tray menu exits
                window.set_visible(false);
                return glib::Propagation::Stop;
            }
            keep.quit();
            glib::Propagation::Proceed
        });
    }

    fn on_switch(self: &Rc<Self>, row: &adw::SwitchRow, setting: fn(bool) -> Setting) {
        let weak = Rc::downgrade(self);
        row.connect_active_notify(move |row| {
            if let Some(ui) = weak.upgrade()
                && !ui.syncing.get()
            {
                ui.send(setting(row.is_active()));
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
            let level = Percent::saturating(ui.level_scale.value().round() as i64);
            ui.send(Setting::SidetoneLevel(level));
        });
        self.level_timer.replace(Some(id));
    }

    /// Sends a setting to the headset the window shows
    fn send(&self, setting: Setting) {
        let id = self.model.borrow().current().map(|(info, _)| info.id.clone());
        if let (Some(id), Some(manager)) = (id, self.manager.borrow().as_ref()) {
            manager.apply(&id, setting);
        }
    }

    // --- headset -> widgets ----------------------------------------------

    fn start_manager(self: &Rc<Self>, backends: Vec<Arc<dyn Backend>>, options: &UiOptions) {
        let (sender, receiver) = async_channel::unbounded::<AppEvent>();
        let manager = DeviceManager::start(backends, options.manager, move |event| {
            let _ = sender.send_blocking(event);
        });
        self.manager.replace(Some(manager));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = receiver.recv().await {
                let Some(ui) = weak.upgrade() else { break };
                let changed = ui.model.borrow_mut().apply(&event);
                if changed {
                    ui.render();
                    ui.update_tray();
                }
            }
        });
    }

    fn render(&self) {
        let model = self.model.borrow();
        self.title.set_title(&model.title());
        match model.page() {
            Page::Missing { description } => {
                self.missing_page.set_description(Some(&description));
                self.stack.set_visible_child_name("missing");
            }
            Page::Off => self.stack.set_visible_child_name("off"),
            Page::Settings => self.stack.set_visible_child_name("settings"),
        }
        let Some(view) = model.view() else {
            self.title.set_subtitle("");
            return;
        };
        self.syncing.set(true);
        self.title.set_subtitle(&view.subtitle);
        if let Some(level) = view.battery {
            self.battery_bar.set_value(level.into());
        }
        self.battery_label.set_label(&view.battery_text);
        self.battery_row.set_subtitle(&view.charge_text);
        self.mute_row.set_active(view.muted);
        self.mic_row.set_subtitle(&view.mic_text);
        self.sidetone_row.set_active(view.sidetone);
        self.level_row.set_sensitive(view.sidetone);
        if let Some(level) = view.level
            && self.level_timer.borrow().is_none()
        {
            self.level_scale.set_value(level.into());
        }
        if let Some(minutes) = view.auto_off {
            self.select_auto_off(minutes);
        }
        self.voice_row.set_active(view.voice_prompts);
        self.syncing.set(false);
    }

    fn select_auto_off(&self, minutes: u8) {
        let choices = auto_off_choices(Some(minutes));
        if *self.auto_off_values.borrow() != choices {
            let labels: Vec<String> = choices.iter().map(|m| minutes_text(*m)).collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let old_len = self.auto_off_model.n_items();
            self.auto_off_model.splice(0, old_len, &labels);
            self.auto_off_values.replace(choices);
        }
        let index = self.auto_off_values.borrow().iter().position(|m| *m == minutes);
        if let Some(index) = index {
            self.auto_off_row.set_selected(index as u32);
        }
    }

    // --- tray --------------------------------------------------------------

    fn start_tray(self: &Rc<Self>) {
        let (sender, receiver) = async_channel::unbounded();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let handle = match HeadsetTray::new(sender).spawn().await {
                Ok(handle) => handle,
                Err(error) => {
                    // no tray on this desktop: the window's X quits the app instead
                    eprintln!("earmuff: no system tray ({error})");
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
                    TrayEvent::SetMute(on) => ui.send(Setting::MicMuted(on)),
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
        let state = TrayState::from_model(&self.model.borrow());
        glib::spawn_future_local(async move {
            handle.update(move |tray| tray.state = state).await;
        });
    }

    fn quit(&self) {
        if let Some(id) = self.level_timer.take() {
            id.remove();
        }
        if let Some(mut manager) = self.manager.take() {
            manager.stop();
        }
        self.hold.take();
        self.app.quit();
    }
}
