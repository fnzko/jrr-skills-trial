//! Turns one input line into a typed command, applying the shape, required-parameter and
//! parameter-value checks of the spec in that order. Checks that need queue state are left to
//! the broker.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::values::{
    DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, QueueName, Ranged,
    RetentionSeconds, TickSeconds, VisibilityTimeout,
};

const QUEUE_NAME: &str = "QueueName";
const ATTRIBUTES: &str = "Attributes";
const RECEIPT_HANDLE: &str = "ReceiptHandle";
const VISIBILITY_TIMEOUT: &str = "VisibilityTimeout";
const DELAY_SECONDS: &str = "DelaySeconds";
const TOKEN_RULE: &str = "must be 1 to 128 characters from A-Z a-z 0-9 _ -";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandName {
    CreateQueue,
    SetQueueAttributes,
    GetQueueAttributes,
    ListQueues,
    DeleteQueue,
    PurgeQueue,
    Tick,
    SendMessage,
    ReceiveMessage,
    DeleteMessage,
    ChangeMessageVisibility,
}

impl CommandName {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "CreateQueue" => Self::CreateQueue,
            "SetQueueAttributes" => Self::SetQueueAttributes,
            "GetQueueAttributes" => Self::GetQueueAttributes,
            "ListQueues" => Self::ListQueues,
            "DeleteQueue" => Self::DeleteQueue,
            "PurgeQueue" => Self::PurgeQueue,
            "Tick" => Self::Tick,
            "SendMessage" => Self::SendMessage,
            "ReceiveMessage" => Self::ReceiveMessage,
            "DeleteMessage" => Self::DeleteMessage,
            "ChangeMessageVisibility" => Self::ChangeMessageVisibility,
            _ => return None,
        })
    }
}

/// Queue attributes as given in a request; absent ones are `None`. The pairing rule of the
/// dead-letter attributes is checked by the command that uses them, at its own place in the
/// order of checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes<'a> {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<&'a str>,
    pub max_receive_count: Option<MaxReceiveCount>,
}

/// A command whose parameters have passed every check that needs no queue state.
#[derive(Debug, PartialEq, Eq)]
pub enum Command<'a> {
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
        body: &'a str,
        delay: Option<DelaySeconds>,
        group: Option<GroupId>,
        deduplication: Option<DeduplicationId>,
    },
    ReceiveMessage {
        queue: &'a str,
        max_messages: Option<MaxMessages>,
        visibility_timeout: Option<VisibilityTimeout>,
    },
    DeleteMessage {
        queue: &'a str,
        receipt: &'a str,
    },
    ChangeMessageVisibility {
        queue: &'a str,
        receipt: &'a str,
        visibility_timeout: VisibilityTimeout,
    },
}

/// One input line that is a JSON object with a string `Command`.
#[derive(Debug)]
pub struct Request {
    command: String,
    members: Map<String, Value>,
}

impl Request {
    /// Checks the request shape: UTF-8, one JSON object, a string `Command`.
    pub fn from_line(line: &[u8]) -> Result<Self, Error> {
        let text = std::str::from_utf8(line)
            .map_err(|e| Error::invalid_request(format!("line is not valid UTF-8: {e}")))?;
        let Value::Object(mut members) = serde_json::from_str(text)
            .map_err(|e| Error::invalid_request(format!("line is not valid JSON: {e}")))?
        else {
            return Err(Error::invalid_request("line is not a JSON object"));
        };
        match members.remove("Command") {
            Some(Value::String(command)) => Ok(Self { command, members }),
            _ => Err(Error::invalid_request("Command is absent or not a string")),
        }
    }

    pub fn command(&self) -> Result<Command<'_>, Error> {
        let name = CommandName::parse(&self.command)
            .ok_or_else(|| Error::invalid_request(format!("unknown command {:?}", self.command)))?;
        let params = Params(&self.members);
        match name {
            CommandName::CreateQueue => params.create_queue(),
            CommandName::SetQueueAttributes => params.set_queue_attributes(),
            CommandName::GetQueueAttributes => Ok(Command::GetQueueAttributes {
                queue: params.queue()?,
            }),
            CommandName::ListQueues => Ok(Command::ListQueues {
                prefix: params.optional_with("Prefix", string)?,
            }),
            CommandName::DeleteQueue => Ok(Command::DeleteQueue {
                queue: params.queue()?,
            }),
            CommandName::PurgeQueue => Ok(Command::PurgeQueue {
                queue: params.queue()?,
            }),
            CommandName::Tick => {
                let seconds = params.required("Seconds")?;
                Ok(Command::Tick {
                    seconds: ranged("Seconds", seconds)?,
                })
            }
            CommandName::SendMessage => params.send_message(),
            CommandName::ReceiveMessage => params.receive_message(),
            CommandName::DeleteMessage => {
                let queue = params.required(QUEUE_NAME)?;
                let receipt = params.required(RECEIPT_HANDLE)?;
                Ok(Command::DeleteMessage {
                    queue: string(QUEUE_NAME, queue)?,
                    receipt: string(RECEIPT_HANDLE, receipt)?,
                })
            }
            CommandName::ChangeMessageVisibility => {
                let queue = params.required(QUEUE_NAME)?;
                let receipt = params.required(RECEIPT_HANDLE)?;
                let timeout = params.required(VISIBILITY_TIMEOUT)?;
                Ok(Command::ChangeMessageVisibility {
                    queue: string(QUEUE_NAME, queue)?,
                    receipt: string(RECEIPT_HANDLE, receipt)?,
                    visibility_timeout: ranged(VISIBILITY_TIMEOUT, timeout)?,
                })
            }
        }
    }
}

/// The members of a command. A `null` member counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn optional(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &str) -> Result<&'a Value, Error> {
        self.optional(name).ok_or_else(|| Error::missing(name))
    }

    fn optional_with<T>(
        &self,
        name: &str,
        convert: impl FnOnce(&str, &'a Value) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        self.optional(name)
            .map(|value| convert(name, value))
            .transpose()
    }

    /// The `QueueName` of every command that only looks a queue up.
    fn queue(&self) -> Result<&'a str, Error> {
        string(QUEUE_NAME, self.required(QUEUE_NAME)?)
    }

    fn create_queue(&self) -> Result<Command<'a>, Error> {
        let name = self.required(QUEUE_NAME)?;
        let attributes = self.optional(ATTRIBUTES);
        let name = string(QUEUE_NAME, name)?;
        let name = QueueName::parse(name).map_err(|_| {
            Error::new(
                ErrorCode::QueueNameInvalid,
                format!(
                    "queue name {name:?} must be 1 to 80 characters from A-Z a-z 0-9 _ -, \
                     optionally ending in .fifo after at most 75 of them"
                ),
            )
        })?;
        Ok(Command::CreateQueue {
            name,
            attributes: attributes.map_or_else(|| Ok(Attributes::default()), attributes_from)?,
        })
    }

    fn set_queue_attributes(&self) -> Result<Command<'a>, Error> {
        let queue = self.required(QUEUE_NAME)?;
        let attributes = self.required(ATTRIBUTES)?;
        Ok(Command::SetQueueAttributes {
            queue: string(QUEUE_NAME, queue)?,
            attributes: attributes_from(attributes)?,
        })
    }

    fn send_message(&self) -> Result<Command<'a>, Error> {
        let queue = self.required(QUEUE_NAME)?;
        let body = self.required("Body")?;
        let body = string("Body", body)?;
        if !(1..=1024).contains(&body.len()) {
            return Err(Error::invalid_value(
                "Body",
                "must be 1 to 1024 bytes in UTF-8",
            ));
        }
        Ok(Command::SendMessage {
            queue: string(QUEUE_NAME, queue)?,
            body,
            delay: self.optional_with(DELAY_SECONDS, ranged)?,
            group: self
                .optional_with("GroupId", string)?
                .map(GroupId::parse)
                .transpose()
                .map_err(|_| Error::invalid_value("GroupId", TOKEN_RULE))?,
            deduplication: self
                .optional_with("DeduplicationId", string)?
                .map(DeduplicationId::parse)
                .transpose()
                .map_err(|_| Error::invalid_value("DeduplicationId", TOKEN_RULE))?,
        })
    }

    fn receive_message(&self) -> Result<Command<'a>, Error> {
        let queue = self.queue()?;
        Ok(Command::ReceiveMessage {
            queue,
            max_messages: self.optional_with("MaxMessages", ranged)?,
            visibility_timeout: self.optional_with(VISIBILITY_TIMEOUT, ranged)?,
        })
    }
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(name, "must be a string"))
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(name, "must be true or false"))
}

/// Integers are numbers written without fraction or exponent. With serde_json's
/// `arbitrary_precision` feature a number keeps its text, so `as_i64` accepts `-0` and rejects
/// `1.0` and `1e0`. Anything past `i64` is outside every range of the spec.
fn ranged<const MIN: u32, const MAX: u32>(
    name: &str,
    value: &Value,
) -> Result<Ranged<MIN, MAX>, Error> {
    let integer = value
        .as_i64()
        .ok_or_else(|| Error::invalid_value(name, "must be an integer"))?;
    Ranged::try_from(integer).map_err(|e| Error::invalid_value(name, e))
}

fn attributes_from(value: &Value) -> Result<Attributes<'_>, Error> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value(ATTRIBUTES, "must be an object"));
    };
    let mut attributes = Attributes::default();
    for (name, value) in members.iter().filter(|(_, value)| !value.is_null()) {
        match name.as_str() {
            VISIBILITY_TIMEOUT => attributes.visibility_timeout = Some(ranged(name, value)?),
            DELAY_SECONDS => attributes.delay = Some(ranged(name, value)?),
            "RetentionSeconds" => attributes.retention = Some(ranged(name, value)?),
            "Fifo" => attributes.fifo = Some(boolean(name, value)?),
            "ContentBasedDeduplication" => {
                attributes.content_based_deduplication = Some(boolean(name, value)?);
            }
            "DeadLetterQueue" => attributes.dead_letter_queue = Some(string(name, value)?),
            "MaxReceiveCount" => attributes.max_receive_count = Some(ranged(name, value)?),
            _ => {
                return Err(Error::invalid_value(
                    ATTRIBUTES,
                    format!("unknown attribute {name:?}"),
                ));
            }
        }
    }
    Ok(attributes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(line: &str) -> ErrorCode {
        let request = Request::from_line(line.as_bytes());
        match request.and_then(|r| r.command().map(|_| ())) {
            Ok(()) => panic!("{line} was accepted"),
            Err(e) => e.code(),
        }
    }

    fn parse<T>(line: &str, check: impl FnOnce(Command<'_>) -> T) -> T {
        let request = Request::from_line(line.as_bytes()).expect("line has a valid shape");
        check(request.command().expect("command has valid parameters"))
    }

    #[test]
    fn bad_shapes_are_invalid_requests() {
        let cases: [&[u8]; 9] = [
            b"",
            b"   ",
            b"[]",
            b"{}",
            b"{\"Command\": 1}",
            b"{\"Command\": null}",
            b"{\"Command\": \"Nope\"}",
            b"{\"command\": \"Tick\", \"Seconds\": 1}",
            b"{\"Command\": \"Tick\", \"Seconds\": 1} x",
        ];
        for line in cases {
            let error = Request::from_line(line).and_then(|r| r.command().map(|_| ()));
            assert_eq!(
                error.unwrap_err().code(),
                ErrorCode::InvalidRequest,
                "{line:?}"
            );
        }
        let not_utf8 = Request::from_line(b"{\"Command\": \"\xff\"}").unwrap_err();
        assert_eq!(not_utf8.code(), ErrorCode::InvalidRequest);
    }

    #[test]
    fn missing_beats_wrong_type_of_another_parameter() {
        let line = r#"{"Command": "SendMessage", "Body": 5}"#;
        assert_eq!(code_of(line), ErrorCode::MissingParameter);
        let line = r#"{"Command": "SendMessage", "QueueName": null, "Body": "b"}"#;
        assert_eq!(code_of(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn integers_must_be_written_as_integers_in_range() {
        for seconds in [
            "1.0",
            "1e0",
            "\"1\"",
            "-1",
            "1209601",
            "true",
            "99999999999999999999",
        ] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(
                code_of(&line),
                ErrorCode::InvalidParameterValue,
                "{seconds}"
            );
        }
        let line = r#"{"Command": "Tick", "Seconds": 1209600, "Extra": 1}"#;
        parse(line, |c| {
            let Command::Tick { seconds } = c else {
                panic!("not a Tick: {c:?}")
            };
            assert_eq!(seconds.get(), 1_209_600);
        });
    }

    #[test]
    fn create_queue_checks_the_name_syntax_first() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 5}"#;
        assert_eq!(code_of(line), ErrorCode::QueueNameInvalid);
        let line = r#"{"Command": "CreateQueue", "QueueName": "ab", "Attributes": 5}"#;
        assert_eq!(code_of(line), ErrorCode::InvalidParameterValue);
        let line = r#"{"Command": "CreateQueue", "QueueName": 5}"#;
        assert_eq!(code_of(line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn lookup_commands_accept_any_queue_name_string() {
        parse(r#"{"Command": "PurgeQueue", "QueueName": ""}"#, |c| {
            assert_eq!(c, Command::PurgeQueue { queue: "" });
        });
    }

    #[test]
    fn attributes_are_typed_and_null_means_absent() {
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes":
            {"VisibilityTimeout": 5, "DelaySeconds": null, "Fifo": false,
             "DeadLetterQueue": "dlq", "MaxReceiveCount": 3}}"#;
        parse(line, |c| {
            let Command::SetQueueAttributes { queue, attributes } = c else {
                panic!("not SetQueueAttributes: {c:?}")
            };
            assert_eq!(queue, "q");
            assert_eq!(
                attributes,
                Attributes {
                    visibility_timeout: Some(Ranged::constant(5)),
                    fifo: Some(false),
                    dead_letter_queue: Some("dlq"),
                    max_receive_count: Some(Ranged::constant(3)),
                    ..Attributes::default()
                }
            );
        });
        for bad in [
            r#""Unknown": 1"#,
            r#""Fifo": 1"#,
            r#""VisibilityTimeout": 43201"#,
            r#""DeadLetterQueue": 1"#,
            r#""RetentionSeconds": 59"#,
        ] {
            let line = format!(
                r#"{{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {{{bad}}}}}"#
            );
            assert_eq!(code_of(&line), ErrorCode::InvalidParameterValue, "{bad}");
        }
        let line = r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": null}"#;
        assert_eq!(code_of(line), ErrorCode::MissingParameter);
    }

    #[test]
    fn send_message_checks_body_and_token_lengths() {
        let long_body = "x".repeat(1025);
        let cases = [
            (r#""Body": """#.to_owned(), false),
            (format!(r#""Body": "{long_body}""#), false),
            (r#""Body": "é""#.to_owned(), true),
            (r#""Body": "b", "GroupId": """#.to_owned(), false),
            (r#""Body": "b", "GroupId": "a.b""#.to_owned(), false),
            (r#""Body": "b", "DeduplicationId": "a b""#.to_owned(), false),
            (r#""Body": "b", "DelaySeconds": 901"#.to_owned(), false),
            (
                r#""Body": "b", "GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 900"#
                    .to_owned(),
                true,
            ),
        ];
        for (members, valid) in cases {
            let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", {members}}}"#);
            let request = Request::from_line(line.as_bytes()).expect("valid shape");
            match request.command() {
                Ok(_) => assert!(valid, "{members}"),
                Err(e) => {
                    assert!(!valid, "{members}");
                    assert_eq!(e.code(), ErrorCode::InvalidParameterValue, "{members}");
                }
            }
        }
    }
}
