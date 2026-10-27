//! Compiles the icons and the stylesheet in data/ into a GResource bundle that is
//! embedded in the binary, so Earmuff finds them whether it is installed or not.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let target = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo")).join("earmuff.gresource");
    println!("cargo::rerun-if-changed={}", data.display());
    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir")
        .arg(&data)
        .arg("--target")
        .arg(&target)
        .arg(data.join("resources.gresource.xml"))
        .status()
        .expect("glib-compile-resources (part of GLib) is needed to build Earmuff");
    assert!(status.success(), "glib-compile-resources failed");
}
