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
    Alert, AppEvent, Autostart, Backend, BatteryAlerts, DeviceManager, Preferences, PreferencesStore,
};
use earmuff_domain::{DeviceId, Setting};
use gtk::gio;
use gtk::glib::{self, SourceId};
use ksni::TrayMethods;

use super::battery_ring::BatteryRing;
use super::view_model::{
    ACCENTS, BATTERY_ALERTS, Control, ControlView, DeviceView, Input, Model, Page, SidebarItem, Widget, accent_class,
    accent_index, alert_text, autostart_hint, battery_alert_index, battery_alert_label,
};
use super::{APP_ID, RESOURCE_PATH, UiOptions};
use crate::tray::{HeadsetTray, TrayEvent, TrayState};

/// Builds the window, starts looking for headsets and shows it
pub(super) fn open(app: &adw::Application, backends: Vec<Arc<dyn Backend>>, options: &UiOptions) {
    // the gaming look is dark, whatever the desktop's style
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    let ui = Ui::new(app, options.preferences.clone(), options.autostart.clone());
    ui.connect_signals();
    ui.install_actions();
    ui.apply_accent();
    if options.tray {
        ui.start_tray();
    }
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
}

/// The tabs as currently built
#[derive(Default)]
struct Rows {
    device: Option<DeviceId>,
    controls: Vec<Control>,
    pages: Vec<gtk::Widget>,
    rows: HashMap<Control, Row>,
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
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        text.append(&name);
        text.append(&detail);
        text.append(&badge);
        let ring = BatteryRing::new();
        let widget = gtk::Box::builder().name("hero").spacing(28).css_classes(["hero"]).build();
        widget.append(&art);
        widget.append(&text);
        widget.append(&ring.widget);
        Hero { widget, name, detail, badge, ring }
    }

    fn show(&self, view: &DeviceView) {
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
    alerts: RefCell<BatteryAlerts>,
}

impl Ui {
    fn new(app: &adw::Application, store: Arc<dyn PreferencesStore>, autostart: Arc<dyn Autostart>) -> Rc<Ui> {
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
            alerts: RefCell::new(BatteryAlerts::new(store.load().alerts())),
            preferences: RefCell::new(store.load()),
            store,
            autostart,
        })
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
        let notifications = adw::PreferencesGroup::builder().title("Notifications").build();
        notifications.add(&low);
        notifications.add(&charged);

        let page = adw::PreferencesPage::builder().title("General").icon_name("preferences-system-symbolic").build();
        page.add(&appearance);
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
    fn input(&self, control: Control, input: Input) {
        if self.syncing.get() {
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
    fn schedule_level(self: &Rc<Self>, control: Control) {
        if let Some(id) = self.level_timers.borrow_mut().remove(&control) {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(150), move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.level_timers.borrow_mut().remove(&control);
            let value = match ui.rows.borrow().rows.get(&control) {
                Some(Row::Level(_, scale)) => scale.value().round().clamp(0.0, 100.0) as u8,
                _ => return,
            };
            ui.input(control, Input::Level(value));
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
                let changed = ui.model.borrow_mut().apply(&event);
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
        self.hero.show(&view);
        let rebuild = {
            let rows = self.rows.borrow();
            let controls: Vec<Control> = view.layout().into_iter().map(|(control, _)| control).collect();
            rows.device != device || rows.controls != controls
        };
        if rebuild {
            self.build_tabs(&view, device);
        }
        self.update_rows(&view);
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
        for tab in view.tabs() {
            let page = gtk::Box::builder().name(tab.name()).orientation(gtk::Orientation::Vertical).spacing(24).build();
            for section in view.sections.iter().filter(|section| section.tab == tab) {
                let group = adw::PreferencesGroup::builder().title(section.title.to_uppercase()).build();
                group.set_description(section.description);
                for control in &section.controls {
                    let row = self.build_row(control);
                    group.add(match &row {
                        Row::Switch(row) => row.upcast_ref::<gtk::Widget>(),
                        Row::Level(row, _) | Row::Info(row) => row.upcast_ref(),
                        Row::Choice(row) => row.upcast_ref(),
                    });
                    rows.insert(control.control, row);
                }
                page.append(&group);
            }
            self.tabs.add_titled_with_icon(&page, Some(tab.name()), tab.title(), tab.icon());
            pages.push(page.upcast());
        }
        let controls = view.layout().into_iter().map(|(control, _)| control).collect();
        self.rows.replace(Rows { device, controls, pages, rows });
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
                        ui.schedule_level(control);
                    }
                });
                let row = adw::ActionRow::builder().name(format!("{}-row", control.name())).title(view.title).build();
                row.add_suffix(&scale);
                Row::Level(row, scale)
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
