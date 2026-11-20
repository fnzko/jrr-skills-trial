//! Turns one input line into a typed [`Command`], applying the shape, required-parameter
//! and parameter-value checks of the spec in that order.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributeSet;
use crate::error::{Error, ErrorCode};
use crate::types::{DeduplicationId, GroupId, QueueName, Seconds};

#[derive(Debug, PartialEq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributeSet,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeSet,
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
        deduplication_id: Option<DeduplicationId>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: Seconds,
    },
}

pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let text = std::str::from_utf8(line)
        .map_err(|_| Error::new(ErrorCode::InvalidRequest, "line is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Error::new(ErrorCode::InvalidRequest, format!("line is not JSON: {e}")))?;
    let Value::Object(members) = value else {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "line is not a JSON object",
        ));
    };
    let fields = Fields(&members);
    let Some(Value::String(name)) = fields.get("Command") else {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "Command must be a string",
        ));
    };
    match name.as_str() {
        "CreateQueue" => create_queue(&fields),
        "SetQueueAttributes" => set_queue_attributes(&fields),
        "GetQueueAttributes" => queue_only(&fields, |queue| Command::GetQueueAttributes { queue }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: fields.optional_string("Prefix")?.map(str::to_owned),
        }),
        "DeleteQueue" => queue_only(&fields, |queue| Command::DeleteQueue { queue }),
        "PurgeQueue" => queue_only(&fields, |queue| Command::PurgeQueue { queue }),
        "Tick" => tick(&fields),
        "SendMessage" => send_message(&fields),
        "ReceiveMessage" => receive_message(&fields),
        "DeleteMessage" => delete_message(&fields),
        "ChangeMessageVisibility" => change_message_visibility(&fields),
        other => Err(Error::new(
            ErrorCode::InvalidRequest,
            format!("unknown command {other:?}"),
        )),
    }
}

/// The members of a JSON object, where `null` counts as absent.
struct Fields<'a>(&'a Map<String, Value>);

impl<'a> Fields<'a> {
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &str) -> Result<&'a Value, Error> {
        self.get(name).ok_or_else(|| Error::missing(name))
    }

    fn optional_string(&self, name: &str) -> Result<Option<&'a str>, Error> {
        self.get(name).map(|value| as_str(name, value)).transpose()
    }

    fn optional_integer(
        &self,
        name: &str,
        range: RangeInclusive<u64>,
    ) -> Result<Option<u64>, Error> {
        self.get(name)
            .map(|value| as_integer(name, value, range))
            .transpose()
    }

    fn optional_bool(&self, name: &str) -> Result<Option<bool>, Error> {
        self.get(name)
            .map(|value| match value {
                Value::Bool(flag) => Ok(*flag),
                _ => Err(Error::invalid_value(name, "must be a boolean")),
            })
            .transpose()
    }
}

fn as_str<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(name, "must be a string"))
}

fn as_integer(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<u64, Error> {
    // `as_u64` is `None` for fractions, exponents and negative numbers alike.
    let number = value
        .as_u64()
        .ok_or_else(|| Error::invalid_value(name, "must be an integer"))?;
    if range.contains(&number) {
        Ok(number)
    } else {
        Err(Error::invalid_value(
            name,
            &format!("must be between {} and {}", range.start(), range.end()),
        ))
    }
}

fn as_seconds(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<Seconds, Error> {
    as_integer(name, value, range).map(Seconds)
}

fn queue_only(fields: &Fields, build: impl FnOnce(String) -> Command) -> Result<Command, Error> {
    let queue = fields.required("QueueName")?;
    Ok(build(as_str("QueueName", queue)?.to_owned()))
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

fn attribute_set(value: &Value) -> Result<AttributeSet, Error> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value("Attributes", "must be an object"));
    };
    if let Some(unknown) = members
        .keys()
        .find(|key| !ATTRIBUTE_NAMES.contains(&key.as_str()))
    {
        return Err(Error::invalid_value(
            "Attributes",
            &format!("has the unknown attribute {unknown:?}"),
        ));
    }
    let fields = Fields(members);
    let seconds = |name, range| {
        fields
            .optional_integer(name, range)
            .map(|value| value.map(Seconds))
    };
    Ok(AttributeSet {
        visibility_timeout: seconds("VisibilityTimeout", 0..=43_200)?,
        delay: seconds("DelaySeconds", 0..=900)?,
        retention: seconds("RetentionSeconds", 60..=1_209_600)?,
        fifo: fields.optional_bool("Fifo")?,
        content_based_deduplication: fields.optional_bool("ContentBasedDeduplication")?,
        dead_letter_queue: fields
            .optional_string("DeadLetterQueue")?
            .map(str::to_owned),
        max_receive_count: fields
            .optional_integer("MaxReceiveCount", 1..=1000)?
            .map(|count| u32::try_from(count).expect("MaxReceiveCount is at most 1000")),
    })
}

fn create_queue(fields: &Fields) -> Result<Command, Error> {
    let name = fields.required("QueueName")?;
    let name = QueueName::parse(as_str("QueueName", name)?)?;
    let attributes = fields.get("Attributes").map(attribute_set).transpose()?;
    Ok(Command::CreateQueue {
        name,
        attributes: attributes.unwrap_or_default(),
    })
}

fn set_queue_attributes(fields: &Fields) -> Result<Command, Error> {
    let queue = fields.required("QueueName")?;
    let attributes = fields.required("Attributes")?;
    Ok(Command::SetQueueAttributes {
        queue: as_str("QueueName", queue)?.to_owned(),
        attributes: attribute_set(attributes)?,
    })
}

fn tick(fields: &Fields) -> Result<Command, Error> {
    let seconds = fields.required("Seconds")?;
    Ok(Command::Tick {
        seconds: as_seconds("Seconds", seconds, 0..=1_209_600)?,
    })
}

fn send_message(fields: &Fields) -> Result<Command, Error> {
    let queue = fields.required("QueueName")?;
    let body = fields.required("Body")?;
    let queue = as_str("QueueName", queue)?.to_owned();
    let body = as_str("Body", body)?;
    if !(1..=1024).contains(&body.len()) {
        return Err(Error::invalid_value("Body", "must be 1 to 1024 bytes"));
    }
    Ok(Command::SendMessage {
        queue,
        body: body.to_owned(),
        delay: fields
            .optional_integer("DelaySeconds", 0..=900)?
            .map(Seconds),
        group: fields
            .optional_string("GroupId")?
            .map(GroupId::parse)
            .transpose()?,
        deduplication_id: fields
            .optional_string("DeduplicationId")?
            .map(DeduplicationId::parse)
            .transpose()?,
    })
}

fn receive_message(fields: &Fields) -> Result<Command, Error> {
    let queue = fields.required("QueueName")?;
    let max_messages = fields.optional_integer("MaxMessages", 1..=10)?.unwrap_or(1);
    Ok(Command::ReceiveMessage {
        queue: as_str("QueueName", queue)?.to_owned(),
        max_messages: usize::try_from(max_messages).expect("MaxMessages is at most 10"),
        visibility_timeout: fields
            .optional_integer("VisibilityTimeout", 0..=43_200)?
            .map(Seconds),
    })
}

fn delete_message(fields: &Fields) -> Result<Command, Error> {
    let queue = fields.required("QueueName")?;
    let handle = fields.required("ReceiptHandle")?;
    Ok(Command::DeleteMessage {
        queue: as_str("QueueName", queue)?.to_owned(),
        receipt_handle: as_str("ReceiptHandle", handle)?.to_owned(),
    })
}

fn change_message_visibility(fields: &Fields) -> Result<Command, Error> {
    let queue = fields.required("QueueName")?;
    let handle = fields.required("ReceiptHandle")?;
    let timeout = fields.required("VisibilityTimeout")?;
    Ok(Command::ChangeMessageVisibility {
        queue: as_str("QueueName", queue)?.to_owned(),
        receipt_handle: as_str("ReceiptHandle", handle)?.to_owned(),
        visibility_timeout: as_seconds("VisibilityTimeout", timeout, 0..=43_200)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn rejects_lines_that_are_not_commands() {
        let lines = [
            "",
            "   ",
            "not json",
            "[]",
            "\"Tick\"",
            "{}",
            r#"{"Command": 5}"#,
            r#"{"Command": null}"#,
            r#"{"command": "Tick", "Seconds": 1}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "Tick", "Seconds": 1} x"#,
        ];
        for line in lines {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"\xff\"}").unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameter_wins_over_a_bad_value() {
        let line = r#"{"Command": "SendMessage", "QueueName": 7}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let null_counts_as_absent = r#"{"Command": "PurgeQueue", "QueueName": null}"#;
        assert_eq!(code(null_counts_as_absent), ErrorCode::MissingParameter);
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        for seconds in ["30.0", "3e1", "\"30\"", "-1", "1209601", "true", "null"] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            let expected = if seconds == "null" {
                ErrorCode::MissingParameter
            } else {
                ErrorCode::InvalidParameterValue
            };
            assert_eq!(code(&line), expected, "{seconds}");
        }
        let line = r#"{"Command": "Tick", "Seconds": 1209600}"#;
        assert_eq!(
            parse(line.as_bytes()).unwrap(),
            Command::Tick {
                seconds: Seconds(1_209_600)
            }
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let bad_name = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(code(bad_name), ErrorCode::QueueNameInvalid);
        let bad_attributes = r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": 3}"#;
        assert_eq!(code(bad_attributes), ErrorCode::InvalidParameterValue);
        let wrong_type = r#"{"Command": "CreateQueue", "QueueName": 3}"#;
        assert_eq!(code(wrong_type), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn attributes_are_range_and_type_checked() {
        let cases = [
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"DelaySeconds": -1}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"ContentBasedDeduplication": 1}"#,
            r#"{"DeadLetterQueue": 1}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"MaxReceiveCount": 1001}"#,
            r#"{"Unknown": 1}"#,
            r#"{"Unknown": null}"#,
        ];
        for attributes in cases {
            let line = format!(
                r#"{{"Command": "CreateQueue", "QueueName": "q", "Attributes": {attributes}}}"#
            );
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "{attributes}"
            );
        }
    }

    #[test]
    fn null_attributes_are_absent() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": null, "Fifo": null}}"#;
        let expected = Command::CreateQueue {
            name: QueueName::parse("q").unwrap(),
            attributes: AttributeSet::default(),
        };
        assert_eq!(parse(line.as_bytes()).unwrap(), expected);
    }

    #[test]
    fn set_queue_attributes_requires_the_attributes_object() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q"}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let empty = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}"#;
        let expected = Command::SetQueueAttributes {
            queue: "q".to_owned(),
            attributes: AttributeSet::default(),
        };
        assert_eq!(parse(empty.as_bytes()).unwrap(), expected);
    }

    #[test]
    fn send_message_limits_body_and_ids() {
        let too_long = "x".repeat(1025);
        let multibyte = "é".repeat(513);
        for body in ["".to_owned(), too_long, multibyte] {
            let line =
                format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{body}"}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue);
        }
        let max = "é".repeat(512);
        let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{max}"}}"#);
        assert!(parse(line.as_bytes()).is_ok());
        let bad_group =
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "b", "GroupId": "a b"}"#;
        assert_eq!(code(bad_group), ErrorCode::InvalidParameterValue);
        let delay =
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 901}"#;
        assert_eq!(code(delay), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn receive_message_defaults_to_one_message() {
        let line = r#"{"Command": "ReceiveMessage", "QueueName": "q", "Extra": true}"#;
        let expected = Command::ReceiveMessage {
            queue: "q".to_owned(),
            max_messages: 1,
            visibility_timeout: None,
        };
        assert_eq!(parse(line.as_bytes()).unwrap(), expected);
        let too_many = r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#;
        assert_eq!(code(too_many), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn change_visibility_requires_all_three_parameters() {
        let line =
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }
}
