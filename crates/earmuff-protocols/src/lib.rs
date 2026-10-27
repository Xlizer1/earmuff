//! Byte-level protocols of the headsets Earmuff drives itself ("native" headsets).
//!
//! Pure functions only: a codec turns settings and queries into reports and reports
//! into domain events. Moving the bytes is earmuff-infrastructure's job.

mod codec;
pub mod hyperx;

pub use codec::{Codec, Exchange, Model};

use earmuff_domain::UsbId;

/// Every headset with a native driver
pub fn catalog() -> &'static [&'static dyn Codec] {
    &[&hyperx::CloudAlphaWireless]
}

/// The codec for a USB dongle, if Earmuff drives it natively
pub fn find(usb: UsbId) -> Option<&'static dyn Codec> {
    catalog().iter().copied().find(|codec| codec.model().ids.contains(&usb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_cloud_alpha_wireless_by_usb_id() {
        let codec = find(UsbId::new(0x03F0, 0x098D)).expect("in the catalog");
        assert_eq!(codec.model().name, "Cloud Alpha Wireless");
        assert!(find(UsbId::new(0x1234, 0x5678)).is_none());
    }

    #[test]
    fn catalog_ids_are_unique() {
        let mut ids: Vec<UsbId> = catalog().iter().flat_map(|c| c.model().ids.iter().copied()).collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
