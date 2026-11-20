//! Queue attributes: the settled [`Config`] of a queue and the
//! [`AttributePatch`] a request may apply to it.

/// Dead-letter configuration. Holding both fields in one value makes the
/// rule "both are set or both are unset" unrepresentable to break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

/// The attributes a queue has, with defaults filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            visibility_timeout: 30,
            delay_seconds: 0,
            retention_seconds: 345_600,
            fifo: false,
            content_based_deduplication: false,
            dead_letter: None,
        }
    }
}

/// The attributes named in a request. Each field is already checked against
/// its own type and range, but not against the other attributes or queues.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributePatch {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

impl Config {
    /// Overwrites the attributes the patch names. `Fifo` is not applied: the
    /// caller decides whether it may change at all, and creation sets it
    /// through [`Config::new_from`].
    pub fn apply(&mut self, patch: &AttributePatch, dead_letter: Option<DeadLetter>) {
        if let Some(v) = patch.visibility_timeout {
            self.visibility_timeout = v;
        }
        if let Some(v) = patch.delay_seconds {
            self.delay_seconds = v;
        }
        if let Some(v) = patch.retention_seconds {
            self.retention_seconds = v;
        }
        if let Some(v) = patch.content_based_deduplication {
            self.content_based_deduplication = v;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    /// The configuration of a new queue: defaults overlaid with the patch.
    pub fn new_from(patch: &AttributePatch, dead_letter: Option<DeadLetter>) -> Self {
        let mut config = Self {
            fifo: patch.fifo.unwrap_or(false),
            ..Self::default()
        };
        config.apply(patch, dead_letter);
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_queue_takes_defaults_for_missing_attributes() {
        let patch = AttributePatch {
            delay_seconds: Some(5),
            fifo: Some(true),
            ..AttributePatch::default()
        };
        let config = Config::new_from(&patch, None);
        assert_eq!(
            config,
            Config {
                visibility_timeout: 30,
                delay_seconds: 5,
                retention_seconds: 345_600,
                fifo: true,
                content_based_deduplication: false,
                dead_letter: None,
            }
        );
    }

    #[test]
    fn apply_keeps_what_the_patch_leaves_out() {
        let mut config = Config {
            dead_letter: Some(DeadLetter {
                queue: "dlq".into(),
                max_receive_count: 3,
            }),
            ..Config::default()
        };
        let patch = AttributePatch {
            visibility_timeout: Some(7),
            ..AttributePatch::default()
        };
        config.apply(&patch, None);
        assert_eq!(config.visibility_timeout, 7);
        assert_eq!(
            config.dead_letter.as_ref().map(|d| d.max_receive_count),
            Some(3)
        );
    }
}
