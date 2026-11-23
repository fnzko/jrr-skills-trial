use serde_json::Value;

use crate::error::Error;
use crate::json::{boolean, integer, string};
use crate::types::{DelaySeconds, MaxReceiveCount, QueueName, RetentionSeconds, VisibilityTimeout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Standard,
    Fifo { content_based_deduplication: bool },
}

impl Kind {
    pub fn is_fifo(self) -> bool {
        matches!(self, Kind::Fifo { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub queue: QueueName,
    pub max_receive_count: MaxReceiveCount,
}

/// The attributes a queue holds. A dead-letter queue and its receive limit
/// only exist together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub visibility_timeout: VisibilityTimeout,
    pub delay: DelaySeconds,
    pub retention: RetentionSeconds,
    pub kind: Kind,
    pub dead_letter: Option<DeadLetter>,
}

/// The attributes named in a request, each type- and range-checked on its own.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RequestedAttributes {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<MaxReceiveCount>,
}

impl RequestedAttributes {
    /// Reads an `Attributes` parameter. Members that are `null` count as absent.
    pub fn parse(value: &Value) -> Result<Self, Error> {
        let Value::Object(members) = value else {
            return Err(Error::invalid_value("Attributes must be an object"));
        };
        let mut requested = Self::default();
        for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
            match name.as_str() {
                "VisibilityTimeout" => {
                    requested.visibility_timeout = Some(integer(name, value)?);
                }
                "DelaySeconds" => requested.delay = Some(integer(name, value)?),
                "RetentionSeconds" => requested.retention = Some(integer(name, value)?),
                "Fifo" => requested.fifo = Some(boolean(name, value)?),
                "ContentBasedDeduplication" => {
                    requested.content_based_deduplication = Some(boolean(name, value)?);
                }
                "DeadLetterQueue" => {
                    requested.dead_letter_queue = Some(string(name, value)?.to_owned());
                }
                "MaxReceiveCount" => requested.max_receive_count = Some(integer(name, value)?),
                unknown => {
                    return Err(Error::invalid_value(format!(
                        "{unknown:?} is not a queue attribute"
                    )));
                }
            }
        }
        Ok(requested)
    }

    /// The requested dead-letter configuration; it must be given whole or not at all.
    pub fn dead_letter(&self) -> Result<Option<(&str, MaxReceiveCount)>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max)) => Ok(Some((queue, max))),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    /// The queue kind this request describes, with defaults applied.
    pub fn kind(&self) -> Kind {
        if self.fifo.unwrap_or(false) {
            Kind::Fifo {
                content_based_deduplication: self.content_based_deduplication.unwrap_or(false),
            }
        } else {
            Kind::Standard
        }
    }
}

impl Attributes {
    /// Attributes of a new queue: the request over the defaults.
    pub fn from_request(requested: &RequestedAttributes, dead_letter: Option<DeadLetter>) -> Self {
        Self {
            visibility_timeout: requested.visibility_timeout.unwrap_or_default(),
            delay: requested.delay.unwrap_or_default(),
            retention: requested.retention.unwrap_or_default(),
            kind: requested.kind(),
            dead_letter,
        }
    }

    /// Whether creating a queue with this request would change nothing.
    pub fn satisfies(
        &self,
        requested: &RequestedAttributes,
        dead_letter: Option<(&str, MaxReceiveCount)>,
    ) -> bool {
        let current = self
            .dead_letter
            .as_ref()
            .map(|d| (d.queue.as_str(), d.max_receive_count));
        requested.visibility_timeout.unwrap_or_default() == self.visibility_timeout
            && requested.delay.unwrap_or_default() == self.delay
            && requested.retention.unwrap_or_default() == self.retention
            && requested.kind() == self.kind
            && dead_letter == current
    }

    /// Applies the attributes a `SetQueueAttributes` request names. The caller
    /// has already rejected `Fifo` and a content-based flag the queue cannot hold.
    pub fn update(&mut self, requested: &RequestedAttributes, dead_letter: Option<DeadLetter>) {
        if let Some(value) = requested.visibility_timeout {
            self.visibility_timeout = value;
        }
        if let Some(value) = requested.delay {
            self.delay = value;
        }
        if let Some(value) = requested.retention {
            self.retention = value;
        }
        if let (
            Kind::Fifo {
                content_based_deduplication,
            },
            Some(value),
        ) = (&mut self.kind, requested.content_based_deduplication)
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
    use serde_json::json;

    use super::*;
    use crate::error::ErrorCode;

    fn parse(value: Value) -> Result<RequestedAttributes, ErrorCode> {
        RequestedAttributes::parse(&value).map_err(|e| e.code)
    }

    #[test]
    fn parses_every_attribute() {
        let requested = parse(json!({
            "VisibilityTimeout": 5, "DelaySeconds": 6, "RetentionSeconds": 60,
            "Fifo": true, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dead", "MaxReceiveCount": 3,
        }))
        .unwrap();
        assert_eq!(
            requested.visibility_timeout.map(VisibilityTimeout::get),
            Some(5)
        );
        assert_eq!(requested.delay.map(DelaySeconds::get), Some(6));
        assert_eq!(requested.retention.map(RetentionSeconds::get), Some(60));
        assert_eq!(requested.fifo, Some(true));
        assert_eq!(requested.content_based_deduplication, Some(false));
        assert_eq!(requested.dead_letter_queue.as_deref(), Some("dead"));
        assert_eq!(
            requested.max_receive_count.map(MaxReceiveCount::get),
            Some(3)
        );
    }

    #[test]
    fn null_members_are_absent() {
        let requested = parse(json!({"VisibilityTimeout": null, "Bogus": null})).unwrap();
        assert_eq!(requested, RequestedAttributes::default());
    }

    #[test]
    fn rejects_bad_types_ranges_and_unknown_names() {
        let bad = [
            json!([]),
            json!("x"),
            json!({"Bogus": 1}),
            json!({"VisibilityTimeout": "30"}),
            json!({"VisibilityTimeout": 30.0}),
            json!({"VisibilityTimeout": 3e1}),
            json!({"VisibilityTimeout": 43201}),
            json!({"DelaySeconds": -1}),
            json!({"RetentionSeconds": 59}),
            json!({"Fifo": "true"}),
            json!({"DeadLetterQueue": 1}),
            json!({"MaxReceiveCount": 0}),
        ];
        for value in bad {
            assert_eq!(
                parse(value.clone()),
                Err(ErrorCode::InvalidParameterValue),
                "{value}"
            );
        }
    }

    #[test]
    fn dead_letter_configuration_comes_whole_or_not_at_all() {
        let both = parse(json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2})).unwrap();
        assert_eq!(
            both.dead_letter().unwrap().map(|(q, m)| (q, m.get())),
            Some(("d", 2))
        );
        let none = parse(json!({})).unwrap();
        assert_eq!(none.dead_letter().unwrap(), None);
        let half = parse(json!({"MaxReceiveCount": 2})).unwrap();
        assert_eq!(
            half.dead_letter().unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn requests_over_defaults_equal_a_default_queue() {
        let defaults = Attributes::from_request(&RequestedAttributes::default(), None);
        let explicit = parse(json!({"VisibilityTimeout": 30, "Fifo": false})).unwrap();
        assert!(defaults.satisfies(&explicit, None));
        let different = parse(json!({"DelaySeconds": 1})).unwrap();
        assert!(!defaults.satisfies(&different, None));
    }

    #[test]
    fn update_changes_only_named_attributes() {
        let mut attributes = Attributes::from_request(&parse(json!({"Fifo": true})).unwrap(), None);
        attributes.update(
            &parse(json!({"DelaySeconds": 9, "ContentBasedDeduplication": true})).unwrap(),
            None,
        );
        assert_eq!(attributes.delay.get(), 9);
        assert_eq!(attributes.visibility_timeout.get(), 30);
        assert_eq!(
            attributes.kind,
            Kind::Fifo {
                content_based_deduplication: true
            }
        );
    }
}
