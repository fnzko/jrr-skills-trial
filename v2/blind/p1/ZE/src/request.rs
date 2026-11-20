//! Turns one input line into a typed [`Command`], applying the shape,
//! required-parameter and parameter-value checks of the error order.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::attributes::AttributeUpdate;
use crate::error::{Error, ErrorCode};
use crate::queue::SendRequest;

/// Suffix that marks the name of a FIFO queue.
pub const FIFO_SUFFIX: &str = ".fifo";

/// A parsed command with every parameter checked on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: String,
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
        seconds: u64,
    },
    SendMessage {
        queue: String,
        request: SendRequest,
    },
    ReceiveMessage {
        queue: String,
        max_messages: usize,
        visibility_timeout: Option<u64>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: u64,
    },
}

impl Command {
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let text = std::str::from_utf8(line)
            .map_err(|e| Error::invalid_request(format!("line is not valid UTF-8: {e}")))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|e| Error::invalid_request(format!("line is not valid JSON: {e}")))?;
        let Value::Object(members) = value else {
            return Err(Error::invalid_request("line is not a JSON object"));
        };
        let mut params = Params(members);
        let Some(Value::String(name)) = params.0.remove("Command") else {
            return Err(Error::invalid_request("Command must be a string"));
        };
        match name.as_str() {
            "CreateQueue" => {
                params.require(&["QueueName"])?;
                let name = params.required_string("QueueName")?;
                if !is_valid_queue_name(&name) {
                    return Err(Error::new(
                        ErrorCode::QueueNameInvalid,
                        format!("{name:?} is not a valid queue name"),
                    ));
                }
                let attributes = params.attributes()?.unwrap_or_default();
                Ok(Self::CreateQueue { name, attributes })
            }
            "SetQueueAttributes" => {
                params.require(&["QueueName", "Attributes"])?;
                let queue = params.required_string("QueueName")?;
                let attributes = params
                    .attributes()?
                    .ok_or_else(|| Error::missing("Attributes"))?;
                Ok(Self::SetQueueAttributes { queue, attributes })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: params.queue()?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params.string("Prefix")?,
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: params.queue()?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: params.queue()?,
            }),
            "Tick" => {
                params.require(&["Seconds"])?;
                Ok(Self::Tick {
                    seconds: params.required_integer("Seconds", 0..=1_209_600)?,
                })
            }
            "SendMessage" => {
                params.require(&["QueueName", "Body"])?;
                let queue = params.required_string("QueueName")?;
                let body = params.required_string("Body")?;
                if !(1..=1024).contains(&body.len()) {
                    return Err(Error::invalid_value("Body must be 1 to 1024 bytes"));
                }
                let request = SendRequest {
                    body,
                    delay_seconds: params.integer("DelaySeconds", 0..=900)?,
                    group_id: params.identifier("GroupId")?,
                    deduplication_id: params.identifier("DeduplicationId")?,
                };
                Ok(Self::SendMessage { queue, request })
            }
            "ReceiveMessage" => {
                params.require(&["QueueName"])?;
                let queue = params.required_string("QueueName")?;
                let max_messages = params.integer("MaxMessages", 1..=10)?.map_or(1, |n| {
                    usize::try_from(n).expect("MaxMessages is at most 10")
                });
                let visibility_timeout = params.integer("VisibilityTimeout", 0..=43_200)?;
                Ok(Self::ReceiveMessage {
                    queue,
                    max_messages,
                    visibility_timeout,
                })
            }
            "DeleteMessage" => {
                params.require(&["QueueName", "ReceiptHandle"])?;
                let queue = params.required_string("QueueName")?;
                let receipt_handle = params.required_string("ReceiptHandle")?;
                Ok(Self::DeleteMessage {
                    queue,
                    receipt_handle,
                })
            }
            "ChangeMessageVisibility" => {
                params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                let queue = params.required_string("QueueName")?;
                let receipt_handle = params.required_string("ReceiptHandle")?;
                let visibility_timeout =
                    params.required_integer("VisibilityTimeout", 0..=43_200)?;
                Ok(Self::ChangeMessageVisibility {
                    queue,
                    receipt_handle,
                    visibility_timeout,
                })
            }
            _ => Err(Error::invalid_request(format!("unknown command {name:?}"))),
        }
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// 1 to 80 name characters, or 1 to 75 of them followed by `.fifo`.
fn is_valid_queue_name(name: &str) -> bool {
    let stem = name.strip_suffix(FIFO_SUFFIX).unwrap_or(name);
    !stem.is_empty() && name.len() <= 80 && stem.bytes().all(is_name_byte)
}

/// The members of a command or of an `Attributes` object. A `null` member
/// counts as absent.
struct Params(Map<String, Value>);

impl Params {
    fn take(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name).filter(|value| !value.is_null())
    }

    /// Fails with the first of `names` that is absent, before any value is looked at.
    fn require(&self, names: &[&str]) -> Result<(), Error> {
        match names
            .iter()
            .find(|name| self.0.get(**name).is_none_or(Value::is_null))
        {
            Some(name) => Err(Error::missing(name)),
            None => Ok(()),
        }
    }

    fn string(&mut self, name: &str) -> Result<Option<String>, Error> {
        match self.take(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(Error::invalid_value(format!("{name} must be a string"))),
        }
    }

    fn required_string(&mut self, name: &str) -> Result<String, Error> {
        self.string(name)?.ok_or_else(|| Error::missing(name))
    }

    fn queue(mut self) -> Result<String, Error> {
        self.require(&["QueueName"])?;
        self.required_string("QueueName")
    }

    /// A group or deduplication id: 1 to 128 name characters.
    fn identifier(&mut self, name: &str) -> Result<Option<String>, Error> {
        let Some(text) = self.string(name)? else {
            return Ok(None);
        };
        if (1..=128).contains(&text.len()) && text.bytes().all(is_name_byte) {
            Ok(Some(text))
        } else {
            Err(Error::invalid_value(format!(
                "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
            )))
        }
    }

    fn boolean(&mut self, name: &str) -> Result<Option<bool>, Error> {
        match self.take(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(flag)),
            Some(_) => Err(Error::invalid_value(format!("{name} must be a boolean"))),
        }
    }

    fn integer(&mut self, name: &str, range: RangeInclusive<u64>) -> Result<Option<u64>, Error> {
        let Some(value) = self.take(name) else {
            return Ok(None);
        };
        match value.as_u64() {
            Some(n) if range.contains(&n) => Ok(Some(n)),
            _ => Err(Error::invalid_value(format!(
                "{name} must be an integer from {} to {}",
                range.start(),
                range.end()
            ))),
        }
    }

    fn required_integer(&mut self, name: &str, range: RangeInclusive<u64>) -> Result<u64, Error> {
        self.integer(name, range)?
            .ok_or_else(|| Error::missing(name))
    }

    /// The `Attributes` member, if present: an object of known attributes.
    fn attributes(&mut self) -> Result<Option<AttributeUpdate>, Error> {
        let Some(value) = self.take("Attributes") else {
            return Ok(None);
        };
        let Value::Object(members) = value else {
            return Err(Error::invalid_value("Attributes must be an object"));
        };
        let mut attributes = Params(members);
        let update = AttributeUpdate {
            visibility_timeout: attributes.integer("VisibilityTimeout", 0..=43_200)?,
            delay_seconds: attributes.integer("DelaySeconds", 0..=900)?,
            retention_seconds: attributes.integer("RetentionSeconds", 60..=1_209_600)?,
            fifo: attributes.boolean("Fifo")?,
            content_based_deduplication: attributes.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: attributes.string("DeadLetterQueue")?,
            max_receive_count: attributes.integer("MaxReceiveCount", 1..=1000)?,
        };
        match attributes.0.iter().find(|(_, value)| !value.is_null()) {
            Some((name, _)) => Err(Error::invalid_value(format!("unknown attribute {name}"))),
            None => Ok(Some(update)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn code(line: &str) -> ErrorCode {
        Command::parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn parses_tick() {
        let command = Command::parse(br#"{"Command":"Tick","Seconds":5,"Extra":[1]}"#);
        assert_eq!(command, Ok(Command::Tick { seconds: 5 }));
    }

    #[test]
    fn rejects_lines_that_are_not_commands() {
        let cases = [
            "",
            "not json",
            "[]",
            "\"Tick\"",
            "{}",
            r#"{"Command":3}"#,
            r#"{"Command":null}"#,
            r#"{"command":"Tick","Seconds":1}"#,
            r#"{"Command":"Nope"}"#,
            r#"{"Command":"Tick","Seconds":1} x"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
        assert_eq!(
            Command::parse(b"{\"Command\":\"Tick\",\"Seconds\":1,\"X\":\"\xff\"}")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn null_counts_as_absent() {
        assert_eq!(
            code(r#"{"Command":"Tick","Seconds":null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        for value in [
            "1.0",
            "1e0",
            "\"1\"",
            "true",
            "-1",
            "1209601",
            "18446744073709551616",
        ] {
            let line = format!(r#"{{"Command":"Tick","Seconds":{value}}}"#);
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "value {value}"
            );
        }
        for value in ["0", "1209600"] {
            let line = format!(r#"{{"Command":"Tick","Seconds":{value}}}"#);
            assert!(matches!(
                Command::parse(line.as_bytes()),
                Ok(Command::Tick { .. })
            ));
        }
    }
}
