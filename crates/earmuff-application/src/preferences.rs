//! How the user set up Earmuff itself, as opposed to a headset's settings.

/// Earmuff's own preferences. Missing values fall back to these defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    /// The accent color's name, e.g. "red"
    pub accent: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences { accent: "red".into() }
    }
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
}
