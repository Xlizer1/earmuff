//! The main window. It shows what the [`Model`] says and sends the user's changes to
//! the device manager, which talks to the headset on its own thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use earmuff_application::{AppEvent, Backend, DeviceManager, Preferences, PreferencesStore};
use earmuff_domain::{DeviceId, Setting};
use gtk::gio;
use gtk::glib::{self, SourceId};
use ksni::TrayMethods;

use super::UiOptions;
use super::view_model::{
    ACCENTS, Control, ControlView, DeviceView, Input, Model, Page, Widget, accent_class, accent_index,
};
use crate::tray::{HeadsetTray, TrayEvent, TrayState};

/// Builds the window, starts looking for headsets and shows it
pub(super) fn open(app: &adw::Application, backends: Vec<Arc<dyn Backend>>, options: &UiOptions) {
    // the gaming look is dark, whatever the desktop's style
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    let ui = Ui::new(app, options.preferences.clone());
    ui.connect_signals();
    ui.install_actions();
    ui.apply_accent();
    if options.tray {
        ui.start_tray();
    }
    ui.start_manager(backends, options);
    ui.render();
    ui.window.present();
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

struct BatteryRow {
    row: adw::ActionRow,
    bar: gtk::LevelBar,
    label: gtk::Label,
}

/// The settings page as currently built
#[derive(Default)]
struct Rows {
    device: Option<DeviceId>,
    controls: Vec<Control>,
    battery: Option<BatteryRow>,
    rows: HashMap<Control, Row>,
}

struct Ui {
    app: adw::Application,
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    picker: gtk::DropDown,
    picker_ids: RefCell<Vec<DeviceId>>,
    toasts: adw::ToastOverlay,
    stack: gtk::Stack,
    missing_page: adw::StatusPage,
    /// Holds the settings page, which is rebuilt when the headset or its controls change
    settings: adw::Bin,
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
}

impl Ui {
    fn new(app: &adw::Application, store: Arc<dyn PreferencesStore>) -> Rc<Ui> {
        let title = adw::WindowTitle::new("Earmuff", "");
        title.set_widget_name("title");
        let picker = gtk::DropDown::builder()
            .name("device-picker")
            .model(&gtk::StringList::new(&[]))
            .tooltip_text("Headset")
            .visible(false)
            .build();
        let menu = gio::Menu::new();
        menu.append(Some("Preferences"), Some("app.preferences"));
        menu.append(Some("Quit"), Some("app.quit"));
        let menu_button = gtk::MenuButton::builder()
            .name("main-menu")
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Menu")
            .build();
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&picker);
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
        let settings = adw::Bin::builder().name("settings-page").build();

        let stack = gtk::Stack::builder().name("pages").transition_type(gtk::StackTransitionType::Crossfade).build();
        stack.add_named(&missing_page, Some("missing"));
        stack.add_named(&off_page, Some("off"));
        stack.add_named(&settings, Some("settings"));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));

        let view = adw::ToolbarView::builder().content(&toasts).build();
        view.add_top_bar(&header);
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Earmuff")
            .default_width(460)
            .default_height(660)
            .content(&view)
            .build();

        Rc::new(Ui {
            app: app.clone(),
            window,
            title,
            picker,
            picker_ids: RefCell::default(),
            toasts,
            stack,
            missing_page,
            settings,
            rows: RefCell::default(),
            model: RefCell::new(Model::default()),
            manager: RefCell::new(None),
            syncing: Cell::new(false),
            level_timers: RefCell::default(),
            tray: RefCell::new(None),
            tray_online: Cell::new(false),
            hold: RefCell::new(None),
            preferences: RefCell::new(store.load()),
            store,
        })
    }

    // --- app actions and preferences --------------------------------------

    fn install_actions(self: &Rc<Self>) {
        let actions: [AppAction; 2] =
            [("preferences", "<Control>comma", Ui::show_preferences), ("quit", "<Control>q", |ui| ui.quit())];
        for (name, accel, run) in actions {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(ui) = weak.upgrade() {
                    run(&ui);
                }
            });
            self.app.add_action(&action);
            self.app.set_accels_for_action(&format!("app.{name}"), &[accel]);
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
        let page = adw::PreferencesPage::builder().title("General").icon_name("preferences-system-symbolic").build();
        page.add(&appearance);
        let dialog = adw::PreferencesDialog::builder().name("preferences").build();
        dialog.add(&page);
        dialog.present(Some(&self.window));
    }

    /// Changes the preferences and saves them
    fn update_preferences(&self, change: impl FnOnce(&mut Preferences)) {
        change(&mut self.preferences.borrow_mut());
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
        self.picker.connect_selected_notify(move |picker| {
            let Some(ui) = weak.upgrade() else { return };
            if ui.syncing.get() {
                return;
            }
            let id = ui.picker_ids.borrow().get(picker.selected() as usize).cloned();
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
            }
        });
    }

    fn render(self: &Rc<Self>) {
        self.sync_picker();
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
        let rebuild = {
            let rows = self.rows.borrow();
            let controls: Vec<Control> = view.layout().into_iter().map(|(control, _)| control).collect();
            rows.device != device || rows.controls != controls || rows.battery.is_some() != view.has_battery
        };
        if rebuild {
            self.build_rows(&view, device);
        }
        self.update_rows(&view);
    }

    /// Lists the connected headsets in the picker, which shows when there are several
    fn sync_picker(&self) {
        let model = self.model.borrow();
        let ids: Vec<DeviceId> = model.devices().map(|info| info.id.clone()).collect();
        self.syncing.set(true);
        if *self.picker_ids.borrow() != ids {
            let names: Vec<String> = model.devices().map(|info| info.name.clone()).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            self.picker.set_model(Some(&gtk::StringList::new(&names)));
            self.picker_ids.replace(ids.clone());
        }
        self.picker.set_visible(ids.len() > 1);
        if let Some(index) = model.current().and_then(|(info, _)| ids.iter().position(|id| *id == info.id)) {
            self.picker.set_selected(index as u32);
        }
        self.syncing.set(false);
    }

    fn build_rows(self: &Rc<Self>, view: &DeviceView, device: Option<DeviceId>) {
        for (_, timer) in self.level_timers.borrow_mut().drain() {
            timer.remove();
        }
        let page = adw::PreferencesPage::new();
        let battery = view.has_battery.then(|| {
            let bar = gtk::LevelBar::builder()
                .name("battery-bar")
                .min_value(0.0)
                .max_value(100.0)
                .valign(gtk::Align::Center)
                .width_request(120)
                .build();
            let label = gtk::Label::builder().name("battery-label").width_chars(4).xalign(1.0).build();
            let row = adw::ActionRow::builder().name("battery").title("Battery").build();
            row.add_suffix(&bar);
            row.add_suffix(&label);
            let group = adw::PreferencesGroup::new();
            group.add(&row);
            page.add(&group);
            BatteryRow { row, bar, label }
        });
        let mut rows = HashMap::new();
        for section in &view.sections {
            let group = adw::PreferencesGroup::builder().title(section.title).build();
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
            page.add(&group);
        }
        self.settings.set_child(Some(&page));
        let controls = view.layout().into_iter().map(|(control, _)| control).collect();
        self.rows.replace(Rows { device, controls, battery, rows });
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
        self.title.set_subtitle(&view.subtitle);
        let rows = self.rows.borrow();
        if let Some(battery) = &rows.battery {
            if let Some(level) = view.battery {
                battery.bar.set_value(level.into());
            }
            battery.label.set_label(&view.battery_text);
            battery.row.set_subtitle(&view.charge_text);
        }
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
