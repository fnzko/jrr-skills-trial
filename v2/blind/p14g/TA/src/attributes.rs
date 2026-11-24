//! Queue attributes: the stored settings of a queue and the partial update a command carries.

use crate::error::Error;
use crate::types::Seconds;

pub const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds(30);
pub const DEFAULT_DELAY: Seconds = Seconds(0);
pub const DEFAULT_RETENTION: Seconds = Seconds(345_600);

/// A standard queue has no deduplication setting; a FIFO queue always has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

/// Where messages that were received too often are moved, and how often is too often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    /// Name of the dead-letter queue. The service keeps it pointing at an existing queue.
    pub(crate) queue: String,
    pub(crate) max_receive_count: u32,
}

/// The attributes a command mentions. Every value is already inside its allowed range: the
/// request parser is the only place that builds one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributePatch {
    pub(crate) visibility_timeout: Option<Seconds>,
    pub(crate) delay: Option<Seconds>,
    pub(crate) retention: Option<Seconds>,
    pub(crate) fifo: Option<bool>,
    pub(crate) content_based_deduplication: Option<bool>,
    pub(crate) dead_letter_queue: Option<String>,
    pub(crate) max_receive_count: Option<u32>,
}

impl AttributePatch {
    /// The dead-letter configuration the patch asks for. The two attributes that make it up must
    /// come together.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            _ => Err(Error::invalid_value(
                "Attributes",
                "DeadLetterQueue and MaxReceiveCount together",
            )),
        }
    }
}

/// The settings of one queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAttributes {
    pub(crate) visibility_timeout: Seconds,
    pub(crate) delay: Seconds,
    pub(crate) retention: Seconds,
    pub(crate) kind: QueueKind,
    pub(crate) dead_letter: Option<DeadLetter>,
}

impl QueueAttributes {
    /// The settings of a new queue: what `patch` states, the defaults for the rest.
    pub fn from_patch(patch: &AttributePatch, dead_letter: Option<DeadLetter>) -> Self {
        let kind = if patch.fifo.unwrap_or(false) {
            QueueKind::Fifo {
                content_based_deduplication: patch.content_based_deduplication.unwrap_or(false),
            }
        } else {
            QueueKind::Standard
        };
        Self {
            visibility_timeout: patch
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: patch.delay.unwrap_or(DEFAULT_DELAY),
            retention: patch.retention.unwrap_or(DEFAULT_RETENTION),
            kind,
            dead_letter,
        }
    }

    /// Overwrites the attributes `patch` states. The queue type never changes, and a dead-letter
    /// configuration can be replaced but not removed.
    pub fn apply(&mut self, patch: &AttributePatch, dead_letter: Option<DeadLetter>) {
        if let Some(value) = patch.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = patch.delay {
            self.delay = value;
        }
        if let Some(value) = patch.retention {
            self.retention = value;
        }
        if let (
            QueueKind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut self.kind, patch.content_based_deduplication)
        {
            *content_based_deduplication = value;
        }
        if dead_letter.is_some() {
            self.dead_letter = dead_letter;
        }
    }

    pub fn is_fifo(&self) -> bool {
        matches!(self.kind, QueueKind::Fifo { .. })
    }

    /// Drops the dead-letter configuration if it points at `queue`.
    pub fn forget_dead_letter(&mut self, queue: &str) {
        if self.dead_letter.as_ref().is_some_and(|d| d.queue == queue) {
            self.dead_letter = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn from_patch_fills_defaults() {
        let attributes = QueueAttributes::from_patch(&AttributePatch::default(), None);
        assert_eq!(
            attributes,
            QueueAttributes {
                visibility_timeout: Seconds(30),
                delay: Seconds(0),
                retention: Seconds(345_600),
                kind: QueueKind::Standard,
                dead_letter: None,
            }
        );
    }

    #[test]
    fn apply_keeps_unmentioned_attributes_and_the_dead_letter_configuration() {
        let dead_letter = DeadLetter {
            queue: "dead".into(),
            max_receive_count: 3,
        };
        let mut attributes = QueueAttributes::from_patch(
            &AttributePatch {
                fifo: Some(true),
                ..AttributePatch::default()
            },
            Some(dead_letter.clone()),
        );
        attributes.apply(
            &AttributePatch {
                delay: Some(Seconds(9)),
                content_based_deduplication: Some(true),
                ..AttributePatch::default()
            },
            None,
        );
        assert_eq!(
            attributes,
            QueueAttributes {
                visibility_timeout: Seconds(30),
                delay: Seconds(9),
                retention: Seconds(345_600),
                kind: QueueKind::Fifo {
                    content_based_deduplication: true
                },
                dead_letter: Some(dead_letter),
            }
        );
    }

    #[test]
    fn dead_letter_attributes_must_come_together() {
        let only_queue = AttributePatch {
            dead_letter_queue: Some("dead".into()),
            ..AttributePatch::default()
        };
        let only_count = AttributePatch {
            max_receive_count: Some(2),
            ..AttributePatch::default()
        };
        for patch in [only_queue, only_count] {
            let error = patch.dead_letter().unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidParameterValue);
        }
    }

    #[test]
    fn forget_dead_letter_only_matches_the_named_queue() {
        let mut attributes = QueueAttributes::from_patch(
            &AttributePatch::default(),
            Some(DeadLetter {
                queue: "dead".into(),
                max_receive_count: 1,
            }),
        );
        attributes.forget_dead_letter("other");
        assert_eq!(
            attributes.dead_letter,
            Some(DeadLetter {
                queue: "dead".into(),
                max_receive_count: 1
            })
        );
        attributes.forget_dead_letter("dead");
        assert_eq!(attributes.dead_letter, None);
    }
}
