//! End to end: the real window, driven like a user would, against a simulated headset.
//! Runs headless (see tests/support), so nothing appears on the desktop.

mod support;

use std::process::ExitCode;
use std::sync::Arc;

use adw::prelude::*;
use earmuff_application::Backend;
use earmuff_infrastructure::demo::{DemoBackend, DemoHeadset};
use support::{Outcome, ensure, find, page, run_app, until, window};

fn main() -> ExitCode {
    support::isolated("window", || {
        let demo = DemoBackend::new();
        let headset = demo.headsets()[0].clone();
        let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(demo)];
        run_app(backends, false, move |app| scenario(app, headset))
    })
}

async fn scenario(app: adw::Application, headset: DemoHeadset) -> Outcome {
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

    // the headset turns off; a setting changes meanwhile; it turns back on
    headset.with(|sim, now| sim.set_powered(false, now));
    until("the headset-is-off page", || page(&window) == "off").await?;
    headset.with(|sim, _| sim.auto_off = 45);
    headset.with(|sim, now| sim.set_powered(true, now));
    until("the settings page again", || page(&window) == "settings").await?;
    until("the auto-off set while it was off", || selected_text(&auto_off) == "45 minutes").await?;

    // the dongle is unplugged and plugged back in
    headset.unplug();
    until("the plug-in page", || page(&window) == "missing").await?;
    ensure(missing.description().as_deref() == Some(earmuff::ui::view_model::LOOKING), "says it is looking")?;
    headset.plug();
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
