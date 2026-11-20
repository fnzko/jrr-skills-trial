use crate::error::Error;

pub const DEFAULT_VISIBILITY_TIMEOUT: u32 = 30;
pub const DEFAULT_RETENTION_SECONDS: u32 = 345_600;

/// Standard or FIFO. Content-based deduplication only exists on FIFO queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl QueueKind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo { .. })
    }
}

/// Dead-letter configuration: both parts are always set together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u32,
}

/// The attributes of a queue after defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub visibility_timeout: u32,
    pub delay_seconds: u32,
    pub retention_seconds: u32,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetter>,
}

/// Attributes as given in a request; absent ones are `None`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributeSet {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeSet {
    /// The dead-letter configuration of the request, if it names one.
    /// Naming only half of it is invalid.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "Attributes",
                "must set DeadLetterQueue and MaxReceiveCount together",
            )),
        }
    }
}

impl Config {
    /// The configuration a new queue gets: given attributes over defaults.
    pub fn from_attributes(attributes: &AttributeSet, dead_letter: Option<DeadLetter>) -> Self {
        let kind = if attributes.fifo.unwrap_or(false) {
            QueueKind::Fifo {
                content_based_deduplication: attributes
                    .content_based_deduplication
                    .unwrap_or(false),
            }
        } else {
            QueueKind::Standard
        };
        Self {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay_seconds: attributes.delay_seconds.unwrap_or(0),
            retention_seconds: attributes
                .retention_seconds
                .unwrap_or(DEFAULT_RETENTION_SECONDS),
            kind,
            dead_letter,
        }
    }

    /// This configuration with the attributes present in `attributes`
    /// replaced. `Fifo` is ignored (a queue never changes type) and a
    /// dead-letter configuration is only replaced, never removed.
    pub fn updated(&self, attributes: &AttributeSet, dead_letter: Option<DeadLetter>) -> Self {
        let kind = match (self.kind, attributes.content_based_deduplication) {
            (QueueKind::Fifo { .. }, Some(content_based_deduplication)) => QueueKind::Fifo {
                content_based_deduplication,
            },
            (kind, _) => kind,
        };
        Self {
            visibility_timeout: attributes
                .visibility_timeout
                .unwrap_or(self.visibility_timeout),
            delay_seconds: attributes.delay_seconds.unwrap_or(self.delay_seconds),
            retention_seconds: attributes
                .retention_seconds
                .unwrap_or(self.retention_seconds),
            kind,
            dead_letter: dead_letter.or_else(|| self.dead_letter.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn defaults_apply_to_a_new_standard_queue() {
        let config = Config::from_attributes(&AttributeSet::default(), None);
        assert_eq!(
            config,
            Config {
                visibility_timeout: 30,
                delay_seconds: 0,
                retention_seconds: 345_600,
                kind: QueueKind::Standard,
                dead_letter: None,
            }
        );
    }

    #[test]
    fn update_keeps_unspecified_attributes_and_dead_letter() {
        let dead_letter = DeadLetter {
            queue: "dlq".into(),
            max_receive_count: 3,
        };
        let base = Config::from_attributes(
            &AttributeSet {
                delay_seconds: Some(5),
                ..AttributeSet::default()
            },
            Some(dead_letter.clone()),
        );
        let changed = base.updated(
            &AttributeSet {
                visibility_timeout: Some(7),
                ..AttributeSet::default()
            },
            None,
        );
        assert_eq!(changed.visibility_timeout, 7);
        assert_eq!(changed.delay_seconds, 5);
        assert_eq!(changed.dead_letter, Some(dead_letter));
    }

    #[test]
    fn half_a_dead_letter_configuration_is_invalid() {
        let only_queue = AttributeSet {
            dead_letter_queue: Some("dlq".into()),
            ..AttributeSet::default()
        };
        let only_count = AttributeSet {
            max_receive_count: Some(2),
            ..AttributeSet::default()
        };
        for attributes in [only_queue, only_count] {
            let error = attributes.dead_letter().unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidParameterValue);
        }
    }
}
