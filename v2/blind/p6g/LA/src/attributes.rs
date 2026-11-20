//! Queue attributes, as stored and as requested.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: String,
    #[serde(rename = "MaxReceiveCount")]
    pub max_receive_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Attributes {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetter>,
}

/// The attributes named in a request. Absent ones are `None`.
///
/// `dead_letter_queue` and `max_receive_count` may arrive alone; commands
/// reject that before using the patch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributePatch {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributePatch {
    /// True when only one of the two dead-letter attributes is present.
    pub fn dead_letter_is_partial(&self) -> bool {
        self.dead_letter_queue.is_some() != self.max_receive_count.is_some()
    }

    fn dead_letter(&self) -> Option<DeadLetter> {
        Some(DeadLetter {
            queue: self.dead_letter_queue.clone()?,
            max_receive_count: self.max_receive_count?,
        })
    }
}

impl Attributes {
    /// The attributes a new queue gets: the patch over the defaults.
    pub fn from_patch(patch: &AttributePatch) -> Self {
        Self {
            visibility_timeout: patch.visibility_timeout.unwrap_or(30),
            delay_seconds: patch.delay_seconds.unwrap_or(0),
            retention_seconds: patch.retention_seconds.unwrap_or(345_600),
            fifo: patch.fifo.unwrap_or(false),
            content_based_deduplication: patch.content_based_deduplication.unwrap_or(false),
            dead_letter: patch.dead_letter(),
        }
    }

    /// Overwrites the attributes the patch names. `fifo` never changes.
    pub fn apply(&mut self, patch: &AttributePatch) {
        self.visibility_timeout = patch.visibility_timeout.unwrap_or(self.visibility_timeout);
        self.delay_seconds = patch.delay_seconds.unwrap_or(self.delay_seconds);
        self.retention_seconds = patch.retention_seconds.unwrap_or(self.retention_seconds);
        self.content_based_deduplication = patch
            .content_based_deduplication
            .unwrap_or(self.content_based_deduplication);
        if let Some(dead_letter) = patch.dead_letter() {
            self.dead_letter = Some(dead_letter);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let attributes = Attributes::from_patch(&AttributePatch::default());
        assert_eq!(
            attributes,
            Attributes {
                visibility_timeout: 30,
                delay_seconds: 0,
                retention_seconds: 345_600,
                fifo: false,
                content_based_deduplication: false,
                dead_letter: None,
            }
        );
    }

    #[test]
    fn apply_keeps_unnamed_attributes_and_dead_letter() {
        let mut attributes = Attributes::from_patch(&AttributePatch {
            dead_letter_queue: Some("dlq".into()),
            max_receive_count: Some(3),
            ..AttributePatch::default()
        });
        attributes.apply(&AttributePatch {
            delay_seconds: Some(7),
            ..AttributePatch::default()
        });
        assert_eq!(attributes.delay_seconds, 7);
        assert_eq!(attributes.visibility_timeout, 30);
        assert_eq!(
            attributes.dead_letter,
            Some(DeadLetter {
                queue: "dlq".into(),
                max_receive_count: 3
            })
        );
    }

    #[test]
    fn serialization_omits_unset_dead_letter() {
        let json =
            serde_json::to_value(Attributes::from_patch(&AttributePatch::default())).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            })
        );
    }
}
