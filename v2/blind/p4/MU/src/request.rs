//! Turns one input line into a typed [`Request`], applying the first three
//! check stages of SPEC.md section 2 (shape, required parameters, values).

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, Expected};
use crate::types::{DeduplicationKey, GroupId, QueueName, Seconds};

const TICK_SECONDS: RangeInclusive<u32> = 0..=1_209_600;
const VISIBILITY_TIMEOUT: RangeInclusive<u32> = 0..=43_200;
const DELAY_SECONDS: RangeInclusive<u32> = 0..=900;
const RETENTION_SECONDS: RangeInclusive<u32> = 60..=1_209_600;
const MAX_RECEIVE_COUNT: RangeInclusive<u32> = 1..=1000;
const MAX_MESSAGES: RangeInclusive<u32> = 1..=10;
pub const MAX_BODY_BYTES: usize = 1024;

#[derive(Debug)]
pub enum Request {
    CreateQueue {
        name: QueueName,
        attributes: AttributeChanges,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeChanges,
    },
    ListQueues {
        prefix: Option<String>,
    },
    GetQueueAttributes {
        queue: String,
    },
    DeleteQueue {
        queue: String,
    },
    PurgeQueue {
        queue: String,
    },
    Tick {
        seconds: Seconds,
    },
    SendMessage {
        queue: String,
        message: NewMessage,
    },
    ReceiveMessage {
        queue: String,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: String,
        handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        handle: String,
        visibility_timeout: Seconds,
    },
}

/// The message parameters of `SendMessage`; which of them a queue accepts
/// depends on its kind.
#[derive(Debug)]
pub struct NewMessage {
    pub body: String,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationKey>,
}

/// The attributes named in an `Attributes` object, each checked against its
/// own type and range; how they combine is decided by the command.
#[derive(Debug, Default)]
pub struct AttributeChanges {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeChanges {
    /// The dead-letter configuration, if any; the spec only allows both
    /// halves together.
    pub fn dead_letter(&self) -> Result<Option<(&str, u32)>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            _ => Err(Error::PartialDeadLetterConfig),
        }
    }

    fn parse(object: Map<String, Value>) -> Result<Self, Error> {
        let mut changes = AttributeChanges::default();
        for (name, value) in object {
            if value.is_null() {
                continue;
            }
            let param = |name| Param { name, value };
            match name.as_str() {
                "VisibilityTimeout" => {
                    changes.visibility_timeout =
                        Some(param("VisibilityTimeout").seconds(VISIBILITY_TIMEOUT)?)
                }
                "DelaySeconds" => {
                    changes.delay = Some(param("DelaySeconds").seconds(DELAY_SECONDS)?)
                }
                "RetentionSeconds" => {
                    changes.retention = Some(param("RetentionSeconds").seconds(RETENTION_SECONDS)?)
                }
                "Fifo" => changes.fifo = Some(param("Fifo").boolean()?),
                "ContentBasedDeduplication" => {
                    changes.content_based_deduplication =
                        Some(param("ContentBasedDeduplication").boolean()?)
                }
                "DeadLetterQueue" => {
                    changes.dead_letter_queue = Some(param("DeadLetterQueue").string()?)
                }
                "MaxReceiveCount" => {
                    changes.max_receive_count =
                        Some(param("MaxReceiveCount").integer(MAX_RECEIVE_COUNT)?)
                }
                _ => return Err(Error::UnknownAttribute(name)),
            }
        }
        Ok(changes)
    }
}

pub fn parse(line: &[u8]) -> Result<Request, Error> {
    let text = std::str::from_utf8(line).map_err(|_| Error::NotUtf8)?;
    let Value::Object(members) = serde_json::from_str(text).map_err(Error::Malformed)? else {
        return Err(Error::NotAnObject);
    };
    let mut params = Params(members);
    let Some(Value::String(command)) = params.0.remove("Command") else {
        return Err(Error::NoCommand);
    };
    match command.as_str() {
        "CreateQueue" => {
            let name = params.required("QueueName")?;
            // The spec checks the name before every other parameter, and a
            // syntax error there has its own code.
            let name = QueueName::parse(name.string()?).map_err(Error::QueueNameSyntax)?;
            let attributes = params.optional("Attributes").map(Param::attributes);
            Ok(Request::CreateQueue {
                name,
                attributes: attributes.transpose()?.unwrap_or_default(),
            })
        }
        "SetQueueAttributes" => {
            let queue = params.required("QueueName");
            let attributes = params.required("Attributes");
            let (queue, attributes) = (queue?, attributes?);
            Ok(Request::SetQueueAttributes {
                queue: queue.string()?,
                attributes: attributes.attributes()?,
            })
        }
        "ListQueues" => Ok(Request::ListQueues {
            prefix: params.optional("Prefix").map(Param::string).transpose()?,
        }),
        "GetQueueAttributes" => Ok(Request::GetQueueAttributes {
            queue: params.required("QueueName")?.string()?,
        }),
        "DeleteQueue" => Ok(Request::DeleteQueue {
            queue: params.required("QueueName")?.string()?,
        }),
        "PurgeQueue" => Ok(Request::PurgeQueue {
            queue: params.required("QueueName")?.string()?,
        }),
        "SendMessage" => {
            let queue = params.required("QueueName");
            let body = params.required("Body");
            let (queue, body) = (queue?, body?);
            Ok(Request::SendMessage {
                queue: queue.string()?,
                message: NewMessage {
                    body: body.body()?,
                    delay: params
                        .optional("DelaySeconds")
                        .map(|delay| delay.seconds(DELAY_SECONDS))
                        .transpose()?,
                    group: params
                        .optional("GroupId")
                        .map(|group| group.identifier(GroupId::parse))
                        .transpose()?,
                    deduplication_id: params
                        .optional("DeduplicationId")
                        .map(|id| id.identifier(DeduplicationKey::parse_id))
                        .transpose()?,
                },
            })
        }
        "ReceiveMessage" => Ok(Request::ReceiveMessage {
            queue: params.required("QueueName")?.string()?,
            max_messages: params
                .optional("MaxMessages")
                .map(|max| max.integer(MAX_MESSAGES))
                .transpose()?
                .unwrap_or(1),
            visibility_timeout: params
                .optional("VisibilityTimeout")
                .map(|timeout| timeout.seconds(VISIBILITY_TIMEOUT))
                .transpose()?,
        }),
        "DeleteMessage" => {
            let queue = params.required("QueueName");
            let handle = params.required("ReceiptHandle");
            let (queue, handle) = (queue?, handle?);
            Ok(Request::DeleteMessage {
                queue: queue.string()?,
                handle: handle.string()?,
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.required("QueueName");
            let handle = params.required("ReceiptHandle");
            let timeout = params.required("VisibilityTimeout");
            let (queue, handle, timeout) = (queue?, handle?, timeout?);
            Ok(Request::ChangeMessageVisibility {
                queue: queue.string()?,
                handle: handle.string()?,
                visibility_timeout: timeout.seconds(VISIBILITY_TIMEOUT)?,
            })
        }
        "Tick" => {
            let seconds = params.required("Seconds")?;
            Ok(Request::Tick {
                seconds: seconds.seconds(TICK_SECONDS)?,
            })
        }
        _ => Err(Error::UnknownCommand(command)),
    }
}

/// The members of a request object other than `Command`.
///
/// Kept as a JSON map rather than deserialized into structs because the spec
/// fixes which error each kind of bad member produces and in what order, and
/// treats `null` as absent; serde's derived errors cannot express that.
struct Params(Map<String, Value>);

impl Params {
    fn optional(&mut self, name: &'static str) -> Option<Param> {
        match self.0.remove(name) {
            None | Some(Value::Null) => None,
            Some(value) => Some(Param { name, value }),
        }
    }

    fn required(&mut self, name: &'static str) -> Result<Param, Error> {
        self.optional(name).ok_or(Error::MissingParameter(name))
    }
}

/// A present, non-null parameter whose value is not checked yet.
struct Param {
    name: &'static str,
    value: Value,
}

impl Param {
    fn invalid(&self, expected: Expected) -> Error {
        Error::InvalidValue {
            name: self.name,
            expected,
        }
    }

    fn string(self) -> Result<String, Error> {
        match self.value {
            Value::String(text) => Ok(text),
            _ => Err(self.invalid(Expected::String)),
        }
    }

    fn boolean(self) -> Result<bool, Error> {
        match self.value {
            Value::Bool(flag) => Ok(flag),
            _ => Err(self.invalid(Expected::Boolean)),
        }
    }

    fn body(self) -> Result<String, Error> {
        match self.value {
            Value::String(body) if (1..=MAX_BODY_BYTES).contains(&body.len()) => Ok(body),
            _ => Err(self.invalid(Expected::Body)),
        }
    }

    fn identifier<T>(self, parse: fn(String) -> Result<T, String>) -> Result<T, Error> {
        let name = self.name;
        let invalid = || Error::InvalidValue {
            name,
            expected: Expected::Identifier,
        };
        match self.value {
            Value::String(id) => parse(id).map_err(|_| invalid()),
            _ => Err(invalid()),
        }
    }

    fn attributes(self) -> Result<AttributeChanges, Error> {
        match self.value {
            Value::Object(object) => AttributeChanges::parse(object),
            _ => Err(self.invalid(Expected::Object)),
        }
    }

    /// Accepts only a JSON number written as `-?[0-9]+` whose value lies in
    /// `range`; `30.0`, `3e1` and `"30"` are rejected even though they denote 30.
    fn integer(self, range: RangeInclusive<u32>) -> Result<u32, Error> {
        let expected = Expected::Integer {
            min: *range.start(),
            max: *range.end(),
        };
        let Value::Number(number) = &self.value else {
            return Err(self.invalid(expected));
        };
        let text = number.as_str();
        let (negative, digits) = match text.strip_prefix('-') {
            Some(digits) => (true, digits),
            None => (false, text),
        };
        // JSON grammar already guarantees a non-empty digit run without
        // leading zeros, so only fraction and exponent need ruling out.
        let value = if !digits.bytes().all(|b| b.is_ascii_digit()) {
            None
        } else if negative {
            (digits == "0").then_some(0)
        } else {
            digits.parse::<u32>().ok()
        };
        value
            .filter(|value| range.contains(value))
            .ok_or_else(|| self.invalid(expected))
    }

    fn seconds(self, range: RangeInclusive<u32>) -> Result<Seconds, Error> {
        self.integer(range).map(Seconds::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(line: &str) -> Result<u32, Error> {
        let mut params = Params(match serde_json::from_str(line) {
            Ok(Value::Object(members)) => members,
            other => panic!("test input must be an object, got {other:?}"),
        });
        params.required("Seconds")?.integer(TICK_SECONDS)
    }

    #[test]
    fn integer_syntax_and_range() {
        let cases: &[(&str, Option<u32>)] = &[
            (r#"{"Seconds": 30}"#, Some(30)),
            (r#"{"Seconds": 0}"#, Some(0)),
            (r#"{"Seconds": -0}"#, Some(0)),
            (r#"{"Seconds": 1209600}"#, Some(1_209_600)),
            (r#"{"Seconds": 1209601}"#, None),
            (r#"{"Seconds": -1}"#, None),
            (r#"{"Seconds": 30.0}"#, None),
            (r#"{"Seconds": -0.0}"#, None),
            (r#"{"Seconds": 3e1}"#, None),
            (r#"{"Seconds": 3E1}"#, None),
            (r#"{"Seconds": "30"}"#, None),
            (r#"{"Seconds": true}"#, None),
            (r#"{"Seconds": 99999999999999999999999}"#, None),
        ];
        for (line, expected) in cases {
            match (tick(line), expected) {
                (Ok(got), Some(want)) => assert_eq!(got, *want, "{line}"),
                (Err(Error::InvalidValue { name, .. }), None) => {
                    assert_eq!(name, "Seconds", "{line}")
                }
                (got, want) => panic!("{line}: got {got:?}, want {want:?}"),
            }
        }
    }

    #[test]
    fn null_parameter_counts_as_absent() {
        assert!(matches!(
            tick(r#"{"Seconds": null}"#),
            Err(Error::MissingParameter("Seconds"))
        ));
    }
}
