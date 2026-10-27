//! End to end: the real window, driven like a user would, against a simulated headset.
//! Runs headless (see tests/support), so nothing appears on the desktop.

mod support;

use std::process::ExitCode;
use std::sync::Arc;

use adw::prelude::*;
use earmuff_application::Backend;
use earmuff_domain::{NoiseFilter, Percent};
use earmuff_infrastructure::demo::{DemoBackend, DemoHeadset, FullHeadset};
use support::{Outcome, ensure, find, page, run_app, until, window};

fn main() -> ExitCode {
    support::isolated("window", || {
        let demo = DemoBackend::new();
        let headset = demo.headsets()[0].clone();
        let other = demo.full_headset().clone();
        let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(demo)];
        run_app(backends, false, move |app| scenario(app, headset, other))
    })
}

async fn scenario(app: adw::Application, headset: DemoHeadset, other: FullHeadset) -> Outcome {
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

    // only the controls the headset has: the Cloud Alpha has no lights
    ensure(find::<adw::SwitchRow>(&window, "lights").is_err(), "no lights control on the Cloud Alpha")?;

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
    mic_volume.set_value(30.0);
    until("the mic volume", || other.status().mic_volume == Some(Percent::saturating(30))).await?;

    // turning the game/chat dial on the headset updates the window
    other.set_chatmix(20);
    until("the ChatMix reading", || chatmix.subtitle().as_deref() == Some("20 (more game)")).await?;

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
