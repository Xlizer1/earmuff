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
/// --window-bg-color in data/style.css
const WINDOW_BACKGROUND: &str = "#0f1015";

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

/// Draws the window's content (without the shadow around it) into a PNG, at twice its
/// size for sharp text. The headless display only draws frames for a browser, so this
/// asks the widgets to draw themselves.
async fn save(window: &gtk::Window, path: &Path) -> Outcome {
    let content = window.first_child().ok_or("an empty window")?;
    let (width, height) = (WIDTH, HEIGHT);
    // without frames there's no layout pass either, so lay the window out here
    content.allocate(width, height, -1, None);
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

/// Scrolls the page with the widget named `inside` all the way down
fn scroll_down(window: &gtk::Window, inside: &str) -> Outcome {
    let content = window.first_child().ok_or("an empty window")?;
    // lay out first, so the scrolled page knows how tall it is
    content.allocate(WIDTH, HEIGHT, -1, None);
    let widget: gtk::Widget = find(window, inside)?;
    let scrolled = widget.ancestor(gtk::ScrolledWindow::static_type()).ok_or("not on a scrolled page")?;
    let adjustment = scrolled.downcast::<gtk::ScrolledWindow>().map_err(|_| "not scrolled")?.vadjustment();
    adjustment.set_value(adjustment.upper() - adjustment.page_size());
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
    let tabs: adw::ViewStack = find(&window, "tabs")?;
    let list: gtk::ListBox = find(&window, "device-list")?;
    until("the settings page", || page(&window) == "settings").await?;
    until("both headsets", || list.row_at_index(1).is_some()).await?;
    // finish the crossfade that started before animations were off
    pages.set_visible_child_name("missing");
    pages.set_visible_child_name("settings");
    headset.with(|sim, now| sim.set_battery(81, now));

    // the headset with everything, its equalizer on a preset
    other.set_chatmix(40);
    list.select_row(list.row_at_index(1).as_ref());
    until("the demo headset", || find::<adw::ComboRow>(&window, "eq-preset").is_ok()).await?;
    find::<adw::ComboRow>(&window, "eq-preset")?.set_selected(2);
    until("the footsteps curve", || {
        find::<gtk::Scale>(&window, "eq-band-6").is_ok_and(|band| (band.value() - 6.0).abs() < 0.01)
    })
    .await?;
    tabs.set_visible_child_name("audio");
    settle().await;
    save(&window, &dir.join("overview.png")).await?;
    scroll_down(&window, "equalizer")?;
    save(&window, &dir.join("equalizer.png")).await?;

    tabs.set_visible_child_name("microphone");
    settle().await;
    save(&window, &dir.join("microphone.png")).await?;

    // the Cloud Alpha Wireless: its battery history, and the software equalizer
    list.select_row(list.row_at_index(0).as_ref());
    until("the Cloud Alpha", || find::<adw::SwitchRow>(&window, "software-eq").is_ok()).await?;
    tabs.set_visible_child_name("power");
    settle().await;
    save(&window, &dir.join("power.png")).await?;

    find::<adw::SwitchRow>(&window, "software-eq")?.set_active(true);
    until("the software equalizer", || find::<adw::ComboRow>(&window, "software-eq-preset").is_ok()).await?;
    find::<adw::ComboRow>(&window, "software-eq-preset")?.set_selected(1);
    until("the bass boost curve", || {
        find::<gtk::Scale>(&window, "eq-band-0").is_ok_and(|band| (band.value() - 6.0).abs() < 0.01)
    })
    .await?;
    tabs.set_visible_child_name("audio");
    settle().await;
    scroll_down(&window, "equalizer")?;
    save(&window, &dir.join("software-eq.png")).await?;

    // put things back the way they were found, then quit
    find::<adw::SwitchRow>(&window, "software-eq")?.set_active(false);
    settle().await;
    window.close();
    Ok(())
}
