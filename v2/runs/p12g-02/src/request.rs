//! Turns one input line into a typed [`Request`], applying the shape, required-parameter
//! and parameter-value checks of the spec in that order.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{ApiError, ErrorCode};
use crate::types::{DeduplicationId, GroupId, QueueName, Seconds};

/// Attributes as given by `CreateQueue` and `SetQueueAttributes`. Every member is
/// individually validated; the rules that relate members to each other belong to the commands.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributeUpdate {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

/// A complete dead-letter configuration as requested, before the queue name is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadLetterRequest<'a> {
    pub queue: &'a str,
    pub max_receive_count: u32,
}

impl AttributeUpdate {
    /// The dead-letter configuration, which must be given completely or not at all.
    pub fn dead_letter(&self) -> Result<Option<DeadLetterRequest<'_>>, ApiError> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max_receive_count)) => Ok(Some(DeadLetterRequest {
                queue,
                max_receive_count,
            })),
            (None, None) => Ok(None),
            _ => Err(ApiError::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    CreateQueue {
        name: QueueName,
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
        seconds: Seconds,
    },
    SendMessage {
        queue: String,
        body: String,
        delay: Option<Seconds>,
        group: Option<GroupId>,
        deduplication: Option<DeduplicationId>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: String,
        receipt: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt: String,
        visibility_timeout: Seconds,
    },
}

fn invalid_request(message: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::InvalidRequest, message)
}

fn missing(name: &str) -> ApiError {
    ApiError::new(
        ErrorCode::MissingParameter,
        format!("parameter {name} is required"),
    )
}

fn invalid_value(name: &str, requirement: &str) -> ApiError {
    ApiError::new(
        ErrorCode::InvalidParameterValue,
        format!("parameter {name} {requirement}"),
    )
}

/// A JSON object whose `null` members count as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require(&self, names: &[&str]) -> Result<(), ApiError> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(missing(name)),
            None => Ok(()),
        }
    }

    fn string(&self, name: &str) -> Result<Option<&'a str>, ApiError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(invalid_value(name, "must be a string")),
        }
    }

    fn required_string(&self, name: &str) -> Result<&'a str, ApiError> {
        self.string(name)?.ok_or_else(|| missing(name))
    }

    fn boolean(&self, name: &str) -> Result<Option<bool>, ApiError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(invalid_value(name, "must be a boolean")),
        }
    }

    fn integer(&self, name: &str, range: RangeInclusive<u32>) -> Result<Option<u32>, ApiError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        // `as_i64` is `None` for numbers written with a fraction or exponent.
        value
            .as_i64()
            .and_then(|number| u32::try_from(number).ok())
            .filter(|number| range.contains(number))
            .map(Some)
            .ok_or_else(|| {
                invalid_value(
                    name,
                    &format!(
                        "must be an integer from {} to {}",
                        range.start(),
                        range.end()
                    ),
                )
            })
    }

    fn required_integer(&self, name: &str, range: RangeInclusive<u32>) -> Result<u32, ApiError> {
        self.integer(name, range)?.ok_or_else(|| missing(name))
    }

    fn seconds(&self, name: &str, range: RangeInclusive<u32>) -> Result<Option<Seconds>, ApiError> {
        Ok(self.integer(name, range)?.map(Seconds))
    }
}

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

fn parse_attributes(params: &Params<'_>) -> Result<AttributeUpdate, ApiError> {
    let object = match params.get("Attributes") {
        None => return Ok(AttributeUpdate::default()),
        Some(Value::Object(object)) => Params(object),
        Some(_) => return Err(invalid_value("Attributes", "must be an object")),
    };
    if let Some(name) = object
        .0
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, _)| name.as_str())
        .find(|name| !ATTRIBUTE_NAMES.contains(name))
    {
        return Err(invalid_value(
            "Attributes",
            &format!("has unknown attribute {name}"),
        ));
    }
    Ok(AttributeUpdate {
        visibility_timeout: object.seconds("VisibilityTimeout", 0..=43200)?,
        delay: object.seconds("DelaySeconds", 0..=900)?,
        retention: object.seconds("RetentionSeconds", 60..=1_209_600)?,
        fifo: object.boolean("Fifo")?,
        content_based_deduplication: object.boolean("ContentBasedDeduplication")?,
        dead_letter_queue: object.string("DeadLetterQueue")?.map(str::to_owned),
        max_receive_count: object.integer("MaxReceiveCount", 1..=1000)?,
    })
}

fn queue_param(params: &Params<'_>) -> Result<String, ApiError> {
    params.required_string("QueueName").map(str::to_owned)
}

/// Parses one line, without its line terminator.
pub fn parse_line(line: &[u8]) -> Result<Request, ApiError> {
    let text = std::str::from_utf8(line)
        .map_err(|error| invalid_request(format!("line is not valid UTF-8: {error}")))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| invalid_request(format!("line is not valid JSON: {error}")))?;
    let Value::Object(object) = value else {
        return Err(invalid_request("line is not a JSON object"));
    };
    let command = match object.get("Command") {
        Some(Value::String(command)) => command.as_str(),
        _ => return Err(invalid_request("Command must be a string")),
    };
    let params = Params(&object);

    match command {
        "CreateQueue" => {
            params.require(&["QueueName"])?;
            let name = QueueName::new(params.required_string("QueueName")?)
                .map_err(|error| ApiError::new(ErrorCode::QueueNameInvalid, error.to_string()))?;
            let attributes = parse_attributes(&params)?;
            Ok(Request::CreateQueue { name, attributes })
        }
        "SetQueueAttributes" => {
            params.require(&["QueueName", "Attributes"])?;
            let queue = queue_param(&params)?;
            let attributes = parse_attributes(&params)?;
            Ok(Request::SetQueueAttributes { queue, attributes })
        }
        "GetQueueAttributes" => {
            params.require(&["QueueName"])?;
            Ok(Request::GetQueueAttributes {
                queue: queue_param(&params)?,
            })
        }
        "ListQueues" => Ok(Request::ListQueues {
            prefix: params.string("Prefix")?.map(str::to_owned),
        }),
        "DeleteQueue" => {
            params.require(&["QueueName"])?;
            Ok(Request::DeleteQueue {
                queue: queue_param(&params)?,
            })
        }
        "PurgeQueue" => {
            params.require(&["QueueName"])?;
            Ok(Request::PurgeQueue {
                queue: queue_param(&params)?,
            })
        }
        "Tick" => {
            params.require(&["Seconds"])?;
            Ok(Request::Tick {
                seconds: Seconds(params.required_integer("Seconds", 0..=1_209_600)?),
            })
        }
        "SendMessage" => {
            params.require(&["QueueName", "Body"])?;
            let queue = queue_param(&params)?;
            let body = params.required_string("Body")?;
            if !(1..=1024).contains(&body.len()) {
                return Err(invalid_value("Body", "must be 1 to 1024 bytes"));
            }
            let delay = params.seconds("DelaySeconds", 0..=900)?;
            let group = params
                .string("GroupId")?
                .map(|group| GroupId::new(group.to_owned()))
                .transpose()
                .map_err(|error| invalid_value("GroupId", &error.to_string()))?;
            let deduplication = params
                .string("DeduplicationId")?
                .map(|id| DeduplicationId::new(id.to_owned()))
                .transpose()
                .map_err(|error| invalid_value("DeduplicationId", &error.to_string()))?;
            Ok(Request::SendMessage {
                queue,
                body: body.to_owned(),
                delay,
                group,
                deduplication,
            })
        }
        "ReceiveMessage" => {
            params.require(&["QueueName"])?;
            Ok(Request::ReceiveMessage {
                queue: queue_param(&params)?,
                max_messages: params.integer("MaxMessages", 1..=10)?.unwrap_or(1),
                visibility_timeout: params.seconds("VisibilityTimeout", 0..=43200)?,
            })
        }
        "DeleteMessage" => {
            params.require(&["QueueName", "ReceiptHandle"])?;
            Ok(Request::DeleteMessage {
                queue: queue_param(&params)?,
                receipt: params.required_string("ReceiptHandle")?.to_owned(),
            })
        }
        "ChangeMessageVisibility" => {
            params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
            Ok(Request::ChangeMessageVisibility {
                queue: queue_param(&params)?,
                receipt: params.required_string("ReceiptHandle")?.to_owned(),
                visibility_timeout: Seconds(
                    params.required_integer("VisibilityTimeout", 0..=43200)?,
                ),
            })
        }
        other => Err(invalid_request(format!("unknown command {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_code(line: &str) -> ErrorCode {
        match parse_line(line.as_bytes()) {
            Err(error) => error.code,
            Ok(request) => panic!("expected an error for {line}, got {request:?}"),
        }
    }

    #[test]
    fn malformed_lines_are_invalid_requests() {
        let cases = [
            "",
            "   ",
            "[]",
            "\"CreateQueue\"",
            "{}",
            "{\"Command\": 1}",
            "{\"Command\": null}",
            "{\"Command\": \"Nope\"}",
            "{\"command\": \"Tick\", \"Seconds\": 1}",
            "{\"Command\": \"Tick\"} x",
            "{\"Command\": \"Tick\"",
        ];
        for line in cases {
            assert_eq!(error_code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
        assert_eq!(
            parse_line(b"{\"Command\": \"Tick\", \"Seconds\": 1}\xff")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameters_win_over_bad_values() {
        let line = r#"{"Command": "SendMessage", "QueueName": 5}"#;
        assert_eq!(error_code(line), ErrorCode::MissingParameter);
        let null_body = r#"{"Command": "SendMessage", "QueueName": "q", "Body": null}"#;
        assert_eq!(error_code(null_body), ErrorCode::MissingParameter);
    }

    #[test]
    fn integers_must_be_written_as_integers_in_range() {
        let cases = [
            ("30", true),
            ("0", true),
            ("-0", true),
            ("1209600", true),
            ("30.0", false),
            ("3e1", false),
            ("\"30\"", false),
            ("-1", false),
            ("1209601", false),
            ("true", false),
            ("99999999999999999999", false),
        ];
        for (seconds, valid) in cases {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(
                parse_line(line.as_bytes()).is_ok(),
                valid,
                "Seconds {seconds}"
            );
        }
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(error_code(line), ErrorCode::QueueNameInvalid);
        let wrong_type = r#"{"Command": "CreateQueue", "QueueName": 3, "Attributes": 3}"#;
        assert_eq!(error_code(wrong_type), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn attributes_are_validated_one_by_one() {
        let cases = [
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": []}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Nope": 1}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 901}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 59}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": "true"}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxReceiveCount": 0}}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": 1}}"#,
        ];
        for line in cases {
            assert_eq!(
                error_code(line),
                ErrorCode::InvalidParameterValue,
                "line {line}"
            );
        }
    }

    #[test]
    fn null_attributes_and_unknown_members_are_ignored() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Extra": 1,
            "Attributes": {"DelaySeconds": null, "VisibilityTimeout": 5}}"#;
        assert_eq!(
            parse_line(line.as_bytes()),
            Ok(Request::SetQueueAttributes {
                queue: "q".to_owned(),
                attributes: AttributeUpdate {
                    visibility_timeout: Some(Seconds(5)),
                    ..AttributeUpdate::default()
                },
            })
        );
    }

    #[test]
    fn send_message_parameters_are_parsed() {
        let line = r#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "hé",
            "GroupId": "g-1", "DeduplicationId": "d_1"}"#;
        assert_eq!(
            parse_line(line.as_bytes()),
            Ok(Request::SendMessage {
                queue: "q.fifo".to_owned(),
                body: "h\u{e9}".to_owned(),
                delay: None,
                group: Some(GroupId::new("g-1".to_owned()).unwrap()),
                deduplication: Some(DeduplicationId::new("d_1".to_owned()).unwrap()),
            })
        );
    }

    #[test]
    fn body_length_is_counted_in_bytes() {
        let ok = "\u{e9}".repeat(512);
        let too_long = format!("{ok}a");
        let line = |body: &str| {
            format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{body}"}}"#)
        };
        assert_eq!(
            parse_line(line(&ok).as_bytes()),
            Ok(Request::SendMessage {
                queue: "q".to_owned(),
                body: ok.clone(),
                delay: None,
                group: None,
                deduplication: None,
            })
        );
        assert_eq!(
            error_code(&line(&too_long)),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(error_code(&line("")), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn dead_letter_configuration_must_be_complete() {
        let both = AttributeUpdate {
            dead_letter_queue: Some("dlq".to_owned()),
            max_receive_count: Some(3),
            ..AttributeUpdate::default()
        };
        assert_eq!(
            both.dead_letter(),
            Ok(Some(DeadLetterRequest {
                queue: "dlq",
                max_receive_count: 3
            }))
        );
        assert_eq!(AttributeUpdate::default().dead_letter(), Ok(None));
        let half = AttributeUpdate {
            max_receive_count: Some(3),
            ..AttributeUpdate::default()
        };
        assert_eq!(
            half.dead_letter().unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
    }
}
