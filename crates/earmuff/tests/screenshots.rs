//! Not a test: draws the window as the README shows it, into PNG files, using the
//! simulated headsets and the same headless display as the end-to-end tests.
//!
//! ```sh
//! EARMUFF_SCREENSHOTS=docs/screenshots cargo test -p earmuff --test screenshots
//! ```
//!
//! Without EARMUFF_SCREENSHOTS it does nothing.

mod support;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use adw::prelude::*;
use earmuff_application::Backend;
use earmuff_infrastructure::demo::{DemoBackend, DemoHeadset, FullHeadset};
use support::{Outcome, find, page, run_app, until, window};

const SCALE: f64 = 2.0;
const WIDTH: i32 = 1100;
const HEIGHT: i32 = 760;
/// The dashboard's widest (the clamp around it)
const MAX_WIDTH: i32 = 1100;
/// --window-bg-color in data/style.css
const WINDOW_BACKGROUND: &str = "#0d1012";

fn main() -> ExitCode {
    let Some(dir) = std::env::var_os("EARMUFF_SCREENSHOTS").map(PathBuf::from) else {
        println!("screenshots: skipped (set EARMUFF_SCREENSHOTS to a folder to take them)");
        return ExitCode::SUCCESS;
    };
    let dir = std::path::absolute(dir).unwrap_or_default();
    support::isolated("screenshots", move || {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        seed_battery_history()?;
        let demo = DemoBackend::new();
        let headset = demo.headsets()[0].clone();
        let other = demo.full_headset().clone();
        let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(demo)];
        run_app(backends, false, move |app| shoot(app, dir, headset, other))
    })
}

/// A day of use: drained over the evening, charged overnight, draining again
fn seed_battery_history() -> Outcome {
    let state = std::env::var("XDG_STATE_HOME").map_err(|e| e.to_string())?;
    let dir = Path::new(&state).join("earmuff");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs();
    let mut readings = String::new();
    let mut add = |hours_ago: f64, level: u8, charging: u8| {
        readings += &format!("{},{level},{charging}\n", now - (hours_ago * 3600.0) as u64);
    };
    for step in 0..=40 {
        // 23 h ago to 13 h ago: 96% down to 31%
        add(23.0 - f64::from(step) / 4.0, 96 - (f64::from(step) * 1.6) as u8, 0);
    }
    for step in 0..=12 {
        add(12.5 - f64::from(step) / 4.0, 31 + step * 5, 1);
    }
    for step in 0..=36 {
        // 9 h ago until now: 92% down to 80%
        add(9.0 - f64::from(step) / 4.0, 92 - step / 3, 0);
    }
    std::fs::write(dir.join("battery-03f0-098d.csv"), readings).map_err(|e| e.to_string())
}

/// How tall the window must be to show the whole dashboard at `width`, without scrolling
fn page_height(window: &gtk::Window, width: i32) -> Result<i32, String> {
    let header: adw::HeaderBar = find(window, "header")?;
    let dashboard: gtk::Grid = find(window, "dashboard")?;
    let margins = dashboard.margin_start() + dashboard.margin_end();
    let (_, header_height, _, _) = header.measure(gtk::Orientation::Vertical, width);
    let (_, page, _, _) = dashboard.measure(gtk::Orientation::Vertical, width.min(MAX_WIDTH) - margins);
    Ok(header_height + page + dashboard.margin_top() + dashboard.margin_bottom())
}

/// Draws the whole dashboard at `width` into a PNG, at twice the size for sharp text. The
/// headless display only draws frames for a browser, so this lays the window out and asks
/// the widgets to draw themselves.
async fn save(window: &gtk::Window, path: &Path, width: i32) -> Outcome {
    let content = window.first_child().ok_or("an empty window")?;
    // the narrow layout isn't drawn here: libadwaita switches to it on a frame, and the
    // headless display draws none
    window.allocate(width, HEIGHT, -1, None);
    let height = page_height(window, width)?.max(HEIGHT);
    window.allocate(width, height, -1, None);
    let snapshot = gtk::Snapshot::new();
    snapshot.scale(SCALE as f32, SCALE as f32);
    // the window's own background, which its content draws on
    let background = gtk::gdk::RGBA::parse(WINDOW_BACKGROUND).map_err(|e| e.to_string())?;
    snapshot.append_color(&background, &gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
    window.snapshot_child(&content, &snapshot);
    let (width, height) = (f64::from(width), f64::from(height));
    let node = snapshot.to_node().ok_or("nothing to draw")?;
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None::<&gtk::gdk::Surface>).map_err(|e| e.to_string())?;
    let bounds = gtk::graphene::Rect::new(0.0, 0.0, (width * SCALE) as f32, (height * SCALE) as f32);
    let texture = renderer.render_texture(&node, Some(&bounds));
    renderer.unrealize();
    texture.save_to_png(path).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("screenshots: {}", path.display());
    Ok(())
}

/// Lets the window settle (animations, the battery ring) before drawing it
async fn settle() {
    gtk::glib::timeout_future(std::time::Duration::from_millis(900)).await;
}

async fn shoot(app: adw::Application, dir: PathBuf, headset: DemoHeadset, other: FullHeadset) -> Outcome {
    let window = window(&app).await?;
    // without frames, animations never get anywhere: a page halfway through its crossfade
    // stays on the old page. With them off, every change shows at once.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }
    let pages: gtk::Stack = find(&window, "pages")?;
    let switcher: gtk::DropDown = find(&window, "device-switcher")?;
    until("the settings page", || page(&window) == "settings").await?;
    until("both headsets", || switcher.model().is_some_and(|list| list.n_items() == 2)).await?;
    // finish the crossfade that started before animations were off
    pages.set_visible_child_name("missing");
    pages.set_visible_child_name("settings");
    headset.with(|sim, now| sim.set_battery(81, now));

    // the headset with everything, its equalizer on a preset
    other.set_chatmix(40);
    switcher.set_selected(1);
    until("the demo headset", || find::<gtk::ToggleButton>(&window, "eq-preset-2").is_ok()).await?;
    find::<gtk::ToggleButton>(&window, "eq-preset-2")?.set_active(true);
    until("the footsteps curve", || {
        find::<gtk::Scale>(&window, "eq-band-6").is_ok_and(|band| (band.value() - 6.0).abs() < 0.01)
    })
    .await?;
    settle().await;
    save(&window, &dir.join("overview.png"), WIDTH).await?;

    // the Cloud Alpha Wireless: muted, with the software equalizer on
    switcher.set_selected(0);
    until("the Cloud Alpha", || find::<gtk::Switch>(&window, "software-eq").is_ok()).await?;
    find::<gtk::ToggleButton>(&window, "mute")?.set_active(true);
    until("the mic to mute", || headset.with(|sim, _| sim.muted)).await?;
    find::<gtk::Switch>(&window, "software-eq")?.set_active(true);
    until("the software equalizer", || {
        find::<gtk::FlowBox>(&window, "software-eq-preset").is_ok_and(|chips| chips.is_sensitive())
    })
    .await?;
    find::<gtk::ToggleButton>(&window, "software-eq-preset-1")?.set_active(true);
    until("the bass boost curve", || {
        find::<gtk::Scale>(&window, "eq-band-0").is_ok_and(|band| (band.value() - 6.0).abs() < 0.01)
    })
    .await?;
    settle().await;
    save(&window, &dir.join("cloud-alpha.png"), WIDTH).await?;

    // put things back the way they were found, then quit
    find::<gtk::Switch>(&window, "software-eq")?.set_active(false);
    find::<gtk::ToggleButton>(&window, "mute")?.set_active(false);
    settle().await;
    window.close();
    Ok(())
}
