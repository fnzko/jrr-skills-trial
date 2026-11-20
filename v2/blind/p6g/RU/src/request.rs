use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::config::AttributeSet;
use crate::error::{Error, ErrorCode};

/// A fully validated command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: String,
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
        seconds: u32,
    },
    SendMessage {
        queue: String,
        body: String,
        delay_seconds: Option<u32>,
        group_id: Option<String>,
        deduplication_id: Option<String>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: usize,
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

/// Parses one input line into a command, applying the checks of spec
/// sections 1 to 3 in their order.
pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let invalid = |why: &str| Error::new(ErrorCode::InvalidRequest, why);
    let text = std::str::from_utf8(line).map_err(|_| invalid("line is not valid UTF-8"))?;
    let value: Value =
        serde_json::from_str(text).map_err(|e| invalid(&format!("line is not JSON: {e}")))?;
    let Value::Object(mut params) = value else {
        return Err(invalid("line is not a JSON object"));
    };
    let Some(Value::String(name)) = params.remove("Command") else {
        return Err(invalid("Command is missing or not a string"));
    };
    let mut params = Params(params);
    match name.as_str() {
        "CreateQueue" => params.create_queue(),
        "SetQueueAttributes" => params.set_queue_attributes(),
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: params.queue_name()?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.optional_string("Prefix")?,
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: params.queue_name()?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: params.queue_name()?,
        }),
        "Tick" => params.tick(),
        "SendMessage" => params.send_message(),
        "ReceiveMessage" => params.receive_message(),
        "DeleteMessage" => params.delete_message(),
        "ChangeMessageVisibility" => params.change_message_visibility(),
        other => Err(invalid(&format!("unknown command {other:?}"))),
    }
}

/// The members of a request. A `null` member counts as absent, and taking a
/// member removes it.
struct Params(Map<String, Value>);

impl Params {
    fn take(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name).filter(|value| !value.is_null())
    }

    fn require(&mut self, name: &str) -> Result<Value, Error> {
        self.take(name).ok_or_else(|| Error::missing(name))
    }

    fn queue_name(&mut self) -> Result<String, Error> {
        let value = self.require("QueueName")?;
        string("QueueName", value)
    }

    fn optional_string(&mut self, name: &str) -> Result<Option<String>, Error> {
        self.take(name).map(|value| string(name, value)).transpose()
    }

    fn optional_integer(
        &mut self,
        name: &str,
        range: RangeInclusive<u32>,
    ) -> Result<Option<u32>, Error> {
        self.take(name)
            .map(|value| integer(name, &value, range))
            .transpose()
    }

    fn create_queue(&mut self) -> Result<Command, Error> {
        let name = self.require("QueueName")?;
        let name = string("QueueName", name)?;
        if !is_valid_queue_name(&name) {
            return Err(Error::new(
                ErrorCode::QueueNameInvalid,
                "queue names are 1 to 80 characters of A-Z a-z 0-9 _ -, optionally followed by .fifo",
            ));
        }
        let attributes = match self.take("Attributes") {
            Some(value) => attribute_set(value)?,
            None => AttributeSet::default(),
        };
        Ok(Command::CreateQueue { name, attributes })
    }

    fn set_queue_attributes(&mut self) -> Result<Command, Error> {
        let queue = self.require("QueueName")?;
        let attributes = self.require("Attributes")?;
        Ok(Command::SetQueueAttributes {
            queue: string("QueueName", queue)?,
            attributes: attribute_set(attributes)?,
        })
    }

    fn tick(&mut self) -> Result<Command, Error> {
        let seconds = self.require("Seconds")?;
        Ok(Command::Tick {
            seconds: integer("Seconds", &seconds, 0..=1_209_600)?,
        })
    }

    fn send_message(&mut self) -> Result<Command, Error> {
        let queue = self.require("QueueName")?;
        let body = self.require("Body")?;
        let queue = string("QueueName", queue)?;
        let body = string("Body", body)?;
        if !(1..=1024).contains(&body.len()) {
            return Err(Error::invalid_value("Body", "must be 1 to 1024 bytes"));
        }
        let delay_seconds = self.optional_integer("DelaySeconds", 0..=900)?;
        let group_id = self.optional_string("GroupId")?;
        let deduplication_id = self.optional_string("DeduplicationId")?;
        for (name, id) in [
            ("GroupId", &group_id),
            ("DeduplicationId", &deduplication_id),
        ] {
            if id.as_deref().is_some_and(|id| !is_valid_id(id)) {
                return Err(Error::invalid_value(
                    name,
                    "must be 1 to 128 characters of A-Z a-z 0-9 _ -",
                ));
            }
        }
        Ok(Command::SendMessage {
            queue,
            body,
            delay_seconds,
            group_id,
            deduplication_id,
        })
    }

    fn receive_message(&mut self) -> Result<Command, Error> {
        let queue = self.queue_name()?;
        let max_messages = self.optional_integer("MaxMessages", 1..=10)?.unwrap_or(1);
        let visibility_timeout = self.optional_integer("VisibilityTimeout", 0..=43_200)?;
        Ok(Command::ReceiveMessage {
            queue,
            max_messages: usize::try_from(max_messages).expect("at most 10 fits in usize"),
            visibility_timeout,
        })
    }

    fn delete_message(&mut self) -> Result<Command, Error> {
        let queue = self.require("QueueName")?;
        let handle = self.require("ReceiptHandle")?;
        Ok(Command::DeleteMessage {
            queue: string("QueueName", queue)?,
            receipt_handle: string("ReceiptHandle", handle)?,
        })
    }

    fn change_message_visibility(&mut self) -> Result<Command, Error> {
        let queue = self.require("QueueName")?;
        let handle = self.require("ReceiptHandle")?;
        let timeout = self.require("VisibilityTimeout")?;
        Ok(Command::ChangeMessageVisibility {
            queue: string("QueueName", queue)?,
            receipt_handle: string("ReceiptHandle", handle)?,
            visibility_timeout: integer("VisibilityTimeout", &timeout, 0..=43_200)?,
        })
    }
}

fn string(name: &str, value: Value) -> Result<String, Error> {
    match value {
        Value::String(text) => Ok(text),
        _ => Err(Error::invalid_value(name, "must be a string")),
    }
}

/// An integer written without fraction or exponent, within `range`. Such a
/// number is what `serde_json` stores as an integer; anything else, including
/// `30.0`, does not convert.
fn integer(name: &str, value: &Value, range: RangeInclusive<u32>) -> Result<u32, Error> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            Error::invalid_value(
                name,
                &format!(
                    "must be an integer from {} to {}",
                    range.start(),
                    range.end()
                ),
            )
        })
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(name, "must be true or false"))
}

fn attribute_set(value: Value) -> Result<AttributeSet, Error> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value("Attributes", "must be an object"));
    };
    let mut set = AttributeSet::default();
    for (name, value) in members.into_iter().filter(|(_, value)| !value.is_null()) {
        let name = name.as_str();
        match name {
            "VisibilityTimeout" => {
                set.visibility_timeout = Some(integer(name, &value, 0..=43_200)?)
            }
            "DelaySeconds" => set.delay_seconds = Some(integer(name, &value, 0..=900)?),
            "RetentionSeconds" => {
                set.retention_seconds = Some(integer(name, &value, 60..=1_209_600)?);
            }
            "Fifo" => set.fifo = Some(boolean(name, &value)?),
            "ContentBasedDeduplication" => {
                set.content_based_deduplication = Some(boolean(name, &value)?);
            }
            "DeadLetterQueue" => set.dead_letter_queue = Some(string(name, value)?),
            "MaxReceiveCount" => set.max_receive_count = Some(integer(name, &value, 1..=1000)?),
            _ => {
                return Err(Error::invalid_value(
                    "Attributes",
                    &format!("has unknown attribute {name:?}"),
                ));
            }
        }
    }
    Ok(set)
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_valid_id(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.chars().all(is_id_char)
}

fn is_valid_queue_name(name: &str) -> bool {
    let base = name.strip_suffix(".fifo").map_or((name, 80), |b| (b, 75));
    (1..=base.1).contains(&base.0.len()) && base.0.chars().all(is_id_char)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn queue_name_syntax() {
        let long = "a".repeat(80);
        let too_long = "a".repeat(81);
        let fifo_long = format!("{}.fifo", "a".repeat(75));
        let fifo_too_long = format!("{}.fifo", "a".repeat(76));
        let cases = [
            ("jobs", true),
            ("A_b-9", true),
            (long.as_str(), true),
            (too_long.as_str(), false),
            ("", false),
            ("a.fifo", true),
            (".fifo", false),
            (fifo_long.as_str(), true),
            (fifo_too_long.as_str(), false),
            ("a.b", false),
            ("a b", false),
            ("caf\u{e9}", false),
            ("a.fifo.fifo", false),
        ];
        for (name, valid) in cases {
            assert_eq!(is_valid_queue_name(name), valid, "{name:?}");
        }
    }

    #[test]
    fn request_shape_errors() {
        for line in [
            "",
            "not json",
            "[]",
            "{}",
            r#"{"Command": 1}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"command": "Tick"}"#,
            r#"{"Command": "Tick"} trailing"#,
        ] {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"\xff\"}").unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameters_win_over_bad_values() {
        let line = r#"{"Command": "SendMessage", "Body": 5}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
        let line = r#"{"Command": "SendMessage", "QueueName": null, "Body": "x"}"#;
        assert_eq!(code(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn integers_must_be_plain_integers_in_range() {
        for seconds in ["30.0", "3e1", "\"30\"", "-1", "1209601", "true"] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{seconds}");
        }
        let line = r#"{"Command": "Tick", "Seconds": 1209600}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::Tick { seconds: 1_209_600 })
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command": "CreateQueue", "QueueName": "ab", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn attributes_are_validated_one_by_one() {
        for attributes in [
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"DelaySeconds": "1"}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"DeadLetterQueue": 1}"#,
            r#"{"Colour": 1}"#,
        ] {
            let line = format!(
                r#"{{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {attributes}}}"#
            );
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "{attributes}"
            );
        }
    }

    #[test]
    fn null_attributes_and_parameters_are_absent() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": null}}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::SetQueueAttributes {
                queue: "q".into(),
                attributes: AttributeSet::default(),
            })
        );
    }

    #[test]
    fn send_message_body_and_id_rules() {
        let big = "x".repeat(1025);
        let bodies = [
            (r#""""#.to_owned(), false),
            (format!("\"{big}\""), false),
            (format!("\"{}\"", "x".repeat(1024)), true),
            // 512 two-byte characters are 1024 bytes
            (format!("\"{}\"", "\u{e9}".repeat(512)), true),
            (format!("\"{}\"", "\u{e9}".repeat(513)), false),
        ];
        for (body, valid) in bodies {
            let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": {body}}}"#);
            assert_eq!(parse(line.as_bytes()).is_ok(), valid, "{}", body.len());
        }
        let line = r#"{"Command": "SendMessage", "QueueName": "q", "Body": "b", "GroupId": "a b"}"#;
        assert_eq!(code(line), ErrorCode::InvalidParameterValue);
    }
}
