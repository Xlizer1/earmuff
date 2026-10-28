//! The main window. It shows what the [`Model`] says and sends the user's changes to
//! the device manager, which talks to the headset on its own thread.
//!
//! Layout: a sidebar listing every headset; next to it a banner with the headset's
//! art, name, status and battery ring, and below that a tab per kind of setting.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use earmuff_application::{
    Alert, AppEvent, AudioSwitch, AudioSystem, Autostart, Backend, BatteryAlerts, BatteryHistory, DeviceManager,
    EqAction, EqualizerWorker, MIC_TEST_LENGTH, MicRecorder, Preferences, PreferencesStore, Profiles, SoftwareEq,
    SoftwareEqs, Switch, time_left_text, use_headset_when_ready,
};
use earmuff_domain::{DeviceId, Setting, UsbId};
use gtk::gio;
use gtk::glib::{self, SourceId};
use ksni::TrayMethods;

use super::battery_graph::{BatteryGraph, DAY, WEEK};
use super::battery_ring::BatteryRing;
use super::equalizer::EqualizerEditor;
use super::view_model::{
    ACCENTS, BATTERY_ALERTS, Control, ControlView, DeviceView, Input, Model, Page, SidebarItem, Tab, Widget,
    accent_class, accent_index, alert_text, autostart_hint, battery_alert_index, battery_alert_label,
};
use super::{APP_ID, RESOURCE_PATH, UiOptions};
use crate::tray::{HeadsetTray, TrayEvent, TrayState};

/// Builds the window, starts looking for headsets and shows it
pub(super) fn open(app: &adw::Application, backends: Vec<Arc<dyn Backend>>, options: &UiOptions) {
    // the gaming look is dark, whatever the desktop's style
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    let ui = Ui::new(app, options);
    ui.connect_signals();
    ui.install_actions();
    ui.apply_accent();
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

/// An app action: its name, keyboard shortcut and what it does
type AppAction = (&'static str, &'static str, fn(&Rc<Ui>));

/// The widgets of one control
enum Row {
    Switch(adw::SwitchRow),
    Level(adw::ActionRow, gtk::Scale),
    Choice(adw::ComboRow),
    Info(adw::ActionRow),
    /// A read-only meter, e.g. the ChatMix game/chat balance
    Balance(adw::ActionRow, gtk::Scale),
    Button(adw::ActionRow),
    Curve(adw::PreferencesRow, Rc<EqualizerEditor>),
    MicTest(adw::ActionRow, gtk::LevelBar, gtk::Button),
}

/// News from a mic test running off the main thread
enum MicTestUpdate {
    Level(f64),
    Playing,
    Done(Result<(), String>),
}

/// The tabs as currently built
#[derive(Default)]
struct Rows {
    device: Option<DeviceId>,
    controls: Vec<Control>,
    has_battery: bool,
    pages: Vec<gtk::Widget>,
    rows: HashMap<Control, Row>,
    graph: Option<BatteryGraph>,
}

/// The time in Unix seconds
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// One headset's entry in the sidebar
struct SidebarRow {
    status: gtk::Label,
    battery: gtk::LevelBar,
}

/// The banner over the tabs
struct Hero {
    widget: gtk::Box,
    name: gtk::Label,
    detail: gtk::Label,
    badge: gtk::Label,
    time_left: gtk::Label,
    ring: BatteryRing,
}

impl Hero {
    fn new() -> Self {
        let art = gtk::Image::builder()
            .icon_name("earmuff-headset-symbolic")
            .pixel_size(108)
            .css_classes(["hero-art"])
            .valign(gtk::Align::Center)
            .build();
        let name = gtk::Label::builder().name("hero-name").css_classes(["hero-name"]).xalign(0.0).wrap(true).build();
        let detail = gtk::Label::builder().name("hero-detail").css_classes(["dim-label"]).xalign(0.0).build();
        let badge = gtk::Label::builder().name("badge").css_classes(["badge"]).halign(gtk::Align::Start).build();
        let time_left =
            gtk::Label::builder().name("time-left").css_classes(["dim-label", "caption"]).xalign(0.0).build();
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        text.append(&name);
        text.append(&detail);
        text.append(&badge);
        text.append(&time_left);
        let ring = BatteryRing::new();
        let widget = gtk::Box::builder().name("hero").spacing(28).css_classes(["hero"]).build();
        widget.append(&art);
        widget.append(&text);
        widget.append(&ring.widget);
        Hero { widget, name, detail, badge, time_left, ring }
    }

    fn show(&self, view: &DeviceView, time_left: Option<String>) {
        self.time_left.set_visible(time_left.is_some());
        self.time_left.set_label(&time_left.unwrap_or_default());
        self.name.set_label(&view.title.to_uppercase());
        self.detail.set_label(&view.detail);
        self.badge.set_label(view.badge);
        for (class, on) in [("charging", view.charging), ("off", view.badge == "OFF")] {
            if on {
                self.badge.add_css_class(class);
            } else {
                self.badge.remove_css_class(class);
            }
        }
        self.ring.widget.set_visible(view.has_battery);
        self.ring.show(view.battery, &view.battery_text, &view.charge_text, view.battery_low, view.charging);
    }
}

struct Ui {
    app: adw::Application,
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    device_list: gtk::ListBox,
    sidebar_ids: RefCell<Vec<DeviceId>>,
    sidebar_rows: RefCell<Vec<SidebarRow>>,
    toasts: adw::ToastOverlay,
    stack: gtk::Stack,
    missing_page: adw::StatusPage,
    hero: Hero,
    tabs: adw::ViewStack,
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
        // sidebar: every connected headset
        let device_list = gtk::ListBox::builder().name("device-list").css_classes(["navigation-sidebar"]).build();
        device_list.set_placeholder(Some(
            &gtk::Label::builder().label("No headsets").css_classes(["dim-label"]).margin_top(24).build(),
        ));
        let sidebar_header = adw::HeaderBar::builder()
            .show_end_title_buttons(false)
            .title_widget(&gtk::Box::new(gtk::Orientation::Horizontal, 0))
            .build();
        sidebar_header.pack_start(&gtk::Label::builder().label("HEADSETS").css_classes(["sidebar-heading"]).build());
        let sidebar = adw::ToolbarView::builder()
            .content(&gtk::ScrolledWindow::builder().child(&device_list).vexpand(true).build())
            .build();
        sidebar.add_top_bar(&sidebar_header);

        // content: header, then the missing / off / settings pages
        let title = adw::WindowTitle::new("Earmuff", "");
        title.set_widget_name("title");
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
        let sidebar_button =
            gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").tooltip_text("Headsets").build();
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&sidebar_button);
        header.pack_end(&menu_button);

        let missing_page = adw::StatusPage::builder()
            .name("missing-page")
            .icon_name("earmuff-headset-symbolic")
            .title("Plug in your headset's USB dongle")
            .build();
        let off_page = adw::StatusPage::builder()
            .name("off-page")
            .icon_name("earmuff-headset-symbolic")
            .title("Headset is off")
            .description("Turn it on with the power button.")
            .build();

        let hero = Hero::new();
        let tabs = adw::ViewStack::builder().name("tabs").vhomogeneous(false).build();
        let switcher = adw::ViewSwitcher::builder()
            .stack(&tabs)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .halign(gtk::Align::Center)
            .build();
        let settings = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(18)
            .margin_bottom(24)
            .margin_start(18)
            .margin_end(18)
            .build();
        settings.append(&hero.widget);
        settings.append(&switcher);
        settings.append(&tabs);
        let settings_page = gtk::ScrolledWindow::builder()
            .name("settings-page")
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&adw::Clamp::builder().maximum_size(880).child(&settings).build())
            .build();

        let stack = gtk::Stack::builder().name("pages").transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&missing_page, Some("missing"));
        stack.add_named(&off_page, Some("off"));
        stack.add_named(&settings_page, Some("settings"));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let content = adw::ToolbarView::builder().content(&toasts).build();
        content.add_top_bar(&header);

        let split = adw::OverlaySplitView::builder()
            .name("split")
            .sidebar(&sidebar)
            .content(&content)
            .min_sidebar_width(220.0)
            .max_sidebar_width(280.0)
            .build();
        split.bind_property("show-sidebar", &sidebar_button, "active").bidirectional().sync_create().build();
        split.bind_property("collapsed", &sidebar_button, "visible").sync_create().build();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Earmuff")
            .default_width(1000)
            .default_height(720)
            .width_request(360)
            .height_request(480)
            .content(&split)
            .build();
        // narrow windows fold the sidebar away; the header button brings it back
        let narrow =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 720sp").expect("valid condition"));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(narrow);

        Rc::new(Ui {
            app: app.clone(),
            window,
            title,
            device_list,
            sidebar_ids: RefCell::default(),
            sidebar_rows: RefCell::default(),
            toasts,
            stack,
            missing_page,
            hero,
            tabs,
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
                ui.toasts.add_toast(adw::Toast::new(&format!("Equalizer: {error}")));
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
        let labels: Vec<&str> = ACCENTS.iter().map(|(_, label)| *label).collect();
        let accent = adw::ComboRow::builder()
            .name("accent")
            .title("Accent color")
            .model(&gtk::StringList::new(&labels))
            .selected(accent_index(&self.preferences.borrow().accent))
            .build();
        let weak = Rc::downgrade(self);
        accent.connect_selected_notify(move |row| {
            if let (Some(ui), Some((name, _))) = (weak.upgrade(), ACCENTS.get(row.selected() as usize)) {
                ui.update_preferences(|preferences| preferences.accent = (*name).into());
                ui.apply_accent();
            }
        });
        let appearance = adw::PreferencesGroup::builder().title("Appearance").build();
        appearance.add(&accent);

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
        page.add(&appearance);
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

    fn apply_accent(&self) {
        for (name, _) in ACCENTS {
            self.window.remove_css_class(&format!("accent-{name}"));
        }
        if let Some(class) = accent_class(&self.preferences.borrow().accent) {
            self.window.add_css_class(&class);
        }
    }

    // --- widgets -> headset ----------------------------------------------

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.device_list.connect_row_selected(move |_, row| {
            let (Some(ui), Some(row)) = (weak.upgrade(), row) else { return };
            if ui.syncing.get() {
                return;
            }
            let id = ui.sidebar_ids.borrow().get(usize::try_from(row.index()).unwrap_or(usize::MAX)).cloned();
            if let Some(id) = id
                && ui.model.borrow_mut().select(&id)
            {
                ui.render();
                ui.update_tray();
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
        let model = self.model.borrow();
        if let (Some(setting), Some((info, _))) = (model.setting(control, input), model.current()) {
            self.send_to(&info.id, setting);
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
                Some(Row::Level(_, scale)) => Input::Level(scale.value().round().clamp(0.0, 100.0) as u8),
                Some(Row::Curve(_, editor)) => Input::Curve(editor.gains()),
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
                self.toasts.add_toast(adw::Toast::new(&format!("Mic test: {error}")));
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
        self.sync_sidebar();
        let (title, page, view, device) = {
            let model = self.model.borrow();
            (model.title(), model.page(), model.view(), model.current().map(|(info, _)| info.id.clone()))
        };
        self.title.set_title(&title);
        match page {
            Page::Missing { description } => {
                self.missing_page.set_description(Some(&description));
                self.stack.set_visible_child_name("missing");
            }
            Page::Off => self.stack.set_visible_child_name("off"),
            Page::Settings => self.stack.set_visible_child_name("settings"),
        }
        let Some(view) = view else {
            self.title.set_subtitle("");
            return;
        };
        self.title.set_subtitle(&view.subtitle);
        let now = now();
        let time_left = device.as_ref().and_then(|id| self.history.borrow().minutes_left(id, now)).map(time_left_text);
        self.hero.show(&view, time_left);
        let rebuild = {
            let rows = self.rows.borrow();
            let controls: Vec<Control> = view.layout().into_iter().map(|(control, _)| control).collect();
            rows.device != device || rows.controls != controls || rows.has_battery != view.has_battery
        };
        if rebuild {
            self.build_tabs(&view, device.clone());
        }
        self.update_rows(&view);
        if let (Some(graph), Some(id)) = (&self.rows.borrow().graph, &device) {
            graph.show(self.history.borrow().samples(id), now);
        }
    }

    /// The battery graph, with buttons for its span, as a group for the Power tab
    fn battery_history_group(self: &Rc<Self>, graph: &BatteryGraph) -> adw::PreferencesGroup {
        graph.set_span(self.history_span.get());
        let day = gtk::ToggleButton::builder()
            .name("history-day")
            .label("24 h")
            .active(self.history_span.get() == DAY)
            .build();
        let week = gtk::ToggleButton::builder()
            .name("history-week")
            .label("7 days")
            .group(&day)
            .active(self.history_span.get() == WEEK)
            .build();
        for (button, span) in [(&day, DAY), (&week, WEEK)] {
            let weak = Rc::downgrade(self);
            button.connect_toggled(move |button| {
                let Some(ui) = weak.upgrade() else { return };
                if button.is_active() {
                    ui.history_span.set(span);
                    if let Some(graph) = &ui.rows.borrow().graph {
                        graph.set_span(span);
                    }
                }
            });
        }
        let buttons = gtk::Box::builder().css_classes(["linked"]).valign(gtk::Align::Center).build();
        buttons.append(&day);
        buttons.append(&week);
        let card = gtk::Frame::builder().css_classes(["card"]).child(&graph.widget).build();
        graph.widget.set_margin_top(10);
        graph.widget.set_margin_bottom(6);
        graph.widget.set_margin_start(12);
        graph.widget.set_margin_end(12);
        let group = adw::PreferencesGroup::builder().title("BATTERY HISTORY").build();
        group.set_header_suffix(Some(&buttons));
        group.add(&card);
        group
    }

    /// Lists the connected headsets in the sidebar and highlights the one shown
    fn sync_sidebar(&self) {
        let model = self.model.borrow();
        let items = model.sidebar();
        let ids: Vec<DeviceId> = items.iter().map(|item| item.id.clone()).collect();
        self.syncing.set(true);
        if *self.sidebar_ids.borrow() != ids {
            while let Some(child) = self.device_list.first_child() {
                self.device_list.remove(&child);
            }
            let rows: Vec<SidebarRow> = items.iter().map(|item| self.add_sidebar_row(item)).collect();
            self.sidebar_rows.replace(rows);
            self.sidebar_ids.replace(ids.clone());
        }
        for (item, row) in items.iter().zip(self.sidebar_rows.borrow().iter()) {
            row.status.set_label(&item.status);
            row.battery.set_visible(item.battery.is_some());
            row.battery.set_value(item.battery.map_or(0.0, f64::from));
        }
        let current = model.current().and_then(|(info, _)| ids.iter().position(|id| *id == info.id));
        if let Some(row) = current.and_then(|index| self.device_list.row_at_index(index as i32)) {
            self.device_list.select_row(Some(&row));
        }
        self.syncing.set(false);
    }

    fn add_sidebar_row(&self, item: &SidebarItem) -> SidebarRow {
        let name = gtk::Label::builder()
            .label(&item.name)
            .css_classes(["sidebar-name"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let status = gtk::Label::builder().css_classes(["caption", "dim-label"]).xalign(0.0).build();
        let battery = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).css_classes(["sidebar-battery"]).build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(4)
            .margin_end(4)
            .build();
        content.append(&name);
        content.append(&status);
        content.append(&battery);
        self.device_list.append(&content);
        SidebarRow { status, battery }
    }

    /// Builds a tab for each kind of setting the headset has
    fn build_tabs(self: &Rc<Self>, view: &DeviceView, device: Option<DeviceId>) {
        for (_, timer) in self.level_timers.borrow_mut().drain() {
            timer.remove();
        }
        for page in self.rows.borrow().pages.iter() {
            self.tabs.remove(page);
        }
        let mut rows = HashMap::new();
        let mut pages = Vec::new();
        let graph = view.has_battery.then(BatteryGraph::new);
        for tab in view.tabs() {
            let page = gtk::Box::builder().name(tab.name()).orientation(gtk::Orientation::Vertical).spacing(24).build();
            if let (Tab::Power, Some(graph)) = (tab, &graph) {
                page.append(&self.battery_history_group(graph));
            }
            for section in view.sections.iter().filter(|section| section.tab == tab) {
                let group = adw::PreferencesGroup::builder().title(section.title.to_uppercase()).build();
                group.set_description(section.description);
                for control in &section.controls {
                    let row = self.build_row(control);
                    group.add(match &row {
                        Row::Switch(row) => row.upcast_ref::<gtk::Widget>(),
                        Row::Level(row, _) | Row::Info(row) | Row::Balance(row, _) | Row::Button(row) => {
                            row.upcast_ref()
                        }
                        Row::Choice(row) => row.upcast_ref(),
                        Row::Curve(row, _) => row.upcast_ref(),
                        Row::MicTest(row, ..) => row.upcast_ref(),
                    });
                    rows.insert(control.control, row);
                }
                page.append(&group);
            }
            self.tabs.add_titled_with_icon(&page, Some(tab.name()), tab.title(), tab.icon());
            pages.push(page.upcast());
        }
        let controls = view.layout().into_iter().map(|(control, _)| control).collect();
        self.rows.replace(Rows { device, controls, has_battery: view.has_battery, pages, rows, graph });
    }

    fn build_row(self: &Rc<Self>, view: &ControlView) -> Row {
        let control = view.control;
        let weak = Rc::downgrade(self);
        match &view.widget {
            Widget::Switch(_) => {
                let row = adw::SwitchRow::builder().name(control.name()).title(view.title).build();
                row.set_subtitle(view.subtitle.unwrap_or_default());
                row.connect_active_notify(move |row| {
                    if let Some(ui) = weak.upgrade() {
                        ui.input(control, Input::Switch(row.is_active()));
                    }
                });
                Row::Switch(row)
            }
            Widget::Level(_) => {
                let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
                scale.set_widget_name(control.name());
                scale.set_hexpand(true);
                scale.set_round_digits(0);
                scale.add_mark(0.0, gtk::PositionType::Bottom, Some("Quiet"));
                scale.add_mark(100.0, gtk::PositionType::Bottom, Some("Loud"));
                scale.connect_value_changed(move |_| {
                    if let Some(ui) = weak.upgrade()
                        && !ui.syncing.get()
                    {
                        ui.schedule_input(control);
                    }
                });
                let row = adw::ActionRow::builder().name(format!("{}-row", control.name())).title(view.title).build();
                row.add_suffix(&scale);
                Row::Level(row, scale)
            }
            Widget::MicTest => {
                let meter = gtk::LevelBar::builder()
                    .name("mic-level")
                    .min_value(0.0)
                    .max_value(1.0)
                    .width_request(140)
                    .valign(gtk::Align::Center)
                    .build();
                // no "low"/"high" colors: a loud voice isn't a problem
                for offset in [gtk::LEVEL_BAR_OFFSET_LOW, gtk::LEVEL_BAR_OFFSET_HIGH, gtk::LEVEL_BAR_OFFSET_FULL] {
                    meter.remove_offset_value(Some(offset));
                }
                let button = gtk::Button::builder()
                    .label("Test")
                    .name(control.name())
                    .valign(gtk::Align::Center)
                    .css_classes(["pill"])
                    .build();
                button.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.test_mic();
                    }
                });
                let row = adw::ActionRow::builder().name(format!("{}-row", control.name())).title(view.title).build();
                row.add_suffix(&meter);
                row.add_suffix(&button);
                Row::MicTest(row, meter, button)
            }
            Widget::Curve(curve) => {
                let editor = EqualizerEditor::new(curve, move || {
                    if let Some(ui) = weak.upgrade()
                        && !ui.syncing.get()
                    {
                        ui.schedule_input(control);
                    }
                });
                let row = adw::PreferencesRow::builder()
                    .name(format!("{}-row", control.name()))
                    .title(view.title)
                    .activatable(false)
                    .child(&editor.widget)
                    .build();
                Row::Curve(row, editor)
            }
            Widget::Choice { options, .. } => {
                let options: Vec<&str> = options.iter().map(String::as_str).collect();
                let row = adw::ComboRow::builder()
                    .name(control.name())
                    .title(view.title)
                    .model(&gtk::StringList::new(&options))
                    .build();
                row.connect_selected_notify(move |row| {
                    if let Some(ui) = weak.upgrade() {
                        ui.input(control, Input::Choice(row.selected()));
                    }
                });
                Row::Choice(row)
            }
            Widget::Info(_) => Row::Info(adw::ActionRow::builder().name(control.name()).title(view.title).build()),
            Widget::Balance(_) => {
                // follows the dial on the headset; it can't be set from here
                let meter = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 128.0, 1.0);
                meter.set_widget_name("chatmix-meter");
                meter.set_hexpand(true);
                meter.set_draw_value(false);
                meter.set_can_target(false);
                meter.set_focusable(false);
                meter.add_mark(0.0, gtk::PositionType::Bottom, Some("Game"));
                meter.add_mark(64.0, gtk::PositionType::Bottom, None);
                meter.add_mark(128.0, gtk::PositionType::Bottom, Some("Chat"));
                meter.add_css_class("balance");
                let row = adw::ActionRow::builder().name(control.name()).title(view.title).build();
                row.add_suffix(&meter);
                Row::Balance(row, meter)
            }
            Widget::Button(label) => {
                let button = gtk::Button::builder()
                    .label(*label)
                    .name(control.name())
                    .valign(gtk::Align::Center)
                    .css_classes(["pill"])
                    .build();
                button.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.input(control, Input::Press);
                    }
                });
                let row = adw::ActionRow::builder().name(format!("{}-row", control.name())).title(view.title).build();
                row.add_suffix(&button);
                row.set_activatable_widget(Some(&button));
                Row::Button(row)
            }
        }
    }

    fn update_rows(&self, view: &DeviceView) {
        self.syncing.set(true);
        let rows = self.rows.borrow();
        for control in view.sections.iter().flat_map(|section| &section.controls) {
            match (rows.rows.get(&control.control), &control.widget) {
                (Some(Row::Switch(row)), Widget::Switch(on)) => {
                    row.set_active(*on);
                    row.set_sensitive(control.sensitive);
                }
                (Some(Row::Level(row, scale)), Widget::Level(level)) => {
                    row.set_sensitive(control.sensitive);
                    if let Some(level) = level
                        && !self.level_timers.borrow().contains_key(&control.control)
                    {
                        scale.set_value((*level).into());
                    }
                }
                (Some(Row::Choice(row)), Widget::Choice { options, selected }) => {
                    row.set_sensitive(control.sensitive);
                    // new choices (e.g. an auto-off time set elsewhere) replace the old ones in place
                    let shown: Vec<String> = row
                        .model()
                        .map(|model| {
                            (0..model.n_items())
                                .filter_map(|i| model.item(i).and_downcast::<gtk::StringObject>())
                                .map(|item| item.string().to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    if shown != *options {
                        let options: Vec<&str> = options.iter().map(String::as_str).collect();
                        row.set_model(Some(&gtk::StringList::new(&options)));
                    }
                    if let Some(selected) = selected {
                        row.set_selected(*selected);
                    }
                }
                (Some(Row::Info(row)), Widget::Info(text)) => row.set_subtitle(text),
                (Some(Row::Balance(row, meter)), Widget::Balance(value)) => {
                    row.set_subtitle(&value.map(crate::cli::chatmix_text).unwrap_or_default());
                    meter.set_value(value.map_or(64.0, f64::from));
                }
                (Some(Row::Button(row)), Widget::Button(_)) => row.set_sensitive(control.sensitive),
                (Some(Row::MicTest(row, _, button)), Widget::MicTest) if !self.mic_testing.get() => {
                    row.set_subtitle(control.subtitle.unwrap_or_default());
                    button.set_sensitive(control.sensitive);
                }
                // a band still being dragged keeps where the user put it
                (Some(Row::Curve(_, editor)), Widget::Curve(curve))
                    if !self.level_timers.borrow().contains_key(&control.control) =>
                {
                    editor.show(curve);
                }
                _ => {}
            }
        }
        self.syncing.set(false);
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
