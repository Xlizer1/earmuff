//! How the user set up Earmuff itself, as opposed to a headset's settings.

/// Earmuff's own preferences. Missing values fall back to these defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    /// The accent color's name, e.g. "red"
    pub accent: String,
    /// Battery percentage that triggers a warning (and half of it a second one); 0 is off
    pub battery_alert: u8,
    /// Say when a charging headset is full
    pub notify_charged: bool,
    /// Put each headset's settings back when it reconnects
    pub remember_settings: bool,
}

impl Preferences {
    pub fn alerts(&self) -> crate::AlertSettings {
        crate::AlertSettings {
            low_at: (self.battery_alert > 0).then_some(self.battery_alert),
            when_charged: self.notify_charged,
        }
    }
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences { accent: "red".into(), battery_alert: 20, notify_charged: true, remember_settings: true }
    }
}

/// Starts Earmuff when the user logs in
pub trait Autostart: Send + Sync {
    fn is_enabled(&self) -> bool;

    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
}

/// Keeps preferences between runs
pub trait PreferencesStore: Send + Sync {
    /// The saved preferences, or the defaults when there are none (or they can't be read)
    fn load(&self) -> Preferences;

    fn save(&self, preferences: &Preferences) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_is_the_default_accent() {
        assert_eq!(Preferences::default().accent, "red");
    }

    #[test]
    fn alert_preferences_become_alert_settings() {
        assert_eq!(Preferences::default().alerts(), crate::AlertSettings { low_at: Some(20), when_charged: true });
        let off = Preferences { battery_alert: 0, notify_charged: false, ..Preferences::default() };
        assert_eq!(off.alerts(), crate::AlertSettings { low_at: None, when_charged: false });
    }
}
