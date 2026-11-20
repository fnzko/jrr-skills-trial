//! Reads one input line into a typed `Command`. This layer runs the checks of SPEC.md section 2
//! that need no queue state: request shape, required parameters and parameter values. Checks
//! that need queue state or a command's own rules are left to the broker, so that the error
//! code of a line with several problems follows the order of section 2.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{ErrorCode, Failure};
use crate::types::{Body, DeduplicationId, GroupId, QueueName, QueueRef, ReceiptHandle, Seconds};

/// A parsed command with its parameters. Defaults that the spec fixes are already applied.
#[derive(Debug, PartialEq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributeRequest,
    },
    SetQueueAttributes {
        queue: QueueRef,
        attributes: AttributeRequest,
    },
    GetQueueAttributes {
        queue: QueueRef,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        queue: QueueRef,
    },
    PurgeQueue {
        queue: QueueRef,
    },
    Tick {
        seconds: Seconds,
    },
    SendMessage {
        queue: QueueRef,
        request: SendRequest,
    },
    ReceiveMessage {
        queue: QueueRef,
        max_messages: u32,
        visibility: Option<Seconds>,
    },
    /// `receipt` is None when the text is not in the form of any handle this process can issue.
    /// Such a handle can never match, which the broker reports only after the queue lookup.
    DeleteMessage {
        queue: QueueRef,
        receipt: Option<ReceiptHandle>,
    },
    /// `receipt` is None for the same reason as in `DeleteMessage`.
    ChangeMessageVisibility {
        queue: QueueRef,
        receipt: Option<ReceiptHandle>,
        visibility: Seconds,
    },
}

/// The parameters of a `SendMessage` command.
#[derive(Debug, PartialEq)]
pub struct SendRequest {
    pub body: Body,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication: Option<DeduplicationId>,
}

/// A dead-letter configuration as a request spells it, before the target queue is looked up.
#[derive(Debug, PartialEq)]
pub struct DeadLetterRequest {
    pub queue: QueueRef,
    pub max_receive_count: u32,
}

/// The attributes a request gave. Each is optional, and the rules of section 5 decide what a
/// given combination means.
#[derive(Debug, Default, PartialEq)]
pub struct AttributeRequest {
    pub visibility_timeout: Option<Seconds>,
    pub delay_seconds: Option<Seconds>,
    pub retention_seconds: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<QueueRef>,
    pub max_receive_count: Option<u32>,
}

/// Parses one input line, without its `\n`. Any failure is returned as the code section 2 gives.
pub fn parse_line(line: &[u8]) -> Result<Command, Failure> {
    let text = std::str::from_utf8(line).map_err(|error| {
        Failure::new(
            ErrorCode::InvalidRequest,
            format!("line is not UTF-8: {error}"),
        )
    })?;
    let object: Map<String, Value> = serde_json::from_str(text).map_err(|error| {
        Failure::new(
            ErrorCode::InvalidRequest,
            format!("line is not a JSON object: {error}"),
        )
    })?;
    let command = match object.get("Command") {
        Some(Value::String(name)) => name.as_str(),
        _ => {
            return Err(Failure::new(
                ErrorCode::InvalidRequest,
                "line has no string Command member",
            ))
        }
    };
    let params = Params(&object);
    match command {
        "CreateQueue" => {
            let queue = params.require("QueueName")?;
            let attributes = params.optional("Attributes");
            // QueueName goes first: a bad name is reported before any other parameter is read.
            let name = QueueName::try_from(text_of("QueueName", queue)?)?;
            Ok(Command::CreateQueue {
                name,
                attributes: attributes_of(attributes)?,
            })
        }
        "SetQueueAttributes" => {
            let queue = params.require("QueueName")?;
            let attributes = params.require("Attributes")?;
            Ok(Command::SetQueueAttributes {
                queue: queue_ref("QueueName", queue)?,
                attributes: attributes_of(Some(attributes))?,
            })
        }
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: queue_ref("QueueName", params.require("QueueName")?)?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params
                .optional("Prefix")
                .map(|value| text_of("Prefix", value).map(str::to_owned))
                .transpose()?,
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: queue_ref("QueueName", params.require("QueueName")?)?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: queue_ref("QueueName", params.require("QueueName")?)?,
        }),
        "Tick" => Ok(Command::Tick {
            seconds: seconds("Seconds", params.require("Seconds")?, 0..=1_209_600)?,
        }),
        "SendMessage" => {
            let queue = params.require("QueueName")?;
            let body = params.require("Body")?;
            Ok(Command::SendMessage {
                queue: queue_ref("QueueName", queue)?,
                request: SendRequest {
                    body: Body::try_from(text_of("Body", body)?.to_owned())?,
                    delay: params
                        .optional("DelaySeconds")
                        .map(|value| seconds("DelaySeconds", value, 0..=900))
                        .transpose()?,
                    group: params
                        .optional("GroupId")
                        .map(|value| GroupId::try_from(text_of("GroupId", value)?.to_owned()))
                        .transpose()?,
                    deduplication: params
                        .optional("DeduplicationId")
                        .map(|value| {
                            DeduplicationId::try_from(text_of("DeduplicationId", value)?.to_owned())
                        })
                        .transpose()?,
                },
            })
        }
        "ReceiveMessage" => Ok(Command::ReceiveMessage {
            queue: queue_ref("QueueName", params.require("QueueName")?)?,
            max_messages: params
                .optional("MaxMessages")
                .map(|value| integer("MaxMessages", value, 1..=10))
                .transpose()?
                .unwrap_or(1),
            visibility: params
                .optional("VisibilityTimeout")
                .map(|value| seconds("VisibilityTimeout", value, 0..=43_200))
                .transpose()?,
        }),
        "DeleteMessage" => {
            let queue = params.require("QueueName")?;
            let receipt = params.require("ReceiptHandle")?;
            Ok(Command::DeleteMessage {
                queue: queue_ref("QueueName", queue)?,
                receipt: ReceiptHandle::parse(text_of("ReceiptHandle", receipt)?),
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.require("QueueName")?;
            let receipt = params.require("ReceiptHandle")?;
            let visibility = params.require("VisibilityTimeout")?;
            Ok(Command::ChangeMessageVisibility {
                queue: queue_ref("QueueName", queue)?,
                receipt: ReceiptHandle::parse(text_of("ReceiptHandle", receipt)?),
                visibility: seconds("VisibilityTimeout", visibility, 0..=43_200)?,
            })
        }
        other => Err(Failure::new(
            ErrorCode::InvalidRequest,
            format!("unknown Command {other:?}"),
        )),
    }
}

/// The members of a command line. A member whose value is `null` counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn require(&self, name: &str) -> Result<&'a Value, Failure> {
        self.optional(name)
            .ok_or_else(|| Failure::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }

    fn optional(&self, name: &str) -> Option<&'a Value> {
        match self.0.get(name) {
            None | Some(Value::Null) => None,
            Some(value) => Some(value),
        }
    }
}

/// Reads an attributes object. Absent means no attribute was given.
fn attributes_of(value: Option<&Value>) -> Result<AttributeRequest, Failure> {
    let Some(value) = value else {
        return Ok(AttributeRequest::default());
    };
    let Value::Object(members) = value else {
        return Err(Failure::new(
            ErrorCode::InvalidParameterValue,
            "Attributes must be an object",
        ));
    };
    let mut attributes = AttributeRequest::default();
    for (name, value) in members {
        if value.is_null() {
            continue;
        }
        match name.as_str() {
            "VisibilityTimeout" => {
                attributes.visibility_timeout = Some(seconds(name, value, 0..=43_200)?)
            }
            "DelaySeconds" => attributes.delay_seconds = Some(seconds(name, value, 0..=900)?),
            "RetentionSeconds" => {
                attributes.retention_seconds = Some(seconds(name, value, 60..=1_209_600)?)
            }
            "Fifo" => attributes.fifo = Some(flag(name, value)?),
            "ContentBasedDeduplication" => {
                attributes.content_based_deduplication = Some(flag(name, value)?)
            }
            "DeadLetterQueue" => attributes.dead_letter_queue = Some(queue_ref(name, value)?),
            "MaxReceiveCount" => {
                attributes.max_receive_count = Some(integer(name, value, 1..=1000)?)
            }
            _ => {
                return Err(Failure::new(
                    ErrorCode::InvalidParameterValue,
                    format!("{name} is not an attribute"),
                ))
            }
        }
    }
    Ok(attributes)
}

impl AttributeRequest {
    /// The dead-letter pair of section 4: both members are given or neither is. Returns None when
    /// neither is given.
    pub fn dead_letter_request(&self) -> Result<Option<DeadLetterRequest>, Failure> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (None, None) => Ok(None),
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterRequest {
                queue: queue.clone(),
                max_receive_count,
            })),
            _ => Err(Failure::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

fn text_of<'a>(name: &str, value: &'a Value) -> Result<&'a str, Failure> {
    value.as_str().ok_or_else(|| {
        Failure::new(
            ErrorCode::InvalidParameterValue,
            format!("{name} must be a string"),
        )
    })
}

fn queue_ref(name: &str, value: &Value) -> Result<QueueRef, Failure> {
    Ok(QueueRef::new(text_of(name, value)?.to_owned()))
}

fn flag(name: &str, value: &Value) -> Result<bool, Failure> {
    value.as_bool().ok_or_else(|| {
        Failure::new(
            ErrorCode::InvalidParameterValue,
            format!("{name} must be true or false"),
        )
    })
}

/// Reads an integer from a JSON number written without fraction or exponent, in `range`.
fn integer<T: TryFrom<i64>>(
    name: &str,
    value: &Value,
    range: RangeInclusive<i64>,
) -> Result<T, Failure> {
    let invalid = || {
        Failure::new(
            ErrorCode::InvalidParameterValue,
            format!(
                "{name} must be an integer from {} to {}",
                range.start(),
                range.end()
            ),
        )
    };
    let number = value
        .as_i64()
        .filter(|number| range.contains(number))
        .ok_or_else(invalid)?;
    T::try_from(number).map_err(|_| invalid())
}

fn seconds(name: &str, value: &Value, range: RangeInclusive<i64>) -> Result<Seconds, Failure> {
    integer::<u32>(name, value, range).map(Seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(line: &str) -> ErrorCode {
        parse_line(line.as_bytes())
            .expect_err("the line should be refused")
            .code
    }

    #[test]
    fn refused_lines_report_the_code_of_section_2() {
        let cases: [(&[u8], ErrorCode); 13] = [
            (b"\xff", ErrorCode::InvalidRequest),
            (b"", ErrorCode::InvalidRequest),
            (b"[]", ErrorCode::InvalidRequest),
            (br#"{"Command": 5}"#, ErrorCode::InvalidRequest),
            (br#"{"Command": null}"#, ErrorCode::InvalidRequest),
            (br#"{"Command": "Nope"}"#, ErrorCode::InvalidRequest),
            (br#"{"Command": "CreateQueue"}"#, ErrorCode::MissingParameter),
            (br#"{"Command": "CreateQueue", "QueueName": null}"#, ErrorCode::MissingParameter),
            (br#"{"Command": "CreateQueue", "QueueName": "a b"}"#, ErrorCode::QueueNameInvalid),
            (
                br#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"VisibilityTimeout": 30.0}}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                br#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"Nope": 1}}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                br#"{"Command": "SetQueueAttributes", "QueueName": "ok"}"#,
                ErrorCode::MissingParameter,
            ),
            (
                br#"{"Command": "SendMessage", "QueueName": "ok", "Body": ""}"#,
                ErrorCode::InvalidParameterValue,
            ),
        ];
        for (line, expected) in cases {
            let failure = parse_line(line).expect_err("the line should be refused");
            assert_eq!(
                failure.code,
                expected,
                "line {:?}",
                String::from_utf8_lossy(line)
            );
        }
    }

    #[test]
    fn queue_name_is_checked_before_other_attributes() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": {"Nope": 1}}"#;
        assert_eq!(code_of(line), ErrorCode::QueueNameInvalid);
    }

    #[test]
    fn missing_parameter_is_reported_before_a_bad_value() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": 7}"#;
        assert_eq!(code_of(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn out_of_range_integers_are_refused_at_both_ends() {
        let cases = [
            r#"{"Command": "Tick", "Seconds": 1209601}"#,
            r#"{"Command": "Tick", "Seconds": -1}"#,
            r#"{"Command": "Tick", "Seconds": "3"}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}"#,
        ];
        for line in cases {
            assert_eq!(
                code_of(line),
                ErrorCode::InvalidParameterValue,
                "line {line}"
            );
        }
    }

    #[test]
    fn null_members_count_as_absent_and_unknown_members_are_ignored() {
        let line =
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": null, "Extra": [1]}"#;
        assert_eq!(
            parse_line(line.as_bytes()),
            Ok(Command::ReceiveMessage {
                queue: QueueRef::new("q".to_owned()),
                max_messages: 1,
                visibility: None,
            })
        );
    }

    #[test]
    fn malformed_receipt_handle_becomes_none_for_the_broker_to_refuse() {
        let line = r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-01"}"#;
        assert_eq!(
            parse_line(line.as_bytes()),
            Ok(Command::DeleteMessage {
                queue: QueueRef::new("q".to_owned()),
                receipt: None,
            })
        );
    }

    #[test]
    fn attributes_null_members_are_absent() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": null, "DelaySeconds": 5}}"#;
        assert_eq!(
            parse_line(line.as_bytes()),
            Ok(Command::CreateQueue {
                name: QueueName::try_from("q").expect("q is a valid name"),
                attributes: AttributeRequest {
                    delay_seconds: Some(Seconds(5)),
                    ..AttributeRequest::default()
                },
            })
        );
    }
}
