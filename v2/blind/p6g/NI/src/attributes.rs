//! Queue attributes and the partial attribute sets that requests carry.

use serde::Serialize;

use crate::error::Error;
use crate::types::{QueueName, Seconds};

pub const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds(30);
pub const DEFAULT_DELAY: Seconds = Seconds::ZERO;
pub const DEFAULT_RETENTION: Seconds = Seconds(345_600);

/// Where messages go once they have been received too often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: QueueName,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: u32,
}

/// The attributes of a queue, as `GetQueueAttributes` reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Attributes {
    pub visibility_timeout: Seconds,
    pub delay_seconds: Seconds,
    pub retention_seconds: Seconds,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetter>,
}

/// Attributes named in a request; every member that is absent is `None`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributePatch {
    pub visibility_timeout: Option<Seconds>,
    pub delay_seconds: Option<Seconds>,
    pub retention_seconds: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributePatch {
    /// The requested dead-letter configuration; both members or neither must be present.
    pub fn dead_letter(&self) -> Result<Option<(&str, u32)>, Error> {
        match (self.dead_letter_queue.as_deref(), self.max_receive_count) {
            (Some(queue), Some(max)) => Ok(Some((queue, max))),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

impl Attributes {
    /// Defaults for everything the patch leaves out.
    pub fn create(patch: &AttributePatch, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: patch
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: patch.delay_seconds.unwrap_or(DEFAULT_DELAY),
            retention_seconds: patch.retention_seconds.unwrap_or(DEFAULT_RETENTION),
            fifo: patch.fifo.unwrap_or(false),
            content_based_deduplication: patch.content_based_deduplication.unwrap_or(false),
            dead_letter,
        }
    }

    /// Whether creating a queue from `patch` (after defaults) would give these attributes.
    /// The patch must have a consistent dead-letter configuration.
    pub fn matches(&self, patch: &AttributePatch) -> bool {
        let requested_dead_letter = patch
            .dead_letter_queue
            .as_deref()
            .zip(patch.max_receive_count);
        self.visibility_timeout
            == patch
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.delay_seconds == patch.delay_seconds.unwrap_or(DEFAULT_DELAY)
            && self.retention_seconds == patch.retention_seconds.unwrap_or(DEFAULT_RETENTION)
            && self.fifo == patch.fifo.unwrap_or(false)
            && self.content_based_deduplication
                == patch.content_based_deduplication.unwrap_or(false)
            && self
                .dead_letter
                .as_ref()
                .map(|d| (d.queue.as_str(), d.max_receive_count))
                == requested_dead_letter
    }

    /// Overwrites the attributes the patch names. `Fifo` is never changed here.
    pub fn update(&mut self, patch: &AttributePatch, dead_letter: Option<DeadLetter>) {
        if let Some(value) = patch.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = patch.delay_seconds {
            self.delay_seconds = value;
        }
        if let Some(value) = patch.retention_seconds {
            self.retention_seconds = value;
        }
        if let Some(value) = patch.content_based_deduplication {
            self.content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dead_letter(queue: &str, max: u32) -> DeadLetter {
        DeadLetter {
            queue: QueueName::parse(queue).expect("test name is valid"),
            max_receive_count: max,
        }
    }

    #[test]
    fn empty_patch_matches_defaults() {
        let attributes = Attributes::create(&AttributePatch::default(), None);
        assert!(attributes.matches(&AttributePatch::default()));
        let patch = AttributePatch {
            visibility_timeout: Some(Seconds(31)),
            ..AttributePatch::default()
        };
        assert!(!attributes.matches(&patch));
    }

    #[test]
    fn dead_letter_must_agree_to_match() {
        let patch = AttributePatch {
            dead_letter_queue: Some("dlq".into()),
            max_receive_count: Some(3),
            ..AttributePatch::default()
        };
        let attributes = Attributes::create(&patch, Some(dead_letter("dlq", 3)));
        assert!(attributes.matches(&patch));
        assert!(!attributes.matches(&AttributePatch::default()));
        let other = AttributePatch {
            max_receive_count: Some(4),
            ..patch
        };
        assert!(!attributes.matches(&other));
    }

    #[test]
    fn dead_letter_members_come_in_pairs() {
        let only_queue = AttributePatch {
            dead_letter_queue: Some("dlq".into()),
            ..AttributePatch::default()
        };
        assert!(only_queue.dead_letter().is_err());
        assert_eq!(AttributePatch::default().dead_letter(), Ok(None));
    }

    #[test]
    fn update_keeps_what_the_patch_omits() {
        let mut attributes =
            Attributes::create(&AttributePatch::default(), Some(dead_letter("d", 2)));
        let patch = AttributePatch {
            delay_seconds: Some(Seconds(5)),
            ..AttributePatch::default()
        };
        attributes.update(&patch, None);
        assert_eq!(attributes.delay_seconds, Seconds(5));
        assert_eq!(attributes.visibility_timeout, DEFAULT_VISIBILITY_TIMEOUT);
        assert_eq!(attributes.dead_letter, Some(dead_letter("d", 2)));
    }
}
