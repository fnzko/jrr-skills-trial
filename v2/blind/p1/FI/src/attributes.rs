//! Queue attributes: the full set a queue holds and the partial set a request carries.

use std::fmt;

use serde::Serialize;

/// Where a queue sends messages that were received too often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetter {
    #[serde(rename = "DeadLetterQueue")]
    pub queue: String,
    pub max_receive_count: u64,
}

/// The attributes of a queue, with the defaults of the spec applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Attributes {
    pub visibility_timeout: u64,
    pub delay_seconds: u64,
    pub retention_seconds: u64,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    #[serde(flatten)]
    pub dead_letter: Option<DeadLetter>,
}

impl Default for Attributes {
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

impl Attributes {
    /// These attributes with every member present in `update` replaced.
    ///
    /// `dead_letter` is the already paired configuration of the update; when
    /// it is `None` the current configuration is kept.
    pub fn updated(&self, update: &AttributeUpdate, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: update.visibility_timeout.unwrap_or(self.visibility_timeout),
            delay_seconds: update.delay_seconds.unwrap_or(self.delay_seconds),
            retention_seconds: update.retention_seconds.unwrap_or(self.retention_seconds),
            fifo: update.fifo.unwrap_or(self.fifo),
            content_based_deduplication: update
                .content_based_deduplication
                .unwrap_or(self.content_based_deduplication),
            dead_letter: dead_letter.or_else(|| self.dead_letter.clone()),
        }
    }
}

/// The attributes a request names; the others are absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<u64>,
    pub delay_seconds: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

/// Exactly one of `DeadLetterQueue` and `MaxReceiveCount` was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpairedDeadLetter;

impl fmt::Display for UnpairedDeadLetter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeadLetterQueue and MaxReceiveCount must be given together")
    }
}

impl AttributeUpdate {
    /// The dead-letter configuration of the update, if it names one.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, UnpairedDeadLetter> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(UnpairedDeadLetter),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_replaces_only_present_members() {
        let base = Attributes {
            visibility_timeout: 5,
            delay_seconds: 6,
            ..Attributes::default()
        };
        let update = AttributeUpdate {
            delay_seconds: Some(7),
            retention_seconds: Some(100),
            ..AttributeUpdate::default()
        };
        let updated = base.updated(&update, None);
        assert_eq!(
            updated,
            Attributes {
                visibility_timeout: 5,
                delay_seconds: 7,
                retention_seconds: 100,
                ..Attributes::default()
            }
        );
    }

    #[test]
    fn update_keeps_the_dead_letter_configuration_unless_replaced() {
        let dead_letter = |queue: &str, max_receive_count| DeadLetter {
            queue: queue.to_owned(),
            max_receive_count,
        };
        let base = Attributes {
            dead_letter: Some(dead_letter("old", 3)),
            ..Attributes::default()
        };
        let kept = base.updated(&AttributeUpdate::default(), None);
        assert_eq!(kept.dead_letter, Some(dead_letter("old", 3)));
        let replaced = base.updated(&AttributeUpdate::default(), Some(dead_letter("new", 9)));
        assert_eq!(replaced.dead_letter, Some(dead_letter("new", 9)));
    }

    #[test]
    fn dead_letter_members_pair_up() {
        let update = |queue: Option<&str>, count| AttributeUpdate {
            dead_letter_queue: queue.map(str::to_owned),
            max_receive_count: count,
            ..AttributeUpdate::default()
        };
        assert_eq!(update(None, None).dead_letter(), Ok(None));
        assert_eq!(
            update(Some("q"), None).dead_letter(),
            Err(UnpairedDeadLetter)
        );
        assert_eq!(update(None, Some(2)).dead_letter(), Err(UnpairedDeadLetter));
        assert_eq!(
            update(Some("q"), Some(2)).dead_letter(),
            Ok(Some(DeadLetter {
                queue: "q".to_owned(),
                max_receive_count: 2
            }))
        );
    }
}
