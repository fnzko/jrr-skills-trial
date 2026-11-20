//! Queue attributes: the stored form and the partial form a request carries.

use crate::types::{QueueName, Seconds};

pub const DEFAULT_VISIBILITY_TIMEOUT: Seconds = Seconds(30);
pub const DEFAULT_DELAY: Seconds = Seconds(0);
pub const DEFAULT_RETENTION: Seconds = Seconds(345_600);

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: u32,
}

/// The attributes of an existing queue, with defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: Seconds,
    pub delay: Seconds,
    pub retention: Seconds,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetter>,
}

/// The attributes a request names; values are range-checked but not yet related to other queues.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributeSet {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeSet {
    /// The kind of queue this request describes, defaults included.
    pub fn kind(&self) -> QueueKind {
        if self.fifo.unwrap_or(false) {
            QueueKind::Fifo {
                content_based_deduplication: self.content_based_deduplication.unwrap_or(false),
            }
        } else {
            QueueKind::Standard
        }
    }

    pub fn has_exactly_one_dead_letter_part(&self) -> bool {
        self.dead_letter_queue.is_some() != self.max_receive_count.is_some()
    }
}

impl Attributes {
    /// A new queue's attributes: the request over the defaults.
    pub fn from_request(request: &AttributeSet, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: request
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: request.delay.unwrap_or(DEFAULT_DELAY),
            retention: request.retention.unwrap_or(DEFAULT_RETENTION),
            kind: request.kind(),
            dead_letter,
        }
    }

    /// Whether creating a queue from `request` would give exactly these attributes.
    pub fn matches_request(&self, request: &AttributeSet) -> bool {
        let same_dead_letter = match (
            &self.dead_letter,
            &request.dead_letter_queue,
            request.max_receive_count,
        ) {
            (None, None, None) => true,
            (Some(current), Some(queue), Some(max)) => {
                current.queue.as_str() == queue && current.max_receive_count == max
            }
            _ => false,
        };
        same_dead_letter
            && self.visibility_timeout
                == request
                    .visibility_timeout
                    .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
            && self.delay == request.delay.unwrap_or(DEFAULT_DELAY)
            && self.retention == request.retention.unwrap_or(DEFAULT_RETENTION)
            && self.kind == request.kind()
    }

    /// Applies the attributes `update` names. `Fifo` is never changed here and
    /// `dead_letter` replaces the configuration only when given.
    pub fn apply(&mut self, update: &AttributeSet, dead_letter: Option<DeadLetter>) {
        if let Some(value) = update.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = update.delay {
            self.delay = value;
        }
        if let Some(value) = update.retention {
            self.retention = value;
        }
        if let (
            QueueKind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut self.kind, update.content_based_deduplication)
        {
            *content_based_deduplication = value;
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
            queue: QueueName::parse(queue).unwrap(),
            max_receive_count: max,
        }
    }

    #[test]
    fn empty_request_matches_default_attributes() {
        let attributes = Attributes::from_request(&AttributeSet::default(), None);
        assert_eq!(attributes.visibility_timeout, Seconds(30));
        assert_eq!(attributes.retention, Seconds(345_600));
        assert!(attributes.matches_request(&AttributeSet::default()));
    }

    #[test]
    fn request_with_a_different_value_does_not_match() {
        let attributes = Attributes::from_request(&AttributeSet::default(), None);
        let other = AttributeSet {
            delay: Some(Seconds(1)),
            ..AttributeSet::default()
        };
        assert!(!attributes.matches_request(&other));
    }

    #[test]
    fn dead_letter_configuration_must_agree_on_both_parts() {
        let request = AttributeSet {
            dead_letter_queue: Some("dlq".into()),
            max_receive_count: Some(3),
            ..AttributeSet::default()
        };
        let attributes = Attributes::from_request(&request, Some(dead_letter("dlq", 3)));
        assert!(attributes.matches_request(&request));
        assert!(!attributes.matches_request(&AttributeSet::default()));
        let other_count = AttributeSet {
            max_receive_count: Some(4),
            ..request
        };
        assert!(!attributes.matches_request(&other_count));
    }

    #[test]
    fn apply_changes_only_named_attributes() {
        let mut attributes =
            Attributes::from_request(&AttributeSet::default(), Some(dead_letter("dlq", 3)));
        attributes.apply(
            &AttributeSet {
                retention: Some(Seconds(60)),
                ..AttributeSet::default()
            },
            None,
        );
        assert_eq!(attributes.retention, Seconds(60));
        assert_eq!(attributes.visibility_timeout, Seconds(30));
        assert_eq!(attributes.dead_letter, Some(dead_letter("dlq", 3)));
    }

    #[test]
    fn apply_replaces_dead_letter_and_fifo_deduplication() {
        let fifo = AttributeSet {
            fifo: Some(true),
            ..AttributeSet::default()
        };
        let mut attributes = Attributes::from_request(&fifo, None);
        attributes.apply(
            &AttributeSet {
                content_based_deduplication: Some(true),
                ..AttributeSet::default()
            },
            Some(dead_letter("other.fifo", 5)),
        );
        assert_eq!(
            attributes.kind,
            QueueKind::Fifo {
                content_based_deduplication: true
            }
        );
        assert_eq!(attributes.dead_letter, Some(dead_letter("other.fifo", 5)));
    }
}
