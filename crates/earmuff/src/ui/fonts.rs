//! IBM Plex Sans, Earmuff's typeface, ships inside the program so the app looks the same
//! on every desktop. Pango loads fonts from files, so it gets a copy in the cache folder.

use std::path::Path;

use adw::prelude::*;
use earmuff_infrastructure::config::write_atomically;
use gtk::glib;
use pango::prelude::FontMapExt;

const PLEX: &[u8] = include_bytes!("../../../../data/fonts/IBMPlexSans.ttf");

/// Makes the typeface available to every widget; without it the system font is used
pub fn register() {
    let path = glib::user_cache_dir().join("earmuff").join("fonts").join("IBMPlexSans.ttf");
    if let Err(error) = load(&path) {
        eprintln!("earmuff: using the system font: {error}");
    }
}

fn load(path: &Path) -> Result<(), String> {
    // a copy of another size is from another version of Earmuff
    if std::fs::metadata(path).map(|file| file.len()).ok() != Some(PLEX.len() as u64) {
        write_atomically(path, PLEX).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    // a widget without a font map of its own uses the one every widget shares
    let fonts = gtk::Label::new(None).pango_context().font_map().ok_or("no font map")?;
    fonts.add_font_file(path).map_err(|error| error.to_string())
}
