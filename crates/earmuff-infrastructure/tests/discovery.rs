//! Finding headsets among hidraw devices, using a fake sysfs tree.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use earmuff_application::{Backend, BackendError};
use earmuff_domain::{BackendKind, Capability, UsbId};
use earmuff_infrastructure::hidraw::{HidrawNode, scan};
use earmuff_infrastructure::native::NativeBackend;

fn add_node(sysfs: &Path, name: &str, uevent: &str) {
    let device = sysfs.join(name).join("device");
    fs::create_dir_all(&device).unwrap();
    fs::write(device.join("uevent"), uevent).unwrap();
}

fn sysfs_with_devices() -> tempfile::TempDir {
    let sysfs = tempfile::tempdir().unwrap();
    // the same devices as on the development machine: mouse, motherboard RGB, headset, keyboard
    add_node(sysfs.path(), "hidraw0", "HID_ID=0003:0000093A:0000822A\nHID_NAME=Mouse\n");
    add_node(sysfs.path(), "hidraw2", "HID_ID=0003:00001462:00007C71\nHID_NAME=MSI MYSTIC LIGHT\n");
    add_node(sysfs.path(), "hidraw3", "HID_ID=0003:000003F0:0000098D\nHID_NAME=HP, Inc HyperX Cloud Alpha Wireless\n");
    add_node(sysfs.path(), "hidraw5", "HID_ID=0005:000003F0:0000098D\nHID_NAME=Bluetooth twin\n");
    add_node(sysfs.path(), "hidraw9", "garbage\n");
    sysfs
}

#[test]
fn scan_lists_usb_hidraw_nodes() {
    let sysfs = sysfs_with_devices();
    let nodes = scan(sysfs.path());
    assert_eq!(
        nodes,
        [
            HidrawNode { path: "/dev/hidraw0".into(), usb: UsbId::new(0x093A, 0x822A) },
            HidrawNode { path: "/dev/hidraw2".into(), usb: UsbId::new(0x1462, 0x7C71) },
            HidrawNode { path: "/dev/hidraw3".into(), usb: UsbId::new(0x03F0, 0x098D) },
        ]
    );
}

#[test]
fn native_backend_finds_only_supported_headsets() {
    let sysfs = sysfs_with_devices();
    let devices = NativeBackend::with_sysfs(sysfs.path()).discover().unwrap();
    assert_eq!(devices.len(), 1);
    let device = &devices[0];
    assert_eq!(device.id.backend, BackendKind::Native);
    assert_eq!(device.id.instance, "/dev/hidraw3");
    assert_eq!(device.id.usb, UsbId::new(0x03F0, 0x098D));
    assert_eq!((device.vendor.as_str(), device.name.as_str()), ("HyperX", "Cloud Alpha Wireless"));
    assert!(device.supports(Capability::SidetoneLevel));
}

#[test]
fn opening_a_vanished_device_says_not_found() {
    let sysfs = sysfs_with_devices();
    let backend = NativeBackend::with_sysfs(sysfs.path());
    let mut device = backend.discover().unwrap().remove(0);
    device.id.instance = sysfs.path().join("no-such-node").display().to_string();
    assert_eq!(backend.open(&device).err(), Some(BackendError::NotFound));
}

#[test]
fn opening_without_permission_explains_how_to_fix_it() {
    let sysfs = sysfs_with_devices();
    let locked = sysfs.path().join("locked");
    fs::write(&locked, b"").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&locked).is_ok() {
        eprintln!("skipped: running with privileges that ignore file permissions");
        return;
    }
    let backend = NativeBackend::with_sysfs(sysfs.path());
    let mut device = backend.discover().unwrap().remove(0);
    device.id.instance = locked.display().to_string();
    let Some(BackendError::PermissionDenied { path, hint }) = backend.open(&device).err() else {
        panic!("expected a permission error");
    };
    assert_eq!(path, locked.display().to_string());
    assert!(hint.contains("udev rule"));
}
