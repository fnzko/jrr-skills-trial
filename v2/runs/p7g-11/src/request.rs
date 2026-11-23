//! Turns one input line into a typed request, applying the checks of spec section 2 that need
//! no queue state: shape, required parameters, then parameter values.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{
    Body, Bounded, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, QueueName,
    RetentionSeconds, TickSeconds, VisibilityTimeout,
};

/// Attributes as given in a request; the defaults are applied later, where the command's
/// own rules say how.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Attributes<'a> {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<&'a str>,
    pub max_receive_count: Option<MaxReceiveCount>,
}

impl<'a> Attributes<'a> {
    const NAMES: [&'static str; 7] = [
        "VisibilityTimeout",
        "DelaySeconds",
        "RetentionSeconds",
        "Fifo",
        "ContentBasedDeduplication",
        "DeadLetterQueue",
        "MaxReceiveCount",
    ];

    /// The dead-letter configuration, which is either complete or absent.
    pub fn dead_letter(&self) -> Result<Option<(&'a str, MaxReceiveCount)>, Error> {
        match (self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }

    fn from_value(value: &'a Value) -> Result<Self, Error> {
        let Value::Object(map) = value else {
            return Err(wrong_type("Attributes", "an object"));
        };
        if let Some(unknown) = map
            .iter()
            .find(|(name, value)| !value.is_null() && !Self::NAMES.contains(&name.as_str()))
        {
            return Err(Error::invalid_value(format!(
                "unknown attribute {:?}",
                unknown.0
            )));
        }
        let params = Params(map);
        Ok(Self {
            visibility_timeout: params.bounded("VisibilityTimeout")?,
            delay: params.bounded("DelaySeconds")?,
            retention: params.bounded("RetentionSeconds")?,
            fifo: params.boolean("Fifo")?,
            content_based_deduplication: params.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: params.string("DeadLetterQueue")?,
            max_receive_count: params.bounded("MaxReceiveCount")?,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Request<'a> {
    CreateQueue {
        name: QueueName,
        attributes: Attributes<'a>,
    },
    SetQueueAttributes {
        queue: &'a str,
        attributes: Attributes<'a>,
    },
    GetQueueAttributes {
        queue: &'a str,
    },
    ListQueues {
        prefix: Option<&'a str>,
    },
    DeleteQueue {
        queue: &'a str,
    },
    PurgeQueue {
        queue: &'a str,
    },
    Tick {
        seconds: TickSeconds,
    },
    SendMessage {
        queue: &'a str,
        body: Body,
        delay: Option<DelaySeconds>,
        group: Option<GroupId>,
        deduplication: Option<DeduplicationId>,
    },
    ReceiveMessage {
        queue: &'a str,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    },
    DeleteMessage {
        queue: &'a str,
        receipt_handle: &'a str,
    },
    ChangeMessageVisibility {
        queue: &'a str,
        receipt_handle: &'a str,
        visibility_timeout: VisibilityTimeout,
    },
}

/// A line that is a JSON object with a string `Command`, not yet checked against that command.
#[derive(Debug)]
pub struct Envelope {
    command: String,
    params: Map<String, Value>,
}

impl Envelope {
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let text = std::str::from_utf8(line)
            .map_err(|_| Error::invalid_request("the line is not valid UTF-8"))?;
        let Value::Object(mut params) = serde_json::from_str(text)
            .map_err(|err| Error::invalid_request(format!("the line is not JSON: {err}")))?
        else {
            return Err(Error::invalid_request("the line is not a JSON object"));
        };
        match params.remove("Command") {
            Some(Value::String(command)) => Ok(Self { command, params }),
            _ => Err(Error::invalid_request("Command must be a string")),
        }
    }

    pub fn request(&self) -> Result<Request<'_>, Error> {
        let params = Params(&self.params);
        match self.command.as_str() {
            "CreateQueue" => {
                let name = params.required("QueueName")?;
                let attributes = params.optional("Attributes");
                // The name is checked before every other parameter.
                let name = string("QueueName", name)?;
                let name = QueueName::parse(name).ok_or_else(|| {
                    Error::new(
                        ErrorCode::QueueNameInvalid,
                        format!("{name:?} is not a valid queue name"),
                    )
                })?;
                let attributes = attributes
                    .map(Attributes::from_value)
                    .transpose()?
                    .unwrap_or_default();
                Ok(Request::CreateQueue { name, attributes })
            }
            "SetQueueAttributes" => {
                let queue = params.required("QueueName")?;
                let attributes = params.required("Attributes")?;
                Ok(Request::SetQueueAttributes {
                    queue: string("QueueName", queue)?,
                    attributes: Attributes::from_value(attributes)?,
                })
            }
            "GetQueueAttributes" => Ok(Request::GetQueueAttributes {
                queue: params.queue()?,
            }),
            "ListQueues" => Ok(Request::ListQueues {
                prefix: params.string("Prefix")?,
            }),
            "DeleteQueue" => Ok(Request::DeleteQueue {
                queue: params.queue()?,
            }),
            "PurgeQueue" => Ok(Request::PurgeQueue {
                queue: params.queue()?,
            }),
            "Tick" => {
                let seconds = params.required("Seconds")?;
                Ok(Request::Tick {
                    seconds: bounded("Seconds", seconds)?,
                })
            }
            "SendMessage" => {
                let queue = params.required("QueueName")?;
                let body = params.required("Body")?;
                let queue = string("QueueName", queue)?;
                let body = string("Body", body)?;
                let body = Body::parse(body)
                    .ok_or_else(|| Error::invalid_value("Body must be 1 to 1024 bytes long"))?;
                let delay = params.bounded("DelaySeconds")?;
                let group = params
                    .string("GroupId")?
                    .map(|text| GroupId::parse(text).ok_or_else(|| bad_id("GroupId")))
                    .transpose()?;
                let deduplication = params
                    .string("DeduplicationId")?
                    .map(|text| {
                        DeduplicationId::parse(text).ok_or_else(|| bad_id("DeduplicationId"))
                    })
                    .transpose()?;
                Ok(Request::SendMessage {
                    queue,
                    body,
                    delay,
                    group,
                    deduplication,
                })
            }
            "ReceiveMessage" => {
                let queue = params.queue()?;
                Ok(Request::ReceiveMessage {
                    queue,
                    max_messages: params
                        .bounded("MaxMessages")?
                        .unwrap_or(MaxMessages::new_const(1)),
                    visibility_timeout: params.bounded("VisibilityTimeout")?,
                })
            }
            "DeleteMessage" => {
                let queue = params.required("QueueName")?;
                let handle = params.required("ReceiptHandle")?;
                Ok(Request::DeleteMessage {
                    queue: string("QueueName", queue)?,
                    receipt_handle: string("ReceiptHandle", handle)?,
                })
            }
            "ChangeMessageVisibility" => {
                let queue = params.required("QueueName")?;
                let handle = params.required("ReceiptHandle")?;
                let timeout = params.required("VisibilityTimeout")?;
                Ok(Request::ChangeMessageVisibility {
                    queue: string("QueueName", queue)?,
                    receipt_handle: string("ReceiptHandle", handle)?,
                    visibility_timeout: bounded("VisibilityTimeout", timeout)?,
                })
            }
            other => Err(Error::invalid_request(format!("unknown command {other:?}"))),
        }
    }
}

/// The members of a request or of an attribute object. A `null` member counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn optional(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &str) -> Result<&'a Value, Error> {
        self.optional(name).ok_or_else(|| Error::missing(name))
    }

    /// The `QueueName` of a command that has no other parameter.
    fn queue(&self) -> Result<&'a str, Error> {
        string("QueueName", self.required("QueueName")?)
    }

    fn string(&self, name: &str) -> Result<Option<&'a str>, Error> {
        self.optional(name)
            .map(|value| string(name, value))
            .transpose()
    }

    fn boolean(&self, name: &str) -> Result<Option<bool>, Error> {
        self.optional(name)
            .map(|value| value.as_bool().ok_or_else(|| wrong_type(name, "a boolean")))
            .transpose()
    }

    fn bounded<const MIN: u32, const MAX: u32>(
        &self,
        name: &str,
    ) -> Result<Option<Bounded<MIN, MAX>>, Error> {
        self.optional(name)
            .map(|value| bounded(name, value))
            .transpose()
    }
}

fn wrong_type(name: &str, expected: &str) -> Error {
    Error::invalid_value(format!("{name} must be {expected}"))
}

fn bad_id(name: &str) -> Error {
    Error::invalid_value(format!(
        "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
    ))
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value.as_str().ok_or_else(|| wrong_type(name, "a string"))
}

/// Accepts only a JSON number written as an integer, so `30.0` and `3e1` are refused.
fn bounded<const MIN: u32, const MAX: u32>(
    name: &str,
    value: &Value,
) -> Result<Bounded<MIN, MAX>, Error> {
    let Value::Number(number) = value else {
        return Err(wrong_type(name, "an integer"));
    };
    let integer = number
        .as_i64()
        .map(i128::from)
        .or_else(|| number.as_u64().map(i128::from))
        .ok_or_else(|| wrong_type(name, "an integer"))?;
    Bounded::new(integer)
        .ok_or_else(|| Error::invalid_value(format!("{name} must be from {MIN} to {MAX}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Request<'static>, Error> {
        // The envelope must outlive the request; leaking is fine for a test process.
        let envelope = Box::leak(Box::new(Envelope::parse(line.as_bytes())?));
        envelope.request()
    }

    fn code(line: &str) -> ErrorCode {
        parse(line).expect_err("the line should be refused").code
    }

    #[test]
    fn shape_errors_are_invalid_requests() {
        for line in [
            "",
            "   ",
            "[]",
            "3",
            "{",
            r#"{"Command": 1}"#,
            r#"{"Command": null}"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "Tick", "Seconds": 1} x"#,
        ] {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line}");
        }
        let invalid_utf8 = Envelope::parse(b"{\"Command\": \"\xff\"}").unwrap_err();
        assert_eq!(invalid_utf8.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        for (literal, ok) in [
            ("30", true),
            ("-0", true),
            ("30.0", false),
            ("3e1", false),
            ("\"30\"", false),
            ("true", false),
            ("-1", false),
            ("99999999999999999999999", false),
        ] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {literal}}}"#);
            let expected = if ok {
                None
            } else {
                Some(ErrorCode::InvalidParameterValue)
            };
            assert_eq!(parse(&line).err().map(|e| e.code), expected, "{literal}");
        }
    }

    #[test]
    fn required_parameters_are_checked_before_values() {
        let line = r#"{"Command": "SendMessage", "Body": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let line = r#"{"Command": "SendMessage", "QueueName": "q", "Body": null}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let line = r#"{"Command": "SendMessage", "QueueName": "q", "Body": 5}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command": "CreateQueue", "QueueName": "ab", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
        let line = r#"{"Command": "CreateQueue", "QueueName": 7, "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn null_attributes_are_absent_and_unknown_ones_are_refused() {
        let request = parse(
            r#"{"Command": "CreateQueue", "QueueName": "q",
                "Attributes": {"VisibilityTimeout": null, "Bogus": null, "DelaySeconds": 5}}"#,
        )
        .expect("null members are absent");
        let Request::CreateQueue { attributes, .. } = request else {
            panic!("expected CreateQueue, got {request:?}");
        };
        assert_eq!(
            attributes,
            Attributes {
                delay: DelaySeconds::new(5),
                ..Attributes::default()
            }
        );
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Bogus": 1}}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn dead_letter_attributes_must_come_as_a_pair() {
        let only_queue = Attributes {
            dead_letter_queue: Some("dlq"),
            ..Attributes::default()
        };
        assert_eq!(
            only_queue.dead_letter().unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(Attributes::default().dead_letter(), Ok(None));
    }
}
