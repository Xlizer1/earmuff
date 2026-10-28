//! The main window. It shows what the [`Model`] says and sends the user's changes to
//! the device manager, which talks to the headset on its own thread.
//!
//! Layout: a header with Earmuff's mark and the headset switcher, and under it one page
//! of panels for the headset ([`dashboard`]).

mod dashboard;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use earmuff_application::{
    Alert, AppEvent, AudioSwitch, AudioSystem, Autostart, Backend, BatteryAlerts, BatteryHistory, DeviceManager,
    EqAction, EqualizerWorker, MIC_TEST_LENGTH, MicRecorder, Preferences, PreferencesStore, Profiles, SoftwareEq,
    SoftwareEqs, Switch, use_headset_when_ready,
};
use earmuff_domain::{DeviceId, Setting, UsbId};
use gtk::gio;
use gtk::glib::{self, SourceId};
use ksni::TrayMethods;

use self::dashboard::{Row, Rows};
use super::battery_graph::DAY;
use super::view_model::{
    BATTERY_ALERTS, Control, Input, Model, Page, alert_text, autostart_hint, battery_alert_index, battery_alert_label,
};
use super::{APP_ID, RESOURCE_PATH, UiOptions};
use crate::tray::{HeadsetTray, TrayEvent, TrayState};

/// Builds the window, starts looking for headsets and shows it
pub(super) fn open(app: &adw::Application, backends: Vec<Arc<dyn Backend>>, options: &UiOptions) {
    // the gaming look is dark, whatever the desktop's style
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    super::fonts::register();
    let ui = Ui::new(app, options);
    ui.connect_signals();
    ui.install_actions();
    if options.tray {
        ui.start_tray();
    }
    ui.start_equalizers(options);
    ui.start_manager(backends, options);
    ui.render();
    // started at login: stay in the tray (the tray shows the window if it can't start)
    if !options.hidden {
        ui.window.present();
    }
}

/// Earmuff borrows a vendor app's look and names vendors' products, so it says it isn't theirs
const TRADEMARKS: &str = "Earmuff isn't affiliated with or endorsed by HP, HyperX, SteelSeries, Logitech, Corsair or any other headset maker. Product names are trademarks of their owners.";

/// An app action: its name, keyboard shortcut and what it does
type AppAction = (&'static str, &'static str, fn(&Rc<Ui>));

/// News from a mic test running off the main thread
enum MicTestUpdate {
    Level(f64),
    Playing,
    Done(Result<(), String>),
}

/// The time in Unix seconds
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

struct Ui {
    app: adw::Application,
    window: adw::ApplicationWindow,
    /// The headset switcher, and the headset's name when there's only one
    switcher: gtk::DropDown,
    switcher_ids: RefCell<Vec<DeviceId>>,
    headset_name: gtk::Label,
    toasts: adw::ToastOverlay,
    stack: gtk::Stack,
    missing_page: adw::StatusPage,
    dashboard: gtk::Grid,
    /// One column of panels instead of the grid
    narrow: Cell<bool>,
    breakpoint: adw::Breakpoint,
    rows: RefCell<Rows>,

    model: RefCell<Model>,
    manager: RefCell<Option<DeviceManager>>,
    /// True while widgets are being updated from the headset, so their signals are ignored
    syncing: Cell<bool>,
    level_timers: RefCell<HashMap<Control, SourceId>>,
    tray: RefCell<Option<ksni::Handle<HeadsetTray>>>,
    /// While a tray shows the icon, closing the window only hides it
    tray_online: Cell<bool>,
    /// Keeps the app running while the window is hidden
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    preferences: RefCell<Preferences>,
    store: Arc<dyn PreferencesStore>,
    autostart: Arc<dyn Autostart>,
    audio: Arc<dyn AudioSystem>,
    audio_switch: RefCell<AudioSwitch>,
    mic: Arc<dyn MicRecorder>,
    mic_testing: Cell<bool>,
    alerts: RefCell<BatteryAlerts>,
    profiles: RefCell<Profiles>,
    history: RefCell<BatteryHistory>,
    /// How far back the battery graph goes
    history_span: Cell<u64>,
    software_eqs: RefCell<SoftwareEqs>,
    eq_worker: RefCell<Option<EqualizerWorker>>,
}

impl Ui {
    fn new(app: &adw::Application, options: &UiOptions) -> Rc<Ui> {
        let store = options.preferences.clone();
        let preferences = store.load();
        // header: the mark, the headset switcher and the menu
        let mark = gtk::Image::builder()
            .icon_name(format!("{APP_ID}-symbolic"))
            .pixel_size(20)
            .css_classes(["brand-mark"])
            .build();
        let brand = gtk::Box::builder().spacing(10).build();
        brand.append(&mark);
        brand.append(&gtk::Label::builder().label("earmuff").css_classes(["brand-name"]).build());
        let switcher = gtk::DropDown::builder()
            .name("device-switcher")
            .model(&gtk::StringList::new(&[]))
            .tooltip_text("Headset")
            .build();
        let headset_name = gtk::Label::builder().name("title").css_classes(["headset-name"]).build();
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title.append(&switcher);
        title.append(&headset_name);
        let menu = gio::Menu::new();
        menu.append(Some("Preferences"), Some("app.preferences"));
        menu.append(Some("About Earmuff"), Some("app.about"));
        menu.append(Some("Quit"), Some("app.quit"));
        let menu_button = gtk::MenuButton::builder()
            .name("main-menu")
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Menu")
            .build();
        let header = adw::HeaderBar::builder().name("header").title_widget(&title).build();
        header.pack_start(&brand);
        header.pack_end(&menu_button);

        let missing_page = adw::StatusPage::builder()
            .name("missing-page")
            .icon_name("earmuff-headset-symbolic")
            .title("Plug in your headset or its USB dongle")
            .build();
        let off_page = adw::StatusPage::builder()
            .name("off-page")
            .icon_name("earmuff-headset-symbolic")
            .title("Headset is off")
            .description("Turn it on with the power button.")
            .build();

        // the dashboard: a grid of panels, placed by the view model
        let dashboard = gtk::Grid::builder()
            .name("dashboard")
            .column_homogeneous(true)
            .row_spacing(16)
            .column_spacing(16)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .valign(gtk::Align::Start)
            .build();
        let settings_page = gtk::ScrolledWindow::builder()
            .name("settings-page")
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&adw::Clamp::builder().maximum_size(1100).tightening_threshold(1100).child(&dashboard).build())
            .build();

        let stack = gtk::Stack::builder().name("pages").transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&missing_page, Some("missing"));
        stack.add_named(&off_page, Some("off"));
        stack.add_named(&settings_page, Some("settings"));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let content = adw::ToolbarView::builder().content(&toasts).build();
        content.add_top_bar(&header);

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Earmuff")
            .default_width(1100)
            .default_height(800)
            // the widest panel, the microphone, in one column
            .width_request(420)
            .height_request(480)
            .content(&content)
            .build();
        // narrow windows put the panels in one column
        let breakpoint =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 820sp").expect("valid condition"));
        window.add_breakpoint(breakpoint.clone());

        Rc::new(Ui {
            app: app.clone(),
            window,
            switcher,
            switcher_ids: RefCell::default(),
            headset_name,
            toasts,
            stack,
            missing_page,
            dashboard,
            narrow: Cell::new(false),
            breakpoint,
            rows: RefCell::default(),
            model: RefCell::new(Model::default()),
            manager: RefCell::new(None),
            syncing: Cell::new(false),
            level_timers: RefCell::default(),
            tray: RefCell::new(None),
            tray_online: Cell::new(false),
            hold: RefCell::new(None),
            alerts: RefCell::new(BatteryAlerts::new(preferences.alerts())),
            profiles: RefCell::new(Profiles::new(options.profiles.clone(), preferences.remember_settings)),
            history: RefCell::new(BatteryHistory::new(options.history.clone())),
            history_span: Cell::new(DAY),
            audio_switch: RefCell::new(AudioSwitch::new(preferences.switch_audio)),
            preferences: RefCell::new(preferences),
            store,
            autostart: options.autostart.clone(),
            audio: options.audio.clone(),
            mic: options.mic.clone(),
            mic_testing: Cell::new(false),
            software_eqs: RefCell::new(SoftwareEqs::new(options.software_eq_store.clone())),
            eq_worker: RefCell::new(None),
        })
    }

    /// Software equalizers change off the main thread; what goes wrong shows as a toast
    fn start_equalizers(self: &Rc<Self>, options: &UiOptions) {
        let (sender, receiver) = async_channel::unbounded::<String>();
        let worker = EqualizerWorker::start(options.equalizer.clone(), move |error| {
            let _ = sender.send_blocking(error);
        });
        self.eq_worker.replace(Some(worker));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(error) = receiver.recv().await {
                let Some(ui) = weak.upgrade() else { break };
                ui.toasts.add_toast(adw::Toast::new(&format!("Couldn't change the equalizer: {error}")));
            }
        });
    }

    /// The user changed a software equalizer: save it, run it, show it
    fn change_software_eq(self: &Rc<Self>, device: &DeviceId, eq: SoftwareEq) {
        let change = self.software_eqs.borrow_mut().change(device, eq.clone());
        match change {
            Ok(Some(action)) => {
                if let Some(worker) = self.eq_worker.borrow().as_ref() {
                    worker.send(action);
                }
            }
            Ok(None) => {}
            Err(error) => self.toasts.add_toast(adw::Toast::new(&format!("Couldn't save the equalizer: {error}"))),
        }
        self.model.borrow_mut().set_software_eq(device, Some(eq));
        // not from inside the handler of the control that changed: turning it on adds controls
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(ui) = weak.upgrade() {
                ui.render();
            }
        });
    }

    // --- app actions and preferences --------------------------------------

    fn install_actions(self: &Rc<Self>) {
        let actions: [AppAction; 5] = [
            ("preferences", "<Control>comma", Ui::show_preferences),
            // what clicking a notification does
            ("show", "", |ui| ui.window.present()),
            ("about", "F1", Ui::show_about),
            // closing hides the window when there's a tray icon
            ("close", "<Control>w", |ui| ui.window.close()),
            ("quit", "<Control>q", |ui| ui.quit()),
        ];
        for (name, accel, run) in actions {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(ui) = weak.upgrade() {
                    run(&ui);
                }
            });
            self.app.add_action(&action);
            if !accel.is_empty() {
                self.app.set_accels_for_action(&format!("app.{name}"), &[accel]);
            }
        }
    }

    fn show_preferences(self: &Rc<Self>) {
        let login = adw::SwitchRow::builder()
            .name("start-on-login")
            .title("Start when you log in")
            .subtitle("Earmuff starts hidden in the tray")
            .active(self.autostart.is_enabled())
            .build();
        let weak = Rc::downgrade(self);
        login.connect_active_notify(move |row| {
            let Some(ui) = weak.upgrade() else { return };
            if let Err(error) = ui.autostart.set_enabled(row.is_active()) {
                ui.toasts.add_toast(adw::Toast::new(&format!("Couldn't change start-up: {error}")));
            }
        });
        let startup = adw::PreferencesGroup::builder().title("Startup").build();
        startup.set_description(autostart_hint(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()));
        startup.add(&login);

        let alert_labels: Vec<String> = BATTERY_ALERTS.iter().map(|level| battery_alert_label(*level)).collect();
        let alert_labels: Vec<&str> = alert_labels.iter().map(String::as_str).collect();
        let low = adw::ComboRow::builder()
            .name("battery-alert")
            .title("Warn when the battery is low")
            .model(&gtk::StringList::new(&alert_labels))
            .selected(battery_alert_index(self.preferences.borrow().battery_alert))
            .build();
        let weak = Rc::downgrade(self);
        low.connect_selected_notify(move |row| {
            if let (Some(ui), Some(level)) = (weak.upgrade(), BATTERY_ALERTS.get(row.selected() as usize)) {
                ui.update_preferences(|preferences| preferences.battery_alert = *level);
            }
        });
        let charged = adw::SwitchRow::builder()
            .name("notify-charged")
            .title("Say when it's fully charged")
            .active(self.preferences.borrow().notify_charged)
            .build();
        let weak = Rc::downgrade(self);
        charged.connect_active_notify(move |row| {
            if let Some(ui) = weak.upgrade() {
                ui.update_preferences(|preferences| preferences.notify_charged = row.is_active());
            }
        });
        let remember = adw::SwitchRow::builder()
            .name("remember-settings")
            .title("Remember each headset's settings")
            .subtitle("Puts them back when a headset reconnects; some forget them when turned off")
            .active(self.preferences.borrow().remember_settings)
            .build();
        let weak = Rc::downgrade(self);
        remember.connect_active_notify(move |row| {
            if let Some(ui) = weak.upgrade() {
                ui.update_preferences(|preferences| preferences.remember_settings = row.is_active());
            }
        });
        let sound = adw::SwitchRow::builder()
            .name("switch-audio")
            .title("Use the headset for sound")
            .subtitle("Makes a headset the computer's speakers and mic when it turns on")
            .active(self.preferences.borrow().switch_audio)
            .build();
        let weak = Rc::downgrade(self);
        sound.connect_active_notify(move |row| {
            if let Some(ui) = weak.upgrade() {
                ui.update_preferences(|preferences| preferences.switch_audio = row.is_active());
            }
        });
        let headsets = adw::PreferencesGroup::builder().title("Headsets").build();
        headsets.add(&remember);
        headsets.add(&sound);

        let notifications = adw::PreferencesGroup::builder().title("Notifications").build();
        notifications.add(&low);
        notifications.add(&charged);

        let page = adw::PreferencesPage::builder().title("General").icon_name("preferences-system-symbolic").build();
        page.add(&headsets);
        page.add(&notifications);
        page.add(&startup);
        let dialog = adw::PreferencesDialog::builder().name("preferences").build();
        dialog.add(&page);
        dialog.present(Some(&self.window));
    }

    fn show_about(self: &Rc<Self>) {
        let about = adw::AboutDialog::builder()
            .name("about")
            .application_name("Earmuff")
            .application_icon(APP_ID)
            .developer_name("Xlizer1")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Battery and settings for your gaming headset, in place of vendor apps like HyperX NGENUITY.")
            .website("https://github.com/Xlizer1/earmuff")
            .issue_url("https://github.com/Xlizer1/earmuff/issues")
            .license_type(gtk::License::Gpl30)
            .build();
        about.add_acknowledgement_section(
            Some("Headset protocols from"),
            &[
                "HeadsetControl https://github.com/Sapd/HeadsetControl",
                "HyperHeadset https://github.com/LennardKittner/HyperHeadset",
            ],
        );
        about.add_legal_section("Trademarks", None, gtk::License::Custom, Some(TRADEMARKS));
        about.add_legal_section(
            "IBM Plex Sans",
            Some("© 2017 IBM Corp., with Reserved Font Name \"Plex\""),
            gtk::License::Custom,
            Some("Licensed under the SIL Open Font License, Version 1.1."),
        );
        about.present(Some(&self.window));
    }

    /// Changes the preferences, saves them, and passes on what other parts need
    fn update_preferences(&self, change: impl FnOnce(&mut Preferences)) {
        change(&mut self.preferences.borrow_mut());
        self.alerts.borrow_mut().set_settings(self.preferences.borrow().alerts());
        self.profiles.borrow_mut().set_enabled(self.preferences.borrow().remember_settings);
        self.audio_switch.borrow_mut().set_enabled(self.preferences.borrow().switch_audio);
        if let Err(error) = self.store.save(&self.preferences.borrow()) {
            self.toasts.add_toast(adw::Toast::new(&format!("Couldn't save preferences: {error}")));
        }
    }

    // --- widgets -> headset ----------------------------------------------

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.switcher.connect_selected_notify(move |switcher| {
            let Some(ui) = weak.upgrade() else { return };
            if ui.syncing.get() {
                return;
            }
            let id = ui.switcher_ids.borrow().get(switcher.selected() as usize).cloned();
            if let Some(id) = id
                && ui.model.borrow_mut().select(&id)
            {
                ui.render();
                ui.update_tray();
            }
        });
        for narrow in [true, false] {
            let weak = Rc::downgrade(self);
            let placed = move |_: &adw::Breakpoint| {
                if let Some(ui) = weak.upgrade() {
                    ui.narrow.set(narrow);
                    ui.place_panels();
                }
            };
            if narrow {
                self.breakpoint.connect_apply(placed);
            } else {
                self.breakpoint.connect_unapply(placed);
            }
        }

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

    /// The user changed a control: send the setting it stands for
    fn input(self: &Rc<Self>, control: Control, input: Input) {
        if self.syncing.get() {
            return;
        }
        let software = self.model.borrow().software_eq_change(control, &input);
        if let Some((device, eq)) = software {
            self.change_software_eq(&device, eq);
            return;
        }
        let (setting, device) = {
            let model = self.model.borrow();
            (model.setting(control, input), model.current().map(|(info, _)| info.id.clone()))
        };
        match (setting, device) {
            (Some(setting), Some(device)) => self.send_to(&device, setting),
            // a choice that asks for nothing (the "Custom" preset) shows what's true again
            _ => {
                let weak = Rc::downgrade(self);
                glib::idle_add_local_once(move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.render();
                    }
                });
            }
        }
    }

    fn send_to(&self, device: &DeviceId, setting: Setting) {
        if let Some(manager) = self.manager.borrow().as_ref() {
            manager.apply(device, setting);
        }
    }

    /// Waits until a slider stops for a moment instead of sending every step of a drag
    fn schedule_input(self: &Rc<Self>, control: Control) {
        if let Some(id) = self.level_timers.borrow_mut().remove(&control) {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        // a headset's curve takes longer to send, so wait a little longer for the drag to end
        let wait = if control == Control::Equalizer { 300 } else { 150 };
        let id = glib::timeout_add_local_once(Duration::from_millis(wait), move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.level_timers.borrow_mut().remove(&control);
            let input = match ui.rows.borrow().rows.get(&control) {
                Some(Row::Level(_, scale, _)) => Input::Level(scale.value().round().clamp(0.0, 100.0) as u8),
                Some(Row::Curve(editor)) => Input::Curve(editor.gains()),
                _ => return,
            };
            ui.input(control, input);
        });
        self.level_timers.borrow_mut().insert(control, id);
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
                // a headset without an equalizer of its own gets the software one
                let eq_action = ui.software_eqs.borrow_mut().on_event(&event);
                if let (Some(action), Some(worker)) = (eq_action, ui.eq_worker.borrow().as_ref()) {
                    worker.send(action);
                }
                if let AppEvent::Added(info) = &event {
                    let eq = ui.software_eqs.borrow().get(&info.id).cloned();
                    ui.model.borrow_mut().set_software_eq(&info.id, eq);
                }
                let recorded = ui.history.borrow_mut().on_event(&event, now());
                let changed = ui.model.borrow_mut().apply(&event) || recorded;
                if changed {
                    ui.render();
                    ui.update_tray();
                }
                let failure = ui.model.borrow_mut().take_failure();
                if let Some(reason) = failure {
                    ui.toasts.add_toast(adw::Toast::new(&reason));
                }
                let alerts = ui.alerts.borrow_mut().on_event(&event);
                for alert in alerts {
                    ui.notify(&alert);
                }
                // a headset that came back on gets its remembered settings
                let restore = ui.profiles.borrow_mut().on_event(&event);
                for (device, setting) in restore {
                    ui.send_to(&device, setting);
                }
                let switch = ui.audio_switch.borrow_mut().on_event(&event);
                if let Some(headset) = switch {
                    ui.switch_audio(headset);
                }
            }
        });
    }

    /// Records a few seconds from the shown headset's mic and plays them back, off the main thread
    fn test_mic(self: &Rc<Self>) {
        let Some(headset) = self.model.borrow().current().map(|(info, _)| info.id.usb) else { return };
        if self.mic_testing.replace(true) {
            return;
        }
        let mic = self.mic.clone();
        let (sender, receiver) = async_channel::unbounded();
        std::thread::spawn(move || {
            let result = mic
                .record(headset, MIC_TEST_LENGTH, &mut |level| {
                    let _ = sender.send_blocking(MicTestUpdate::Level(level));
                })
                .and_then(|recording| {
                    let _ = sender.send_blocking(MicTestUpdate::Playing);
                    mic.play(headset, &recording)
                });
            let _ = sender.send_blocking(MicTestUpdate::Done(result));
        });
        self.show_mic_test(&MicTestUpdate::Level(0.0));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(update) = receiver.recv().await {
                let Some(ui) = weak.upgrade() else { break };
                ui.show_mic_test(&update);
            }
        });
    }

    fn show_mic_test(self: &Rc<Self>, update: &MicTestUpdate) {
        if let MicTestUpdate::Done(result) = update {
            self.mic_testing.set(false);
            if let Err(error) = result {
                self.toasts.add_toast(adw::Toast::new(&format!("Couldn't test the mic: {error}")));
            }
        }
        if let Some(Row::MicTest(row, meter, button)) = self.rows.borrow().rows.get(&Control::MicTest) {
            match update {
                MicTestUpdate::Level(level) => {
                    row.set_subtitle("Recording… say something");
                    button.set_sensitive(false);
                    meter.set_value(*level);
                }
                MicTestUpdate::Playing => {
                    row.set_subtitle("Playing it back…");
                    meter.set_value(0.0);
                }
                MicTestUpdate::Done(_) => meter.set_value(0.0),
            }
        }
        if matches!(update, MicTestUpdate::Done(_)) {
            // back to what the row says when no test is running
            self.render();
        }
    }

    /// Makes a headset that just turned on the computer's sound device, off the main thread
    fn switch_audio(self: &Rc<Self>, headset: UsbId) {
        let audio = self.audio.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                use_headset_when_ready(audio.as_ref(), headset, 10, Duration::from_millis(500))
            })
            .await;
            let Some(ui) = weak.upgrade() else { return };
            match result {
                Ok(Ok(Switch::Switched(name))) => {
                    ui.toasts.add_toast(adw::Toast::new(&format!("Sound switched to {name}")))
                }
                Ok(Err(error)) => ui.toasts.add_toast(adw::Toast::new(&format!("Couldn't switch the sound: {error}"))),
                // already the default, or a headset without a sound card of its own (e.g. Bluetooth)
                Ok(Ok(Switch::Already | Switch::NotFound)) | Err(_) => {}
            }
        });
    }

    /// Shows a battery alert as a desktop notification
    fn notify(&self, alert: &Alert) {
        let (title, body) = alert_text(alert);
        let notification = gio::Notification::new(&title);
        notification.set_body(Some(&body));
        notification.set_icon(&gio::ThemedIcon::new(APP_ID));
        notification.set_default_action("app.show");
        let device = match alert {
            Alert::Critical { device, .. } => {
                notification.set_priority(gio::NotificationPriority::High);
                device
            }
            Alert::Low { device, .. } | Alert::Charged { device, .. } => device,
        };
        // one notification per headset: a newer alert replaces the older one
        self.app.send_notification(Some(&format!("battery-{}", device.usb)), &notification);
    }

    fn render(self: &Rc<Self>) {
        self.sync_switcher();
        let (title, page, view, device) = {
            let model = self.model.borrow();
            (model.title(), model.page(), model.view(), model.current().map(|(info, _)| info.id.clone()))
        };
        self.window.set_title(Some(&title));
        match page {
            Page::Missing { description } => {
                self.missing_page.set_description(Some(&description));
                self.stack.set_visible_child_name("missing");
            }
            Page::Off => self.stack.set_visible_child_name("off"),
            Page::Settings => self.stack.set_visible_child_name("settings"),
        }
        if let Some(view) = view {
            self.show_dashboard(&view, device);
        }
    }

    // --- tray --------------------------------------------------------------

    fn start_tray(self: &Rc<Self>) {
        let (sender, receiver) = async_channel::unbounded();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let handle = match HeadsetTray::new(sender, tray_pixmap()).spawn().await {
                Ok(handle) => handle,
                Err(error) => {
                    // no tray on this desktop: the window's X quits the app instead, and
                    // a window started hidden shows after all, or Earmuff would be unreachable
                    eprintln!("earmuff: no system tray ({error})");
                    if let Some(ui) = weak.upgrade()
                        && !ui.window.is_visible()
                    {
                        ui.window.present();
                    }
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
                    TrayEvent::SetMute(on) => {
                        let device = ui.model.borrow().current().map(|(info, _)| info.id.clone());
                        if let Some(device) = device {
                            ui.send_to(&device, Setting::MicMuted(on));
                        }
                    }
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
        for (_, timer) in self.level_timers.borrow_mut().drain() {
            timer.remove();
        }
        if let Some(mut manager) = self.manager.take() {
            manager.stop();
        }
        // software equalizers go with Earmuff, and the headsets play straight again
        if let Some(worker) = self.eq_worker.take() {
            for headset in self.software_eqs.borrow().running() {
                worker.send(EqAction::Stop(headset));
            }
            worker.flush();
        }
        self.hold.take();
        self.app.quit();
    }
}

/// The tray icon as pixels: ARGB32 in network byte order, as StatusNotifierItem wants
fn tray_pixmap() -> Vec<ksni::Icon> {
    let texture = gtk::gdk::Texture::from_resource(&format!("{RESOURCE_PATH}/icons/tray/earmuff-64.png"));
    let mut downloader = gtk::gdk::TextureDownloader::new(&texture);
    downloader.set_format(gtk::gdk::MemoryFormat::A8r8g8b8);
    let (bytes, stride) = downloader.download_bytes();
    let (width, height) = (texture.width(), texture.height());
    let row = usize::try_from(width).unwrap_or(0) * 4;
    let data = bytes
        .chunks(stride)
        .take(usize::try_from(height).unwrap_or(0))
        .flat_map(|line| &line[..row])
        .copied()
        .collect();
    vec![ksni::Icon { width, height, data }]
}
