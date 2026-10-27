//! End to end: the tray icon, driven over D-Bus the way Waybar's tray does it.
//! A fake StatusNotifierWatcher stands in for the tray host on a private bus.

mod support;

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use earmuff_application::Backend;
use earmuff_infrastructure::demo::{DemoBackend, DemoHeadset};
use support::{Outcome, find, page, run_app, until, window};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// Records the tray icons that register, like Waybar's tray module
#[derive(Clone, Default)]
struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String) {
        self.items.lock().unwrap().push(service);
    }

    fn register_status_notifier_host(&self, _service: String) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

fn main() -> ExitCode {
    support::isolated("tray", || {
        let watcher = Watcher::default();
        let connection = zbus::block_on(async {
            zbus::connection::Builder::session()?
                .name("org.kde.StatusNotifierWatcher")?
                .serve_at("/StatusNotifierWatcher", watcher.clone())?
                .build()
                .await
        })
        .map_err(|e| format!("fake tray host: {e}"))?;
        let demo = DemoBackend::new();
        let headset = demo.headsets()[0].clone();
        let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(demo)];
        run_app(backends, true, move |app| scenario(app, headset, watcher, connection))
    })
}

fn failed(what: &str) -> impl Fn(zbus::Error) -> String + '_ {
    move |e| format!("{what}: {e}")
}

async fn property(bus: &zbus::Connection, service: &str, path: &str, name: &str) -> Result<OwnedValue, String> {
    bus.call_method(
        Some(service),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &("org.kde.StatusNotifierItem", name),
    )
    .await
    .map_err(failed(name))?
    .body()
    .deserialize::<OwnedValue>()
    .map_err(|e| format!("{name}: {e}"))
}

/// StatusNotifierItem tooltip: icon name, icon pixmaps (width, height, ARGB data), title, text
type ToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

async fn tooltip(bus: &zbus::Connection, service: &str) -> Result<String, String> {
    let value = property(bus, service, "/StatusNotifierItem", "ToolTip").await?;
    let (_icon, _pixmaps, _title, description): ToolTip = value.try_into().map_err(|e| format!("tooltip: {e}"))?;
    Ok(description)
}

/// Menu entries by label: (id, checked)
async fn menu(bus: &zbus::Connection, service: &str, path: &str) -> Result<HashMap<String, (i32, bool)>, String> {
    type Node = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);
    let reply = bus
        .call_method(
            Some(service),
            path,
            Some("com.canonical.dbusmenu"),
            "GetLayout",
            &(0i32, -1i32, Vec::<String>::new()),
        )
        .await
        .map_err(failed("GetLayout"))?;
    let (_revision, (_id, _props, children)): (u32, Node) =
        reply.body().deserialize().map_err(|e| format!("GetLayout: {e}"))?;
    let mut entries = HashMap::new();
    for child in children {
        let (id, props, _): Node = child.try_into().map_err(|e| format!("menu item: {e}"))?;
        let text = |key: &str| props.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
        let checked = props.get("toggle-state").and_then(|v| i32::try_from(v.try_clone().ok()?).ok()) == Some(1);
        if let Some(label) = text("label") {
            entries.insert(label, (id, checked));
        }
    }
    Ok(entries)
}

async fn click(bus: &zbus::Connection, service: &str, path: &str, id: i32) -> Outcome {
    bus.call_method(
        Some(service),
        path,
        Some("com.canonical.dbusmenu"),
        "Event",
        &(id, "clicked", Value::from(""), 0u32),
    )
    .await
    .map_err(failed("Event"))?;
    Ok(())
}

async fn scenario(app: adw::Application, headset: DemoHeadset, watcher: Watcher, bus: zbus::Connection) -> Outcome {
    let window = window(&app).await?;
    until("the settings page", || page(&window) == "settings").await?;

    // the icon registers with the tray host
    until("the tray icon to register", || !watcher.items.lock().unwrap().is_empty()).await?;
    let service = watcher.items.lock().unwrap()[0].clone();
    let menu_path: OwnedObjectPath = property(&bus, &service, "/StatusNotifierItem", "Menu")
        .await?
        .try_into()
        .map_err(|e| format!("menu path: {e}"))?;
    let menu_path = menu_path.as_str().to_string();

    // the icon has a name for the icon theme and pixels for trays that can't find it
    let name: String =
        property(&bus, &service, "/StatusNotifierItem", "IconName").await?.try_into().map_err(|e| format!("{e}"))?;
    support::ensure(name == "io.github.xlizer1.Earmuff", format!("icon name {name}"))?;
    let pixmaps: Vec<(i32, i32, Vec<u8>)> =
        property(&bus, &service, "/StatusNotifierItem", "IconPixmap").await?.try_into().map_err(|e| format!("{e}"))?;
    let (width, height, pixels) = pixmaps.first().ok_or("no icon pixmap")?;
    support::ensure((*width, *height, pixels.len()) == (64, 64, 64 * 64 * 4), "a 64x64 ARGB icon")?;
    // the icon's corners are transparent, its middle is not
    support::ensure(pixels[0] == 0 && pixels[(32 * 64 + 32) * 4] == 255, "icon alpha looks right")?;

    // the tooltip shows the battery
    for _ in 0..500 {
        if tooltip(&bus, &service).await? == "Battery 80%" {
            break;
        }
        gtk::glib::timeout_future(std::time::Duration::from_millis(10)).await;
    }
    let tip = tooltip(&bus, &service).await?;
    support::ensure(tip == "Battery 80%", format!("tooltip says {tip:?}"))?;

    // closing the window only hides it
    window.close();
    until("the window to hide", || !window.is_visible()).await?;

    // clicking the icon brings it back
    bus.call_method(
        Some(service.as_str()),
        "/StatusNotifierItem",
        Some("org.kde.StatusNotifierItem"),
        "Activate",
        &(0i32, 0i32),
    )
    .await
    .map_err(failed("Activate"))?;
    until("the window to come back", || window.is_visible()).await?;

    // muting from the menu mutes the headset, and the menu shows it
    let entries = menu(&bus, &service, &menu_path).await?;
    let (mute, checked) = *entries.get("Mute microphone").ok_or("no mute entry in the menu")?;
    support::ensure(!checked, "mute starts unchecked")?;
    click(&bus, &service, &menu_path, mute).await?;
    until("the headset to mute", || headset.with(|sim, _| sim.muted)).await?;
    let mute_row: adw::SwitchRow = find(&window, "mute")?;
    until("the window's switch to follow", || mute_row.is_active()).await?;
    let mut checked = false;
    for _ in 0..500 {
        checked = menu(&bus, &service, &menu_path).await?["Mute microphone"].1;
        if checked {
            break;
        }
        gtk::glib::timeout_future(std::time::Duration::from_millis(10)).await;
    }
    support::ensure(checked, "the menu shows the mic as muted")?;

    // Quit in the menu ends the app. The app may be gone before D-Bus answers the click,
    // so the click is not awaited here; the harness checks that the app quits.
    let (quit, _) = *entries.get("Quit").ok_or("no Quit entry in the menu")?;
    gtk::glib::spawn_future_local(async move {
        let _ = click(&bus, &service, &menu_path, quit).await;
    });
    Ok(())
}
