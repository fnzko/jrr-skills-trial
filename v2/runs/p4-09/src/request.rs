//! Turns one input line into a typed [`Request`].
//!
//! This covers checks 1 to 3 of the spec's error order: request shape,
//! required parameters, then each parameter value on its own. Everything that
//! needs queue state is left to the service.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{ApiError, ErrorCode};
use crate::queue::DeadLetter;
use crate::types::{QueueName, Tag};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    CreateQueue {
        queue: QueueName,
        attributes: AttributeUpdate,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeUpdate,
    },
    GetQueueAttributes {
        queue: String,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        queue: String,
    },
    PurgeQueue {
        queue: String,
    },
    Tick {
        seconds: u32,
    },
    SendMessage(SendMessage),
    ReceiveMessage {
        queue: String,
        max_messages: u32,
        visibility_timeout: Option<u32>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<Tag>,
    pub deduplication_id: Option<Tag>,
}

/// The attributes named in a request; `None` means the request left it out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeUpdate {
    /// The requested dead-letter configuration, which must name both or
    /// neither of its attributes. The named queue may not exist.
    pub fn dead_letter(&self) -> Result<Option<DeadLetter>, ApiError> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetter {
                queue: queue.clone(),
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(ApiError::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

const VISIBILITY_TIMEOUT: RangeInclusive<u32> = 0..=43_200;
const DELAY_SECONDS: RangeInclusive<u32> = 0..=900;
const RETENTION_SECONDS: RangeInclusive<u32> = 60..=1_209_600;
const MAX_RECEIVE_COUNT: RangeInclusive<u32> = 1..=1000;
const TICK_SECONDS: RangeInclusive<u32> = 0..=1_209_600;
const MAX_MESSAGES: RangeInclusive<u32> = 1..=10;
const BODY_BYTES: RangeInclusive<usize> = 1..=1024;

pub fn parse(line: &[u8]) -> Result<Request, ApiError> {
    let invalid = |message: &str| ApiError::new(ErrorCode::InvalidRequest, message);
    let text = std::str::from_utf8(line).map_err(|_| invalid("line is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text).map_err(|e| invalid(&format!("{e}")))?;
    let Value::Object(members) = value else {
        return Err(invalid("line is not a JSON object"));
    };
    let params = Params(&members);
    let Some(Value::String(command)) = params.get("Command") else {
        return Err(invalid("missing string member Command"));
    };

    // In each arm every required parameter is fetched before any value is
    // checked, so MissingParameter wins over InvalidParameterValue.
    let request = match command.as_str() {
        "CreateQueue" => {
            let queue = params.required("QueueName")?;
            let attributes = params.optional("Attributes");
            // QueueName is checked before every other parameter.
            let queue = queue.string()?;
            let queue = QueueName::parse(queue).ok_or_else(|| {
                ApiError::new(
                    ErrorCode::QueueNameInvalid,
                    format!("{queue:?} is not a valid queue name"),
                )
            })?;
            Request::CreateQueue {
                queue,
                attributes: attributes
                    .map(Field::attributes)
                    .transpose()?
                    .unwrap_or_default(),
            }
        }
        "SetQueueAttributes" => {
            let queue = params.required("QueueName")?;
            let attributes = params.required("Attributes")?;
            Request::SetQueueAttributes {
                queue: queue.owned_string()?,
                attributes: attributes.attributes()?,
            }
        }
        "GetQueueAttributes" => Request::GetQueueAttributes {
            queue: params.required("QueueName")?.owned_string()?,
        },
        "ListQueues" => Request::ListQueues {
            prefix: params
                .optional("Prefix")
                .map(Field::owned_string)
                .transpose()?,
        },
        "DeleteQueue" => Request::DeleteQueue {
            queue: params.required("QueueName")?.owned_string()?,
        },
        "PurgeQueue" => Request::PurgeQueue {
            queue: params.required("QueueName")?.owned_string()?,
        },
        "Tick" => Request::Tick {
            seconds: params.required("Seconds")?.integer(TICK_SECONDS)?,
        },
        "SendMessage" => {
            let queue = params.required("QueueName")?;
            let body = params.required("Body")?;
            let delay_seconds = params.optional("DelaySeconds");
            let group_id = params.optional("GroupId");
            let deduplication_id = params.optional("DeduplicationId");
            Request::SendMessage(SendMessage {
                queue: queue.owned_string()?,
                body: body.body()?,
                delay_seconds: delay_seconds
                    .map(|f| f.integer(DELAY_SECONDS))
                    .transpose()?,
                group_id: group_id.map(Field::tag).transpose()?,
                deduplication_id: deduplication_id.map(Field::tag).transpose()?,
            })
        }
        "ReceiveMessage" => {
            let queue = params.required("QueueName")?;
            let max_messages = params.optional("MaxMessages");
            let visibility_timeout = params.optional("VisibilityTimeout");
            Request::ReceiveMessage {
                queue: queue.owned_string()?,
                max_messages: max_messages
                    .map(|f| f.integer(MAX_MESSAGES))
                    .transpose()?
                    .unwrap_or(1),
                visibility_timeout: visibility_timeout
                    .map(|f| f.integer(VISIBILITY_TIMEOUT))
                    .transpose()?,
            }
        }
        "DeleteMessage" => {
            let queue = params.required("QueueName")?;
            let receipt_handle = params.required("ReceiptHandle")?;
            Request::DeleteMessage {
                queue: queue.owned_string()?,
                receipt_handle: receipt_handle.owned_string()?,
            }
        }
        "ChangeMessageVisibility" => {
            let queue = params.required("QueueName")?;
            let receipt_handle = params.required("ReceiptHandle")?;
            let visibility_timeout = params.required("VisibilityTimeout")?;
            Request::ChangeMessageVisibility {
                queue: queue.owned_string()?,
                receipt_handle: receipt_handle.owned_string()?,
                visibility_timeout: visibility_timeout.integer(VISIBILITY_TIMEOUT)?,
            }
        }
        other => return Err(invalid(&format!("unknown command {other:?}"))),
    };
    Ok(request)
}

/// The members of a request object, where `null` counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn optional(&self, name: &'static str) -> Option<Field<'a>> {
        self.get(name).map(|value| Field { name, value })
    }

    fn required(&self, name: &'static str) -> Result<Field<'a>, ApiError> {
        self.optional(name).ok_or_else(|| {
            ApiError::new(
                ErrorCode::MissingParameter,
                format!("missing required parameter {name}"),
            )
        })
    }
}

/// A present, non-null parameter or attribute, with its name for messages.
#[derive(Clone, Copy)]
struct Field<'a> {
    name: &'a str,
    value: &'a Value,
}

impl<'a> Field<'a> {
    fn invalid(&self, expected: &str) -> ApiError {
        ApiError::invalid_value(format!("{} must be {expected}", self.name))
    }

    fn string(self) -> Result<&'a str, ApiError> {
        self.value.as_str().ok_or_else(|| self.invalid("a string"))
    }

    fn owned_string(self) -> Result<String, ApiError> {
        self.string().map(str::to_owned)
    }

    fn boolean(self) -> Result<bool, ApiError> {
        self.value
            .as_bool()
            .ok_or_else(|| self.invalid("a boolean"))
    }

    /// An integer written without fraction or exponent, within `range`.
    fn integer(self, range: RangeInclusive<u32>) -> Result<u32, ApiError> {
        let expected = || {
            self.invalid(&format!(
                "an integer from {} to {}",
                range.start(),
                range.end()
            ))
        };
        let Value::Number(number) = self.value else {
            return Err(expected());
        };
        // With `arbitrary_precision` the number keeps its source text, which
        // is what decides whether it was written as an integer.
        let text = number.as_str();
        if text.contains(['.', 'e', 'E']) {
            return Err(expected());
        }
        // Digits that do not fit an i64 are out of every range.
        let value: i64 = text.parse().map_err(|_| expected())?;
        u32::try_from(value)
            .ok()
            .filter(|value| range.contains(value))
            .ok_or_else(expected)
    }

    fn tag(self) -> Result<Tag, ApiError> {
        Tag::parse(self.string()?)
            .ok_or_else(|| self.invalid("1 to 128 characters from A-Z a-z 0-9 _ -"))
    }

    fn body(self) -> Result<String, ApiError> {
        let body = self.string()?;
        if !BODY_BYTES.contains(&body.len()) {
            return Err(self.invalid("1 to 1024 bytes long"));
        }
        Ok(body.to_owned())
    }

    fn attributes(self) -> Result<AttributeUpdate, ApiError> {
        let Value::Object(members) = self.value else {
            return Err(self.invalid("an object"));
        };
        let mut update = AttributeUpdate::default();
        for (name, value) in members {
            if value.is_null() {
                continue;
            }
            let field = Field { name, value };
            match name.as_str() {
                "VisibilityTimeout" => {
                    update.visibility_timeout = Some(field.integer(VISIBILITY_TIMEOUT)?)
                }
                "DelaySeconds" => update.delay_seconds = Some(field.integer(DELAY_SECONDS)?),
                "RetentionSeconds" => {
                    update.retention_seconds = Some(field.integer(RETENTION_SECONDS)?)
                }
                "Fifo" => update.fifo = Some(field.boolean()?),
                "ContentBasedDeduplication" => {
                    update.content_based_deduplication = Some(field.boolean()?)
                }
                "DeadLetterQueue" => update.dead_letter_queue = Some(field.owned_string()?),
                "MaxReceiveCount" => {
                    update.max_receive_count = Some(field.integer(MAX_RECEIVE_COUNT)?)
                }
                _ => {
                    return Err(ApiError::invalid_value(format!(
                        "unknown attribute {name:?}"
                    )));
                }
            }
        }
        Ok(update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn rejects_malformed_lines() {
        let cases = [
            "",
            " ",
            "not json",
            "[1]",
            "\"CreateQueue\"",
            "{}",
            r#"{"Command": 5}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "createQueue"}"#,
            r#"{"Command": "Tick", "Seconds": 1} {}"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"\xff\"}").unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn integers_must_be_written_as_integers() {
        let cases = [
            ("0", Ok(0)),
            ("-0", Ok(0)),
            ("1209600", Ok(1_209_600)),
            ("1209601", Err(())),
            ("-1", Err(())),
            ("30.0", Err(())),
            ("3e1", Err(())),
            ("1E400", Err(())),
            ("\"30\"", Err(())),
            ("true", Err(())),
            ("99999999999999999999999", Err(())),
        ];
        for (seconds, want) in cases {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            let got = parse(line.as_bytes()).map_err(|e| e.code);
            let want = want
                .map(|seconds| Request::Tick { seconds })
                .map_err(|()| ErrorCode::InvalidParameterValue);
            assert_eq!(got, want, "Seconds {seconds}");
        }
    }

    #[test]
    fn null_counts_as_absent_and_unknown_members_are_ignored() {
        let request = parse(
            br#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": null, "Extra": [1]}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            Request::ReceiveMessage {
                queue: "q".into(),
                max_messages: 1,
                visibility_timeout: None
            }
        );
        assert_eq!(
            code(r#"{"Command": "PurgeQueue", "QueueName": null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn missing_parameter_wins_over_invalid_value() {
        let line = r#"{"Command": "SendMessage", "QueueName": 7, "DelaySeconds": -1}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let line = r#"{"Command": "CreateQueue", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn create_queue_checks_name_before_attributes() {
        let line =
            r#"{"Command": "CreateQueue", "QueueName": "bad name", "Attributes": {"Nope": 1}}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command": "CreateQueue", "QueueName": 1, "Attributes": {"Nope": 1}}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
        let line = r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"Nope": 1}}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn other_commands_take_any_queue_name_string() {
        let request = parse(br#"{"Command": "GetQueueAttributes", "QueueName": "no such name!"}"#);
        assert_eq!(
            request,
            Ok(Request::GetQueueAttributes {
                queue: "no such name!".into()
            })
        );
    }

    #[test]
    fn parses_attributes() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "q.fifo", "Attributes": {
            "VisibilityTimeout": 5, "DelaySeconds": 900, "RetentionSeconds": 60, "Fifo": true,
            "ContentBasedDeduplication": false, "DeadLetterQueue": "any string",
            "MaxReceiveCount": 3, "MaxReceiveCount2": null}}"#;
        // A null value makes even an unknown attribute absent.
        let want = AttributeUpdate {
            visibility_timeout: Some(5),
            delay_seconds: Some(900),
            retention_seconds: Some(60),
            fifo: Some(true),
            content_based_deduplication: Some(false),
            dead_letter_queue: Some("any string".into()),
            max_receive_count: Some(3),
        };
        let Request::CreateQueue { attributes, .. } = parse(line.as_bytes()).unwrap() else {
            panic!("expected CreateQueue");
        };
        assert_eq!(attributes, want);
    }

    #[test]
    fn rejects_bad_attribute_values() {
        let cases = [
            r#""x""#,
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"ContentBasedDeduplication": 1}"#,
            r#"{"DeadLetterQueue": 1}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"Unknown": 1}"#,
        ];
        for attributes in cases {
            let line = format!(
                r#"{{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {attributes}}}"#
            );
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "attributes {attributes}"
            );
        }
    }

    #[test]
    fn checks_message_parameter_values() {
        let long_body = "x".repeat(1025);
        let multibyte_body = "\u{e9}".repeat(513);
        let cases = [
            r#""Body": """#.to_string(),
            format!(r#""Body": "{long_body}""#),
            format!(r#""Body": "{multibyte_body}""#),
            r#""Body": "b", "GroupId": "a b""#.to_string(),
            r#""Body": "b", "DeduplicationId": """#.to_string(),
            r#""Body": "b", "DelaySeconds": 901"#.to_string(),
        ];
        for members in cases {
            let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", {members}}}"#);
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "members {members}"
            );
        }
        let body = "x".repeat(1024);
        let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{body}"}}"#);
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Request::SendMessage(SendMessage {
                queue: "q".into(),
                body,
                delay_seconds: None,
                group_id: None,
                deduplication_id: None,
            }))
        );
    }

    #[test]
    fn dead_letter_needs_both_attributes() {
        let both = AttributeUpdate {
            dead_letter_queue: Some("dlq".into()),
            max_receive_count: Some(2),
            ..AttributeUpdate::default()
        };
        assert_eq!(
            both.dead_letter(),
            Ok(Some(DeadLetter {
                queue: "dlq".into(),
                max_receive_count: 2
            }))
        );
        assert_eq!(AttributeUpdate::default().dead_letter(), Ok(None));
        let one = AttributeUpdate {
            max_receive_count: Some(2),
            ..AttributeUpdate::default()
        };
        assert_eq!(
            one.dead_letter().unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
    }
}
