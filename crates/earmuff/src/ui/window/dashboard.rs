//! The dashboard: one page of panels, placed in a grid by the view model
//! ([`DeviceView::placements`]), the headset switcher in the header, and the widgets for
//! each control.

use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use earmuff_application::time_left_text;
use earmuff_domain::DeviceId;

use super::{Ui, now};
use crate::ui::battery_graph::{BatteryGraph, DAY, WEEK};
use crate::ui::battery_ring::BatteryRing;
use crate::ui::equalizer::EqualizerEditor;
use crate::ui::view_model::{Control, ControlView, DeviceView, Input, Panel, Placement, SectionView, Widget};

/// Controls held at the bottom of their panel, so panels that share a row's height
/// keep their last row level instead of leaving a gap under it
const PINNED: [Control; 2] = [Control::MicTest, Control::SidetoneLevel];

/// The widgets of one control
pub(super) enum Row {
    // rows in a panel's list
    Switch(adw::SwitchRow),
    Choice(adw::ComboRow),
    Info(adw::ActionRow),
    Button(adw::ActionRow),
    MicTest(adw::ActionRow, gtk::LevelBar, gtk::Button),
    // on their own in a panel
    Level(gtk::Box, gtk::Scale, gtk::Label),
    Segmented(gtk::Box, Vec<gtk::ToggleButton>),
    MuteTile(MuteTile),
    Balance(gtk::Box, gtk::Label, gtk::Scale),
    Curve(Rc<EqualizerEditor>),
    // in a panel's header
    HeaderSwitch(gtk::Switch),
    HeaderLabel(gtk::Label),
}

impl Row {
    fn widget(&self) -> &gtk::Widget {
        match self {
            Row::Switch(row) => row.upcast_ref(),
            Row::Choice(row) => row.upcast_ref(),
            Row::Info(row) | Row::Button(row) | Row::MicTest(row, ..) => row.upcast_ref(),
            Row::Level(row, ..) | Row::Segmented(row, _) | Row::Balance(row, ..) => row.upcast_ref(),
            Row::MuteTile(tile) => tile.button.upcast_ref(),
            Row::Curve(editor) => editor.widget.upcast_ref(),
            Row::HeaderSwitch(switch) => switch.upcast_ref(),
            Row::HeaderLabel(label) => label.upcast_ref(),
        }
    }

    fn in_list(&self) -> bool {
        matches!(self, Row::Switch(_) | Row::Choice(_) | Row::Info(_) | Row::Button(_) | Row::MicTest(..))
    }

    fn in_header(&self) -> bool {
        matches!(self, Row::HeaderSwitch(_) | Row::HeaderLabel(_))
    }
}

/// The big mute button: the same size muted or not, so nothing moves when it changes
pub(super) struct MuteTile {
    button: gtk::ToggleButton,
    icon: gtk::Image,
    title: gtk::Label,
    hint: gtk::Label,
}

impl MuteTile {
    fn show(&self, muted: bool) {
        self.button.set_active(muted);
        self.title.set_label(if muted { "Muted" } else { "Mic on" });
        self.hint.set_label(if muted { "Select to unmute" } else { "Select to mute" });
        self.icon.set_icon_name(Some(if muted {
            "microphone-disabled-symbolic"
        } else {
            "audio-input-microphone-symbolic"
        }));
    }
}

/// The battery panel's widgets
pub(super) struct StatusPanel {
    ring: BatteryRing,
    badge: gtk::Label,
    time_left: gtk::Label,
    detail: gtk::Label,
}

impl StatusPanel {
    fn show(&self, view: &DeviceView, time_left: Option<String>) {
        self.badge.set_label(view.badge);
        for (class, on) in [("charging", view.charging), ("off", view.badge == "Off")] {
            if on {
                self.badge.add_css_class(class);
            } else {
                self.badge.remove_css_class(class);
            }
        }
        // an empty line until there's an estimate, so the panel keeps its height
        self.time_left.set_label(&time_left.unwrap_or_default());
        self.detail.set_label(&view.detail);
        self.ring.show(view.battery, &view.battery_text, &view.charge_text, view.battery_low, view.charging);
    }
}

/// The dashboard as currently built
#[derive(Default)]
pub(super) struct Rows {
    device: Option<DeviceId>,
    controls: Vec<Control>,
    has_battery: bool,
    panels: Vec<(Panel, gtk::Widget)>,
    wide: Vec<Placement>,
    narrow: Vec<Placement>,
    pub(super) rows: HashMap<Control, Row>,
    graph: Option<BatteryGraph>,
    status: Option<StatusPanel>,
}

/// An empty panel: a header with the title, then room for what goes in it
fn panel(panel: Panel, title: &str) -> (gtk::Box, gtk::Box) {
    let header = gtk::Box::builder().spacing(12).css_classes(["panel-header"]).build();
    header.append(&gtk::Label::builder().label(title).css_classes(["panel-title"]).xalign(0.0).hexpand(true).build());
    let widget = gtk::Box::builder()
        .name(panel.name())
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .css_classes(["panel"])
        .build();
    // what's inside may grow to fill the panel, but the panel never stretches the grid
    widget.set_vexpand(false);
    widget.append(&header);
    (widget, header)
}

/// Pushes what comes after it to the bottom of its panel
fn spacer() -> gtk::Box {
    gtk::Box::builder().vexpand(true).build()
}

fn list() -> gtk::ListBox {
    gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["panel-rows"]).build()
}

impl Ui {
    /// Shows the headset in the dashboard, building its panels when it's a new headset
    /// or has other controls than before
    pub(super) fn show_dashboard(self: &Rc<Self>, view: &DeviceView, device: Option<DeviceId>) {
        let rebuild = {
            let rows = self.rows.borrow();
            let controls: Vec<Control> = view.layout().into_iter().map(|(control, _)| control).collect();
            rows.device != device || rows.controls != controls || rows.has_battery != view.has_battery
        };
        if rebuild {
            self.build_dashboard(view, device.clone());
        }
        let now = now();
        let time_left = device.as_ref().and_then(|id| self.history.borrow().minutes_left(id, now)).map(time_left_text);
        let rows = self.rows.borrow();
        if let Some(status) = &rows.status {
            status.show(view, time_left);
        }
        if let (Some(graph), Some(id)) = (&rows.graph, &device) {
            graph.show(self.history.borrow().samples(id), now);
        }
        drop(rows);
        self.update_rows(view);
    }

    fn build_dashboard(self: &Rc<Self>, view: &DeviceView, device: Option<DeviceId>) {
        for (_, timer) in self.level_timers.borrow_mut().drain() {
            timer.remove();
        }
        let mut rows = HashMap::new();
        let graph = view.has_battery.then(BatteryGraph::new);
        let mut status = None;
        let mut panels = Vec::new();
        for kind in view.panels() {
            let widget = match (kind, &graph) {
                (Panel::Battery, _) => {
                    let (widget, built) = self.battery_panel();
                    status = Some(built);
                    widget
                }
                (Panel::History, Some(graph)) => self.history_panel(graph),
                (kind, _) => match view.section(kind) {
                    Some(section) => self.section_panel(section, &mut rows),
                    None => continue,
                },
            };
            panels.push((kind, widget.upcast()));
        }
        let controls = view.layout().into_iter().map(|(control, _)| control).collect();
        self.rows.replace(Rows {
            device,
            controls,
            has_battery: view.has_battery,
            panels,
            wide: view.placements(false),
            narrow: view.placements(true),
            rows,
            graph,
            status,
        });
        self.place_panels();
    }

    /// Puts the panels in the grid: in one column on narrow windows
    pub(super) fn place_panels(&self) {
        while let Some(child) = self.dashboard.first_child() {
            self.dashboard.remove(&child);
        }
        let rows = self.rows.borrow();
        let placements = if self.narrow.get() { &rows.narrow } else { &rows.wide };
        for placement in placements {
            if let Some((_, widget)) = rows.panels.iter().find(|(panel, _)| *panel == placement.panel) {
                self.dashboard.attach(widget, placement.column, placement.row, placement.width, placement.height);
            }
        }
    }

    fn battery_panel(&self) -> (gtk::Box, StatusPanel) {
        let (widget, _) = panel(Panel::Battery, "Battery");
        let ring = BatteryRing::new();
        let badge = gtk::Label::builder().name("badge").css_classes(["status-badge"]).halign(gtk::Align::Start).build();
        let time_left = gtk::Label::builder()
            .name("time-left")
            .css_classes(["dim-label"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let detail = gtk::Label::builder()
            .name("device-detail")
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        text.append(&badge);
        text.append(&time_left);
        text.append(&detail);
        let body = gtk::Box::builder().spacing(20).vexpand(true).valign(gtk::Align::Center).build();
        body.append(&ring.widget);
        body.append(&text);
        widget.append(&body);
        (widget, StatusPanel { ring, badge, time_left, detail })
    }

    /// The battery graph, with buttons for its span
    fn history_panel(self: &Rc<Self>, graph: &BatteryGraph) -> gtk::Box {
        let (widget, header) = panel(Panel::History, "Battery history");
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
        let buttons = gtk::Box::builder().css_classes(["segmented"]).valign(gtk::Align::Center).build();
        buttons.append(&day);
        buttons.append(&week);
        header.append(&buttons);
        graph.widget.set_vexpand(true);
        widget.append(&graph.widget);
        widget
    }

    /// A panel for one section of controls. Rows that belong in a list share one.
    fn section_panel(self: &Rc<Self>, section: &SectionView, rows: &mut HashMap<Control, Row>) -> gtk::Box {
        let (widget, header) = panel(section.panel, section.title);
        if let Some(text) = section.description {
            let description =
                gtk::Label::builder().label(text).css_classes(["dim-label"]).xalign(0.0).wrap(true).build();
            widget.append(&description);
        }
        let mut current: Option<gtk::ListBox> = None;
        for control in &section.controls {
            let row = self.build_row(control);
            if PINNED.contains(&control.control) {
                widget.append(&spacer());
                current = None;
            }
            if row.in_header() {
                header.append(row.widget());
            } else if row.in_list() {
                current
                    .get_or_insert_with(|| {
                        let rows = list();
                        widget.append(&rows);
                        rows
                    })
                    .append(row.widget());
            } else {
                current = None;
                widget.append(row.widget());
            }
            rows.insert(control.control, row);
        }
        widget
    }

    fn build_row(self: &Rc<Self>, view: &ControlView) -> Row {
        let control = view.control;
        let weak = Rc::downgrade(self);
        match (&view.widget, control) {
            (Widget::Switch(_), Control::Mute) => {
                let icon =
                    gtk::Image::builder().pixel_size(22).valign(gtk::Align::Center).css_classes(["mute-icon"]).build();
                let title = gtk::Label::builder().css_classes(["mute-title"]).xalign(0.0).build();
                let hint = gtk::Label::builder()
                    .css_classes(["dim-label", "caption"])
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                let text = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(2)
                    .valign(gtk::Align::Center)
                    .build();
                text.append(&title);
                text.append(&hint);
                let content = gtk::Box::builder().spacing(14).build();
                content.append(&icon);
                content.append(&text);
                let button = gtk::ToggleButton::builder()
                    .name(control.name())
                    .child(&content)
                    .css_classes(["mute-tile"])
                    .build();
                button.connect_toggled(move |button| {
                    if let Some(ui) = weak.upgrade() {
                        ui.input(control, Input::Switch(button.is_active()));
                    }
                });
                let tile = MuteTile { button, icon, title, hint };
                tile.show(false);
                Row::MuteTile(tile)
            }
            (Widget::Switch(_), Control::Sidetone | Control::SoftwareEq) => {
                let switch = gtk::Switch::builder()
                    .name(control.name())
                    .valign(gtk::Align::Center)
                    .tooltip_text(view.title)
                    .build();
                switch.update_property(&[gtk::accessible::Property::Label(view.title)]);
                switch.connect_active_notify(move |switch| {
                    if let Some(ui) = weak.upgrade() {
                        ui.input(control, Input::Switch(switch.is_active()));
                    }
                });
                Row::HeaderSwitch(switch)
            }
            (Widget::Switch(_), _) => {
                let row = adw::SwitchRow::builder().name(control.name()).title(view.title).build();
                row.set_subtitle(view.subtitle.unwrap_or_default());
                row.connect_active_notify(move |row| {
                    if let Some(ui) = weak.upgrade() {
                        ui.input(control, Input::Switch(row.is_active()));
                    }
                });
                Row::Switch(row)
            }
            (Widget::Level(_), _) => {
                let value = gtk::Label::builder().css_classes(["level-value"]).build();
                let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
                scale.set_widget_name(control.name());
                scale.set_hexpand(true);
                scale.set_round_digits(0);
                scale.update_property(&[gtk::accessible::Property::Label(view.title)]);
                let shown = value.clone();
                scale.connect_value_changed(move |scale| {
                    shown.set_label(&format!("{}%", scale.value().round()));
                    if let Some(ui) = weak.upgrade()
                        && !ui.syncing.get()
                    {
                        ui.schedule_input(control);
                    }
                });
                let top = gtk::Box::builder().spacing(8).build();
                top.append(
                    &gtk::Label::builder()
                        .label(view.title)
                        .css_classes(["row-title"])
                        .xalign(0.0)
                        .hexpand(true)
                        .build(),
                );
                top.append(&value);
                let row = gtk::Box::builder()
                    .name(format!("{}-row", control.name()))
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(6)
                    .build();
                row.append(&top);
                row.append(&scale);
                Row::Level(row, scale, value)
            }
            (Widget::Choice { options, .. }, Control::NoiseFilter | Control::MuteLed) => {
                let buttons: Vec<gtk::ToggleButton> = options
                    .iter()
                    .enumerate()
                    .map(|(index, option)| {
                        gtk::ToggleButton::builder()
                            .label(option.as_str())
                            .name(format!("{}-{index}", control.name()))
                            .hexpand(true)
                            .build()
                    })
                    .collect();
                let strip = gtk::Box::builder().css_classes(["segmented"]).homogeneous(true).build();
                for (index, button) in buttons.iter().enumerate() {
                    if index > 0 {
                        button.set_group(Some(&buttons[0]));
                    }
                    let weak = Rc::downgrade(self);
                    button.connect_toggled(move |button| {
                        if let (true, Some(ui)) = (button.is_active(), weak.upgrade()) {
                            ui.input(control, Input::Choice(index as u32));
                        }
                    });
                    strip.append(button);
                }
                let row =
                    gtk::Box::builder().name(control.name()).orientation(gtk::Orientation::Vertical).spacing(8).build();
                row.append(&gtk::Label::builder().label(view.title).css_classes(["row-title"]).xalign(0.0).build());
                row.append(&strip);
                Row::Segmented(row, buttons)
            }
            (Widget::Choice { options, .. }, _) => {
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
            (Widget::Info(_), Control::MicAttached) => Row::HeaderLabel(
                gtk::Label::builder().name(control.name()).css_classes(["dim-label", "caption"]).build(),
            ),
            (Widget::Info(_), _) => Row::Info(adw::ActionRow::builder().name(control.name()).title(view.title).build()),
            (Widget::Balance(_), _) => {
                // follows the dial on the headset; it can't be set from here
                let meter = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 128.0, 1.0);
                meter.set_widget_name("chatmix-meter");
                meter.set_hexpand(true);
                meter.set_can_target(false);
                meter.set_focusable(false);
                meter.add_mark(64.0, gtk::PositionType::Bottom, None);
                meter.add_css_class("balance");
                let value = gtk::Label::builder().name(control.name()).css_classes(["dim-label", "caption"]).build();
                let top = gtk::Box::builder().spacing(8).build();
                top.append(
                    &gtk::Label::builder()
                        .label(view.title)
                        .css_classes(["row-title"])
                        .xalign(0.0)
                        .hexpand(true)
                        .build(),
                );
                top.append(&value);
                let ends = gtk::Box::builder().css_classes(["dim-label", "caption"]).build();
                ends.append(&gtk::Label::builder().label("Game").hexpand(true).xalign(0.0).build());
                ends.append(&gtk::Label::builder().label("Chat").build());
                let row = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
                row.append(&top);
                row.append(&meter);
                row.append(&ends);
                Row::Balance(row, value, meter)
            }
            (Widget::Button(label), _) => {
                let button =
                    gtk::Button::builder().label(*label).name(control.name()).valign(gtk::Align::Center).build();
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
            (Widget::MicTest, _) => {
                let meter = gtk::LevelBar::builder()
                    .name("mic-level")
                    .min_value(0.0)
                    .max_value(1.0)
                    .width_request(72)
                    .valign(gtk::Align::Center)
                    .build();
                // no "low"/"high" colors: a loud voice isn't a problem
                for offset in [gtk::LEVEL_BAR_OFFSET_LOW, gtk::LEVEL_BAR_OFFSET_HIGH, gtk::LEVEL_BAR_OFFSET_FULL] {
                    meter.remove_offset_value(Some(offset));
                }
                let button =
                    gtk::Button::builder().label("Test").name(control.name()).valign(gtk::Align::Center).build();
                button.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.test_mic();
                    }
                });
                // one line whatever it says, so the panel keeps its height
                let row = adw::ActionRow::builder()
                    .name(format!("{}-row", control.name()))
                    .title(view.title)
                    .subtitle_lines(1)
                    .build();
                row.add_suffix(&meter);
                row.add_suffix(&button);
                Row::MicTest(row, meter, button)
            }
            (Widget::Curve(curve), _) => {
                let editor = EqualizerEditor::new(curve, move || {
                    if let Some(ui) = weak.upgrade()
                        && !ui.syncing.get()
                    {
                        ui.schedule_input(control);
                    }
                });
                editor.widget.set_widget_name(&format!("{}-editor", control.name()));
                Row::Curve(editor)
            }
        }
    }

    fn update_rows(&self, view: &DeviceView) {
        self.syncing.set(true);
        let rows = self.rows.borrow();
        let dragging = |control: Control| self.level_timers.borrow().contains_key(&control);
        for control in view.sections.iter().flat_map(|section| &section.controls) {
            let Some(row) = rows.rows.get(&control.control) else { continue };
            row.widget().set_sensitive(control.sensitive);
            match (row, &control.widget) {
                (Row::Switch(row), Widget::Switch(on)) => row.set_active(*on),
                (Row::HeaderSwitch(switch), Widget::Switch(on)) => switch.set_active(*on),
                (Row::MuteTile(tile), Widget::Switch(on)) => tile.show(*on),
                // a slider still being dragged keeps where the user put it
                (Row::Level(_, scale, value), Widget::Level(level)) if !dragging(control.control) => {
                    if let Some(level) = level {
                        scale.set_value((*level).into());
                    }
                    value.set_label(&level.map(|level| format!("{level}%")).unwrap_or_default());
                }
                (Row::Segmented(_, buttons), Widget::Choice { selected, .. }) => match selected {
                    Some(index) => {
                        if let Some(button) = buttons.get(*index as usize) {
                            button.set_active(true);
                        }
                    }
                    None => buttons.iter().for_each(|button| button.set_active(false)),
                },
                (Row::Choice(row), Widget::Choice { options, selected }) => {
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
                (Row::Info(row), Widget::Info(text)) => row.set_subtitle(text),
                (Row::HeaderLabel(label), Widget::Info(text)) => label.set_label(text),
                (Row::Balance(_, value, meter), Widget::Balance(level)) => {
                    value.set_label(&level.map(crate::cli::chatmix_text).unwrap_or_default());
                    meter.set_value(level.map_or(64.0, f64::from));
                }
                // the row's sensitivity covers a muted mic; the button is only off during a test
                (Row::MicTest(row, _, button), Widget::MicTest) if !self.mic_testing.get() => {
                    row.set_subtitle(control.subtitle.unwrap_or_default());
                    button.set_sensitive(true);
                }
                (Row::Curve(editor), Widget::Curve(curve)) if !dragging(control.control) => editor.show(curve),
                _ => {}
            }
        }
        self.syncing.set(false);
    }

    /// Lists the connected headsets in the header and shows which one the window shows
    pub(super) fn sync_switcher(&self) {
        let model = self.model.borrow();
        let items = model.headsets();
        let labels: Vec<String> = items.iter().map(|item| format!("{} · {}", item.name, item.status)).collect();
        self.syncing.set(true);
        if let Some(list) = self.switcher.model().and_downcast::<gtk::StringList>() {
            let shown: Vec<String> = (0..list.n_items()).filter_map(|i| list.string(i)).map(String::from).collect();
            if shown != labels {
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                list.splice(0, list.n_items(), &labels);
            }
        }
        let ids: Vec<DeviceId> = items.iter().map(|item| item.id.clone()).collect();
        if let Some(index) = model.current().and_then(|(info, _)| ids.iter().position(|id| *id == info.id)) {
            self.switcher.set_selected(index as u32);
        }
        self.switcher_ids.replace(ids);
        // with one headset there's nothing to switch: its name is enough
        self.switcher.set_visible(items.len() > 1);
        self.headset_name.set_visible(items.len() <= 1);
        self.headset_name.set_label(&model.title());
        self.syncing.set(false);
    }
}
