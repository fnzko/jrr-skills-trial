//! Queue attributes: the requested changes (`AttributePatch`) and the
//! resolved values a queue holds (`Attributes`).

use serde_json::Value;

use crate::error::{Error, ErrorCode};
use crate::names::QueueName;
use crate::params::{self, invalid};

const DEFAULT_VISIBILITY_TIMEOUT: u64 = 30;
const DEFAULT_RETENTION: u64 = 345_600;

/// A queue's type. Content-based deduplication exists only on FIFO queues, so
/// the flag lives inside that variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: String,
    pub max_receive_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: u64,
    pub delay: u64,
    pub retention: u64,
    pub kind: QueueKind,
    pub dead_letter: Option<DeadLetter>,
}

/// The attributes named in a `CreateQueue` or `SetQueueAttributes` request.
/// Each value is valid on its own; how they combine is checked by the command.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttributePatch {
    pub visibility_timeout: Option<u64>,
    pub delay: Option<u64>,
    pub retention: Option<u64>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u64>,
}

impl AttributePatch {
    pub fn parse(value: &Value) -> Result<Self, Error> {
        let Value::Object(members) = value else {
            return Err(invalid("Attributes", "must be an object"));
        };
        let mut patch = Self::default();
        for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    patch.visibility_timeout = Some(params::integer(name, value, 0..=43_200)?)
                }
                "DelaySeconds" => patch.delay = Some(params::integer(name, value, 0..=900)?),
                "RetentionSeconds" => {
                    patch.retention = Some(params::integer(name, value, 60..=1_209_600)?)
                }
                "Fifo" => patch.fifo = Some(params::boolean(name, value)?),
                "ContentBasedDeduplication" => {
                    patch.content_based_deduplication = Some(params::boolean(name, value)?)
                }
                "DeadLetterQueue" => {
                    patch.dead_letter_queue = Some(params::string(name, value)?.to_owned())
                }
                "MaxReceiveCount" => {
                    patch.max_receive_count = Some(params::integer(name, value, 1..=1000)?)
                }
                _ => return Err(invalid(name, "is not a queue attribute")),
            }
        }
        Ok(patch)
    }

    /// The dead-letter configuration this patch carries. Both halves must be
    /// present or both absent.
    fn dead_letter(&self) -> Result<Option<DeadLetter>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(invalid(
                "DeadLetterQueue",
                "and MaxReceiveCount must be given together",
            )),
        }
    }
}

impl Attributes {
    /// Resolves a `CreateQueue` request against the defaults. Applies rules 1
    /// to 3 of the command, in order.
    pub fn for_new_queue(name: &QueueName, patch: &AttributePatch) -> Result<Self, Error> {
        let fifo = patch.fifo.unwrap_or(false);
        if name.has_fifo_suffix() != fifo {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "the .fifo suffix and the Fifo attribute must agree",
            ));
        }
        let content_based_deduplication = patch.content_based_deduplication.unwrap_or(false);
        let kind = match (fifo, content_based_deduplication) {
            (true, content_based_deduplication) => QueueKind::Fifo {
                content_based_deduplication,
            },
            (false, false) => QueueKind::Standard,
            (false, true) => return Err(dedup_needs_fifo()),
        };
        Ok(Self {
            visibility_timeout: patch
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT),
            delay: patch.delay.unwrap_or(0),
            retention: patch.retention.unwrap_or(DEFAULT_RETENTION),
            kind,
            dead_letter: patch.dead_letter()?,
        })
    }

    /// The attributes after a `SetQueueAttributes` request. Applies rules 1 to
    /// 3 of the command, in order; the caller checks the dead-letter queue.
    pub fn updated(&self, patch: &AttributePatch) -> Result<Self, Error> {
        if patch.fifo.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "the type of a queue cannot change",
            ));
        }
        let kind = match (self.kind, patch.content_based_deduplication) {
            (QueueKind::Standard, Some(true)) => return Err(dedup_needs_fifo()),
            (QueueKind::Fifo { .. }, Some(content_based_deduplication)) => QueueKind::Fifo {
                content_based_deduplication,
            },
            (kind, _) => kind,
        };
        let dead_letter = patch.dead_letter()?.or_else(|| self.dead_letter.clone());
        Ok(Self {
            visibility_timeout: patch.visibility_timeout.unwrap_or(self.visibility_timeout),
            delay: patch.delay.unwrap_or(self.delay),
            retention: patch.retention.unwrap_or(self.retention),
            kind,
            dead_letter,
        })
    }

    pub fn is_fifo(&self) -> bool {
        matches!(self.kind, QueueKind::Fifo { .. })
    }
}

fn dedup_needs_fifo() -> Error {
    invalid(
        "ContentBasedDeduplication",
        "can only be true on a FIFO queue",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn patch(value: Value) -> AttributePatch {
        AttributePatch::parse(&value).unwrap()
    }

    fn name(text: &str) -> QueueName {
        QueueName::parse(text).unwrap()
    }

    #[test]
    fn defaults_apply_to_an_empty_request() {
        let attributes = Attributes::for_new_queue(&name("q"), &patch(json!({}))).unwrap();
        assert_eq!(
            attributes,
            Attributes {
                visibility_timeout: 30,
                delay: 0,
                retention: 345_600,
                kind: QueueKind::Standard,
                dead_letter: None,
            }
        );
    }

    #[test]
    fn null_attributes_count_as_absent() {
        let parsed = patch(json!({"VisibilityTimeout": null, "DelaySeconds": 5}));
        assert_eq!(
            parsed,
            AttributePatch {
                delay: Some(5),
                ..AttributePatch::default()
            }
        );
    }

    #[test]
    fn invalid_attribute_values_are_rejected() {
        let cases = [
            json!(5),
            json!({"Bogus": 1}),
            json!({"VisibilityTimeout": 43201}),
            json!({"VisibilityTimeout": -1}),
            json!({"VisibilityTimeout": 3.0}),
            json!({"VisibilityTimeout": "3"}),
            json!({"DelaySeconds": 901}),
            json!({"RetentionSeconds": 59}),
            json!({"RetentionSeconds": 1_209_601}),
            json!({"Fifo": "true"}),
            json!({"ContentBasedDeduplication": 1}),
            json!({"DeadLetterQueue": 7}),
            json!({"MaxReceiveCount": 0}),
            json!({"MaxReceiveCount": 1001}),
        ];
        for case in cases {
            let error = AttributePatch::parse(&case).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidParameterValue, "{case}");
        }
    }

    #[test]
    fn create_checks_name_suffix_before_dedup_before_pairing() {
        let cases = [
            ("q.fifo", json!({}), ErrorCode::QueueNameInvalid),
            ("q", json!({"Fifo": true}), ErrorCode::QueueNameInvalid),
            (
                "q",
                json!({"ContentBasedDeduplication": true, "DeadLetterQueue": "d"}),
                ErrorCode::InvalidParameterValue,
            ),
            (
                "q.fifo",
                json!({"Fifo": true, "MaxReceiveCount": 3}),
                ErrorCode::InvalidParameterValue,
            ),
            (
                "q",
                json!({"Fifo": true, "ContentBasedDeduplication": true}),
                ErrorCode::QueueNameInvalid,
            ),
        ];
        for (queue, attributes, code) in cases {
            let error = Attributes::for_new_queue(&name(queue), &patch(attributes)).unwrap_err();
            assert_eq!(error.code(), code, "{queue}");
        }
    }

    #[test]
    fn create_builds_a_fifo_queue_with_dead_letter_config() {
        let attributes = Attributes::for_new_queue(
            &name("q.fifo"),
            &patch(json!({
                "Fifo": true,
                "ContentBasedDeduplication": true,
                "DeadLetterQueue": "dead.fifo",
                "MaxReceiveCount": 4,
            })),
        )
        .unwrap();
        assert_eq!(
            attributes.kind,
            QueueKind::Fifo {
                content_based_deduplication: true
            }
        );
        assert_eq!(
            attributes.dead_letter,
            Some(DeadLetter {
                queue: "dead.fifo".into(),
                max_receive_count: 4
            })
        );
    }

    #[test]
    fn update_keeps_what_the_patch_leaves_out() {
        let base = Attributes::for_new_queue(
            &name("q"),
            &patch(json!({"DelaySeconds": 7, "DeadLetterQueue": "d", "MaxReceiveCount": 2})),
        )
        .unwrap();
        let updated = base
            .updated(&patch(json!({"VisibilityTimeout": 1})))
            .unwrap();
        assert_eq!(
            updated,
            Attributes {
                visibility_timeout: 1,
                ..base
            }
        );
    }

    #[test]
    fn update_replaces_dead_letter_config_but_cannot_remove_it() {
        let base = Attributes::for_new_queue(
            &name("q"),
            &patch(json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2})),
        )
        .unwrap();
        let replaced = base
            .updated(&patch(
                json!({"DeadLetterQueue": "e", "MaxReceiveCount": 9}),
            ))
            .unwrap();
        assert_eq!(
            replaced.dead_letter,
            Some(DeadLetter {
                queue: "e".into(),
                max_receive_count: 9
            })
        );
        let kept = base
            .updated(&patch(json!({"DeadLetterQueue": null})))
            .unwrap();
        assert_eq!(kept.dead_letter, base.dead_letter);
    }

    #[test]
    fn update_rejections_follow_rule_order() {
        let standard = Attributes::for_new_queue(&name("q"), &patch(json!({}))).unwrap();
        let cases = [
            (json!({"Fifo": false}), ErrorCode::UnsupportedOperation),
            (
                json!({"Fifo": true, "ContentBasedDeduplication": true}),
                ErrorCode::UnsupportedOperation,
            ),
            (
                json!({"ContentBasedDeduplication": true, "MaxReceiveCount": 1}),
                ErrorCode::InvalidParameterValue,
            ),
            (
                json!({"DeadLetterQueue": "d"}),
                ErrorCode::InvalidParameterValue,
            ),
        ];
        for (request, code) in cases {
            let error = standard.updated(&patch(request.clone())).unwrap_err();
            assert_eq!(error.code(), code, "{request}");
        }
        assert!(
            standard
                .updated(&patch(json!({"ContentBasedDeduplication": false})))
                .is_ok()
        );
    }
}
