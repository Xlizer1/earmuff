use std::time::Duration;

use earmuff_domain::{Capability, Event, Query, Setting, UsbId};

/// A headset model a codec drives
#[derive(Debug)]
pub struct Model {
    pub name: &'static str,
    pub vendor: &'static str,
    /// USB IDs of its dongles
    pub ids: &'static [UsbId],
    pub capabilities: &'static [Capability],
}

/// One report to send, and how to recognize the headset's answer to it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    pub report: Vec<u8>,
    /// The answer starts with these bytes; `None` when the headset doesn't answer
    pub reply_prefix: Option<Vec<u8>>,
}

impl Exchange {
    pub fn answered_by(&self, report: &[u8]) -> bool {
        self.reply_prefix.as_deref().is_some_and(|prefix| report.starts_with(prefix))
    }
}

/// Translates between domain values and one headset's reports
pub trait Codec: Send + Sync {
    fn model(&self) -> &Model;

    /// Reports that ask the headset for current values
    fn query(&self, query: Query) -> Vec<Exchange>;

    /// Reports that change a setting, each followed by the report that reads it back.
    /// `None` when the headset doesn't support the setting.
    fn apply(&self, setting: Setting) -> Option<Vec<Exchange>>;

    /// What a report from the headset means, if anything
    fn decode(&self, report: &[u8]) -> Option<Event>;

    /// How long to wait for an answer before sending the next report
    fn reply_timeout(&self) -> Duration {
        Duration::from_millis(400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_matches_its_reply_prefix() {
        let exchange = Exchange { report: vec![1, 2, 3], reply_prefix: Some(vec![1, 2]) };
        assert!(exchange.answered_by(&[1, 2, 9]));
        assert!(!exchange.answered_by(&[1, 3, 9]));
        assert!(!exchange.answered_by(&[1]));
    }

    #[test]
    fn exchange_without_reply_is_never_answered() {
        let exchange = Exchange { report: vec![1], reply_prefix: None };
        assert!(!exchange.answered_by(&[1]));
    }
}
