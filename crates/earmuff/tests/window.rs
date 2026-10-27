//! End to end: the real window, driven like a user would, against a simulated headset.
//! Runs headless (see tests/support), so nothing appears on the desktop.

mod support;

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use earmuff_application::Backend;
use earmuff_domain::{Gain, NoiseFilter, Percent};
use earmuff_infrastructure::demo::{DemoBackend, DemoHeadset, FullHeadset};
use support::{Outcome, ensure, find, page, run_app, until, window};
use zbus::zvariant::OwnedValue;

/// Stands in for the desktop's notification server and records each title. GLib may
/// use either the freedesktop or the GTK protocol, so it speaks both.
#[derive(Clone, Default)]
struct Notifications {
    titles: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app: String,
        _replaces: u32,
        _icon: String,
        summary: String,
        _body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        _timeout: i32,
    ) -> u32 {
        self.titles.lock().unwrap().push(summary);
        1
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("fake".into(), "earmuff tests".into(), "1".into(), "1.2".into())
    }

    fn close_notification(&self, _id: u32) {}
}

#[derive(Clone)]
struct GtkNotifications(Notifications);

#[zbus::interface(name = "org.gtk.Notifications")]
impl GtkNotifications {
    fn add_notification(&self, _app: String, _id: String, notification: HashMap<String, OwnedValue>) {
        let title = notification.get("title").and_then(|v| String::try_from(v.try_clone().ok()?).ok());
        self.0.titles.lock().unwrap().push(title.unwrap_or_default());
    }

    fn remove_notification(&self, _app: String, _id: String) {}
}

fn main() -> ExitCode {
    support::isolated("window", || {
        let notifications = Notifications::default();
        let _server = zbus::block_on(async {
            zbus::connection::Builder::session()?
                .name("org.freedesktop.Notifications")?
                .name("org.gtk.Notifications")?
                .serve_at("/org/freedesktop/Notifications", notifications.clone())?
                .serve_at("/org/gtk/Notifications", GtkNotifications(notifications.clone()))?
                .build()
                .await
        })
        .map_err(|e| format!("fake notification server: {e}"))?;
        seed_battery_history()?;
        let demo = DemoBackend::new();
        let headset = demo.headsets()[0].clone();
        let other = demo.full_headset().clone();
        let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(demo)];
        run_app(backends, false, move |app| scenario(app, headset, other, notifications))
    })
}

/// Three hours of a draining Cloud Alpha Wireless, so the window can estimate the time left
fn seed_battery_history() -> Outcome {
    let state = std::env::var("XDG_STATE_HOME").map_err(|e| e.to_string())?;
    let dir = std::path::Path::new(&state).join("earmuff");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs();
    let readings: String = [(3 * 3600, 100), (2 * 3600, 95), (3600, 90), (600, 82)]
        .iter()
        .map(|(ago, level)| format!("{},{level},0\n", now - ago))
        .collect();
    std::fs::write(dir.join("battery-03f0-098d.csv"), readings).map_err(|e| e.to_string())
}

async fn scenario(
    app: adw::Application,
    headset: DemoHeadset,
    other: FullHeadset,
    notifications: Notifications,
) -> Outcome {
    let window = window(&app).await?;
    let title: adw::WindowTitle = find(&window, "title")?;
    let battery: gtk::Label = find(&window, "battery-label")?;
    let mute: adw::SwitchRow = find(&window, "mute")?;
    let mic: adw::ActionRow = find(&window, "mic")?;
    let sidetone: adw::SwitchRow = find(&window, "sidetone")?;
    let level_row: adw::ActionRow = find(&window, "level-row")?;
    let level: gtk::Scale = find(&window, "level")?;
    let auto_off: adw::ComboRow = find(&window, "auto-off")?;
    let voice: adw::SwitchRow = find(&window, "voice-prompts")?;
    let missing: adw::StatusPage = find(&window, "missing-page")?;

    // shows the headset's settings as soon as it answers
    until("the settings page", || page(&window) == "settings").await?;
    until("the battery level", || battery.label() == "80%").await?;
    // the color is the last value the headset reports
    until("the color as the subtitle", || title.subtitle() == "Black/Red").await?;
    ensure(title.title() == "Cloud Alpha Wireless (demo)", format!("title is {}", title.title()))?;

    // the banner shows the headset, its state and battery
    let hero_name: gtk::Label = find(&window, "hero-name")?;
    let badge: gtk::Label = find(&window, "badge")?;
    ensure(hero_name.label() == "CLOUD ALPHA WIRELESS (DEMO)", format!("banner says {}", hero_name.label()))?;
    ensure(badge.label() == "CONNECTED", format!("badge says {}", badge.label()))?;

    // the battery history gives a time-left estimate and a graph on the Power tab
    let time_left: gtk::Label = find(&window, "time-left")?;
    until("the time left", || time_left.is_visible() && time_left.label().starts_with("About ")).await?;
    ensure(time_left.label().ends_with("left"), format!("time left says {}", time_left.label()))?;
    ensure(find::<gtk::DrawingArea>(&window, "battery-graph").is_ok(), "a battery graph")?;
    find::<gtk::ToggleButton>(&window, "history-week")?.set_active(true);
    find::<gtk::ToggleButton>(&window, "history-day")?.set_active(true);
    headset.with(|sim, now| sim.set_charging(1, now));
    until("the charging badge", || badge.label() == "CHARGING" && badge.has_css_class("charging")).await?;
    headset.with(|sim, now| sim.set_charging(0, now));
    until("the connected badge", || badge.label() == "CONNECTED").await?;

    // the gaming stylesheet parses cleanly
    let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let provider = gtk::CssProvider::new();
    let seen = errors.clone();
    provider.connect_parsing_error(move |_, section, error| seen.borrow_mut().push(format!("{section}: {error}")));
    provider.load_from_resource(&format!("{}/style.css", earmuff::ui::RESOURCE_PATH));
    ensure(errors.borrow().is_empty(), format!("stylesheet errors: {:?}", errors.borrow()))?;

    // Preferences (Ctrl+,) changes the accent color, and remembers it
    app.activate_action("preferences", None);
    until("the preferences dialog", || find::<adw::ComboRow>(&window, "accent").is_ok()).await?;
    find::<adw::ComboRow>(&window, "accent")?.set_selected(1);
    until("the cyan accent", || window.has_css_class("accent-cyan")).await?;
    let saved =
        std::env::var("XDG_CONFIG_HOME").map(|dir| format!("{dir}/earmuff/preferences.json")).unwrap_or_default();
    let saved = std::fs::read_to_string(&saved).unwrap_or_default();
    ensure(saved.contains("\"cyan\""), format!("preferences file says {saved:?}"))?;
    // ...and starting at login writes an autostart entry that starts hidden
    let entry = std::env::var("XDG_CONFIG_HOME")
        .map(|dir| std::path::PathBuf::from(dir).join("autostart/io.github.xlizer1.Earmuff.desktop"))
        .unwrap_or_default();
    let login: adw::SwitchRow = find(&window, "start-on-login")?;
    ensure(!login.is_active(), "start on login is off at first")?;
    login.set_active(true);
    until("the autostart entry", || std::fs::read_to_string(&entry).is_ok_and(|text| text.contains("--hidden")))
        .await?;
    login.set_active(false);
    until("the autostart entry to go", || !entry.exists()).await?;
    find::<adw::PreferencesDialog>(&window, "preferences")?.close();
    until("the dialog to close", || find::<adw::PreferencesDialog>(&window, "preferences").is_err()).await?;

    // About (F1) names the app and its license
    app.activate_action("about", None);
    until("the about dialog", || find::<adw::AboutDialog>(&window, "about").is_ok()).await?;
    let about: adw::AboutDialog = find(&window, "about")?;
    ensure(about.application_name() == "Earmuff", "about names the app")?;
    ensure(about.license_type() == gtk::License::Gpl30, "about says GPL-3.0-or-later")?;
    about.close();
    until("the about dialog to close", || find::<adw::AboutDialog>(&window, "about").is_err()).await?;
    ensure(mic.subtitle().as_deref() == Some("Attached"), "mic boom attached")?;
    ensure(!mute.is_active() && !sidetone.is_active() && voice.is_active(), "switches match the headset")?;
    ensure(!level_row.is_sensitive(), "level is greyed out while monitoring is off")?;
    ensure(selected_text(&auto_off) == "20 minutes", "auto-off shows 20 minutes")?;

    // the mute switch mutes the headset
    mute.set_active(true);
    until("the headset to mute", || headset.with(|sim, _| sim.muted)).await?;

    // the mute button on the headset flips the switch
    headset.with(|sim, now| sim.press_mute_button(now));
    until("the switch to follow the button", || !mute.is_active()).await?;

    // mic monitoring and its level
    sidetone.set_active(true);
    until("monitoring to turn on", || headset.with(|sim, _| sim.sidetone)).await?;
    until("the level to become adjustable", || level_row.is_sensitive()).await?;
    level.set_value(100.0);
    until("the level to reach the headset", || headset.with(|sim, _| sim.sidetone_level) == 18).await?;
    level.set_value(50.0);
    until("the level to reach the headset", || headset.with(|sim, _| sim.sidetone_level) == 7).await?;

    // auto-off and voice prompts
    let hour = choice_index(&auto_off, "1 hour")?;
    auto_off.set_selected(hour);
    until("auto-off to reach the headset", || headset.with(|sim, _| sim.auto_off) == 60).await?;
    voice.set_active(false);
    until("voice prompts to turn off", || !headset.with(|sim, _| sim.voice_prompts)).await?;

    // battery news from the headset
    headset.with(|sim, now| sim.set_battery(15, now));
    until("the new battery level", || battery.label() == "15%").await?;
    // at 15% it's below the default 20% mark: a desktop notification says so
    until("the low-battery notification", || {
        notifications.titles.lock().unwrap().iter().any(|title| title == "Headset battery low")
    })
    .await?;

    // only the controls the headset has: the Cloud Alpha has no lights
    ensure(find::<adw::SwitchRow>(&window, "lights").is_err(), "no lights control on the Cloud Alpha")?;
    // and what it is
    let usb: adw::ActionRow = find(&window, "usb-id")?;
    let connection: adw::ActionRow = find(&window, "connection")?;
    ensure(usb.subtitle().as_deref() == Some("03f0:098d"), "shows the USB ID")?;
    ensure(connection.subtitle().as_deref() == Some("Simulated (demo)"), "shows how it's connected")?;

    // the sidebar switches to the demo headset with every feature
    let list: gtk::ListBox = find(&window, "device-list")?;
    until("both headsets in the sidebar", || list.row_at_index(1).is_some()).await?;
    list.select_row(list.row_at_index(1).as_ref());
    until("the other headset", || title.title() == "Earmuff Demo Headset").await?;
    until("its settings", || find::<adw::SwitchRow>(&window, "lights").is_ok()).await?;
    let lights: adw::SwitchRow = find(&window, "lights")?;
    let noise: adw::ComboRow = find(&window, "noise-filter")?;
    let preset: adw::ComboRow = find(&window, "eq-preset")?;
    let mic_volume: gtk::Scale = find(&window, "mic-volume")?;
    let chatmix: adw::ActionRow = find(&window, "chatmix")?;
    until("the lights to show as on", || lights.is_active()).await?;
    ensure(selected_text(&preset) == "Flat", "starts on the flat preset")?;

    lights.set_active(false);
    until("the lights to turn off", || other.status().lights == Some(false)).await?;
    noise.set_selected(2);
    until("the noise filter to go high", || other.status().noise_filter == Some(NoiseFilter::High)).await?;
    preset.set_selected(1);
    until("the bass boost preset", || other.status().equalizer_preset == Some(1)).await?;

    // the equalizer shows the preset's curve band by band; moving a band makes it the user's own
    let low: gtk::Scale = find(&window, "eq-band-0")?;
    until("the bass boost curve", || low.value() == 6.0).await?;
    let high: gtk::Scale = find(&window, "eq-band-9")?;
    high.set_value(3.2);
    let own: Vec<Gain> = [60, 40, 20, 0, 0, 0, 0, 0, 0, 30].map(Gain::tenths).to_vec();
    until("the curve to reach the headset", || other.status().equalizer.as_ref() == Some(&own)).await?;
    until("the preset to show as custom", || selected_text(&preset) == "Custom").await?;
    find::<gtk::Button>(&window, "equalizer-flat")?.emit_clicked();
    until("a flat curve", || other.status().equalizer == Some(vec![Gain::tenths(0); 10])).await?;
    until("the flat preset", || selected_text(&preset) == "Flat").await?;
    mic_volume.set_value(30.0);
    until("the mic volume", || other.status().mic_volume == Some(Percent::saturating(30))).await?;

    // turning the game/chat dial on the headset updates the window
    other.set_chatmix(20);
    until("the ChatMix reading", || chatmix.subtitle().as_deref() == Some("20 (more game)")).await?;
    let meter: gtk::Scale = find(&window, "chatmix-meter")?;
    ensure(meter.value() == 20.0, "the balance meter follows the dial")?;

    // the headset plays its notification sound on request
    let sound: gtk::Button = find(&window, "notification-sound")?;
    sound.emit_clicked();
    until("the notification sound", || other.played_sounds() == [0]).await?;

    // the headset forgets its settings while off; Earmuff puts them back when it's on
    other.set_powered(false);
    until("the headset-is-off page", || page(&window) == "off").await?;
    other.forget_settings();
    ensure(other.status().lights == Some(true), "the headset forgot the lights setting")?;
    other.set_powered(true);
    until("the remembered settings to come back", || {
        let status = other.status();
        status.lights == Some(false)
            && status.noise_filter == Some(NoiseFilter::High)
            && status.equalizer_preset == Some(1)
    })
    .await?;
    let profiles =
        std::env::var("XDG_CONFIG_HOME").map(|dir| format!("{dir}/earmuff/headsets.json")).unwrap_or_default();
    ensure(
        std::fs::read_to_string(profiles).is_ok_and(|text| text.contains("f00b:a00d")),
        "settings saved per headset",
    )?;

    // and back to the Cloud Alpha
    list.select_row(list.row_at_index(0).as_ref());
    until("the Cloud Alpha again", || title.title() == "Cloud Alpha Wireless (demo)").await?;

    // the headset turns off; a setting changes meanwhile; it turns back on
    headset.with(|sim, now| sim.set_powered(false, now));
    until("the headset-is-off page", || page(&window) == "off").await?;
    headset.with(|sim, _| sim.auto_off = 45);
    headset.with(|sim, now| sim.set_powered(true, now));
    until("the settings page again", || page(&window) == "settings").await?;
    // switching headsets rebuilt the page, so look the row up again
    until("the auto-off set while it was off", || {
        find::<adw::ComboRow>(&window, "auto-off").is_ok_and(|row| selected_text(&row) == "45 minutes")
    })
    .await?;

    // the dongle is unplugged: the window moves on to the other headset
    headset.unplug();
    until("the other headset", || title.title() == "Earmuff Demo Headset").await?;
    // with both unplugged it asks for a dongle
    other.unplug();
    until("the plug-in page", || page(&window) == "missing").await?;
    ensure(missing.description().as_deref() == Some(earmuff::ui::view_model::LOOKING), "says it is looking")?;
    headset.plug();
    other.plug();
    until("the settings page after replugging", || page(&window) == "settings").await?;

    // without a tray icon, closing the window quits
    window.close();
    Ok(())
}

fn selected_text(combo: &adw::ComboRow) -> String {
    combo.selected_item().and_downcast::<gtk::StringObject>().map(|item| item.string().to_string()).unwrap_or_default()
}

fn choice_index(combo: &adw::ComboRow, text: &str) -> Result<u32, String> {
    let model = combo.model().ok_or("auto-off has no choices")?;
    (0..model.n_items())
        .find(|&i| model.item(i).and_downcast::<gtk::StringObject>().is_some_and(|item| item.string() == text))
        .ok_or_else(|| format!("no auto-off choice {text}"))
}
