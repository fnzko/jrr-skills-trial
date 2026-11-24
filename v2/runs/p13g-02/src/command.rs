//! Turns one input line into a validated [`Command`], applying the check order
//! of the spec: request shape, required parameters, then parameter values.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{DeduplicationId, GroupId, QueueName, Seconds};

/// A parsed command. Checks that need queue state happen later, in the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributeValues,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributeValues,
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
        message: Outgoing,
    },
    ReceiveMessage {
        queue: String,
        max_messages: u32,
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

/// The attributes a request names; every attribute left out is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeValues {
    pub visibility_timeout: Option<Seconds>,
    pub delay: Option<Seconds>,
    pub retention: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

impl AttributeValues {
    /// The requested dead-letter configuration. Both members or neither must be
    /// present; one without the other is an invalid parameter value.
    pub fn dead_letter(&self) -> Result<Option<(&str, u32)>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            (None, None) => Ok(None),
            _ => Err(Error::invalid_value(
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}

/// The parameters of a `SendMessage`, apart from the queue it goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub body: String,
    pub delay: Option<Seconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
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

const VISIBILITY_TIMEOUT_RANGE: RangeInclusive<u32> = 0..=43_200;

static NULL: Value = Value::Null;

/// Members of a command object. A member whose value is `null` counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    fn optional(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// Returns the values of all `names`, or the first one that is absent.
    /// Callers read every required parameter before checking any value so that
    /// a missing parameter wins over an invalid one.
    fn required<const N: usize>(&self, names: [&str; N]) -> Result<[&'a Value; N], Error> {
        let mut found = [&NULL; N];
        for (slot, name) in found.iter_mut().zip(names) {
            *slot = self
                .optional(name)
                .ok_or_else(|| Error::missing_parameter(name))?;
        }
        Ok(found)
    }

    fn string(&self, name: &str) -> Result<Option<&'a str>, Error> {
        self.optional(name)
            .map(|value| string(name, value))
            .transpose()
    }

    fn boolean(&self, name: &str) -> Result<Option<bool>, Error> {
        self.optional(name)
            .map(|value| boolean(name, value))
            .transpose()
    }

    fn integer(&self, name: &str, range: RangeInclusive<u32>) -> Result<Option<u32>, Error> {
        self.optional(name)
            .map(|value| integer(name, value, range))
            .transpose()
    }

    fn seconds(&self, name: &str, range: RangeInclusive<u32>) -> Result<Option<Seconds>, Error> {
        Ok(self.integer(name, range)?.map(Seconds::from))
    }
}

fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a string")))
}

fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be true or false")))
}

/// Accepts only JSON numbers written as integers: `serde_json` stores anything
/// with a fraction or an exponent as a float, which `as_i64` rejects.
fn integer(name: &str, value: &Value, range: RangeInclusive<u32>) -> Result<u32, Error> {
    value
        .as_i64()
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            Error::invalid_value(format!(
                "{name} must be an integer from {} to {}",
                range.start(),
                range.end()
            ))
        })
}

fn queue_name(name: &Value) -> Result<String, Error> {
    string("QueueName", name).map(str::to_owned)
}

fn attributes(value: &Value) -> Result<AttributeValues, Error> {
    let Value::Object(members) = value else {
        return Err(Error::invalid_value("Attributes must be an object"));
    };
    if let Some(unknown) = members
        .iter()
        .find(|(name, value)| !value.is_null() && !ATTRIBUTE_NAMES.contains(&name.as_str()))
    {
        return Err(Error::invalid_value(format!(
            "unknown attribute {}",
            unknown.0
        )));
    }
    let params = Params(members);
    Ok(AttributeValues {
        visibility_timeout: params.seconds("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
        delay: params.seconds("DelaySeconds", 0..=900)?,
        retention: params.seconds("RetentionSeconds", 60..=1_209_600)?,
        fifo: params.boolean("Fifo")?,
        content_based_deduplication: params.boolean("ContentBasedDeduplication")?,
        dead_letter_queue: params.string("DeadLetterQueue")?.map(str::to_owned),
        max_receive_count: params.integer("MaxReceiveCount", 1..=1000)?,
    })
}

/// Parses one input line, which must be a JSON object naming a known command.
pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let invalid = |message: String| Error::new(ErrorCode::InvalidRequest, message);
    let text = std::str::from_utf8(line)
        .map_err(|error| invalid(format!("line is not valid UTF-8: {error}")))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| invalid(format!("line is not valid JSON: {error}")))?;
    let Value::Object(members) = &value else {
        return Err(invalid("line is not a JSON object".to_owned()));
    };
    let params = Params(members);
    let name = params
        .optional("Command")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("Command must be a string".to_owned()))?;
    match name {
        "CreateQueue" => create_queue(&params),
        "SetQueueAttributes" => set_queue_attributes(&params),
        "GetQueueAttributes" => {
            single_queue(&params).map(|queue| Command::GetQueueAttributes { queue })
        }
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.string("Prefix")?.map(str::to_owned),
        }),
        "DeleteQueue" => single_queue(&params).map(|queue| Command::DeleteQueue { queue }),
        "PurgeQueue" => single_queue(&params).map(|queue| Command::PurgeQueue { queue }),
        "Tick" => tick(&params),
        "SendMessage" => send_message(&params),
        "ReceiveMessage" => receive_message(&params),
        "DeleteMessage" => delete_message(&params),
        "ChangeMessageVisibility" => change_message_visibility(&params),
        other => Err(invalid(format!("unknown command {other:?}"))),
    }
}

fn single_queue(params: &Params) -> Result<String, Error> {
    let [name] = params.required(["QueueName"])?;
    queue_name(name)
}

fn create_queue(params: &Params) -> Result<Command, Error> {
    let [name] = params.required(["QueueName"])?;
    let name = QueueName::parse(string("QueueName", name)?)?;
    let attributes = params.optional("Attributes").map(attributes).transpose()?;
    Ok(Command::CreateQueue {
        name,
        attributes: attributes.unwrap_or_default(),
    })
}

fn set_queue_attributes(params: &Params) -> Result<Command, Error> {
    let [name, attribute_values] = params.required(["QueueName", "Attributes"])?;
    Ok(Command::SetQueueAttributes {
        queue: queue_name(name)?,
        attributes: attributes(attribute_values)?,
    })
}

fn tick(params: &Params) -> Result<Command, Error> {
    let [seconds] = params.required(["Seconds"])?;
    Ok(Command::Tick {
        seconds: integer("Seconds", seconds, 0..=1_209_600)?.into(),
    })
}

fn send_message(params: &Params) -> Result<Command, Error> {
    let [name, body] = params.required(["QueueName", "Body"])?;
    let queue = queue_name(name)?;
    let body = string("Body", body)?;
    if !(1..=1024).contains(&body.len()) {
        return Err(Error::invalid_value("Body must be 1 to 1024 bytes"));
    }
    Ok(Command::SendMessage {
        queue,
        message: Outgoing {
            body: body.to_owned(),
            delay: params.seconds("DelaySeconds", 0..=900)?,
            group: params.string("GroupId")?.map(GroupId::parse).transpose()?,
            deduplication_id: params
                .string("DeduplicationId")?
                .map(DeduplicationId::parse)
                .transpose()?,
        },
    })
}

fn receive_message(params: &Params) -> Result<Command, Error> {
    let [name] = params.required(["QueueName"])?;
    Ok(Command::ReceiveMessage {
        queue: queue_name(name)?,
        max_messages: params.integer("MaxMessages", 1..=10)?.unwrap_or(1),
        visibility_timeout: params.seconds("VisibilityTimeout", VISIBILITY_TIMEOUT_RANGE)?,
    })
}

fn delete_message(params: &Params) -> Result<Command, Error> {
    let [name, handle] = params.required(["QueueName", "ReceiptHandle"])?;
    Ok(Command::DeleteMessage {
        queue: queue_name(name)?,
        receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
    })
}

fn change_message_visibility(params: &Params) -> Result<Command, Error> {
    let [name, handle, timeout] =
        params.required(["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
    Ok(Command::ChangeMessageVisibility {
        queue: queue_name(name)?,
        receipt_handle: string("ReceiptHandle", handle)?.to_owned(),
        visibility_timeout: integer("VisibilityTimeout", timeout, VISIBILITY_TIMEOUT_RANGE)?.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code()
    }

    #[test]
    fn malformed_requests_are_invalid_requests() {
        let lines = [
            "",
            "   ",
            "not json",
            "[]",
            "\"PurgeQueue\"",
            "{}",
            r#"{"Command": 1}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"command": "PurgeQueue", "QueueName": "q"}"#,
            r#"{"Command": "Tick", "Seconds": 1} trailing"#,
        ];
        for line in lines {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "line {line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"\xff\"}").unwrap_err().code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameter_wins_over_invalid_value() {
        let lines = [
            r#"{"Command": "SendMessage", "Body": 5}"#,
            r#"{"Command": "SendMessage", "QueueName": 5}"#,
            r#"{"Command": "SendMessage", "QueueName": null, "Body": "x"}"#,
            r#"{"Command": "PurgeQueue"}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": 1}"#,
            r#"{"Command": "CreateQueue", "Attributes": 3}"#,
        ];
        for line in lines {
            assert_eq!(code(line), ErrorCode::MissingParameter, "line {line:?}");
        }
    }

    #[test]
    fn integers_must_be_written_as_integers_in_range() {
        let lines = [
            r#"{"Command": "Tick", "Seconds": 30.0}"#,
            r#"{"Command": "Tick", "Seconds": 3e1}"#,
            r#"{"Command": "Tick", "Seconds": "30"}"#,
            r#"{"Command": "Tick", "Seconds": -1}"#,
            r#"{"Command": "Tick", "Seconds": 1209601}"#,
            r#"{"Command": "Tick", "Seconds": 99999999999999999999}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#,
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 43201}"#,
        ];
        for line in lines {
            assert_eq!(
                code(line),
                ErrorCode::InvalidParameterValue,
                "line {line:?}"
            );
        }
        assert_eq!(
            parse(br#"{"Command": "Tick", "Seconds": 1209600}"#),
            Ok(Command::Tick {
                seconds: Seconds::new(1_209_600)
            })
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": 4, "Attributes": 3}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": 3}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn attributes_are_checked_by_name_type_and_range() {
        let attribute_sets = [
            r#"{"Bogus": 1}"#,
            r#"{"VisibilityTimeout": 43201}"#,
            r#"{"DelaySeconds": 901}"#,
            r#"{"RetentionSeconds": 59}"#,
            r#"{"RetentionSeconds": 1209601}"#,
            r#"{"Fifo": "true"}"#,
            r#"{"ContentBasedDeduplication": 1}"#,
            r#"{"DeadLetterQueue": 7}"#,
            r#"{"MaxReceiveCount": 0}"#,
            r#"{"MaxReceiveCount": 1001}"#,
            r#"[]"#,
        ];
        for set in attribute_sets {
            let line = format!(
                r#"{{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {set}}}"#
            );
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "attributes {set}"
            );
        }
    }

    #[test]
    fn null_members_count_as_absent() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": null, "Bogus": null, "DelaySeconds": 5}}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::CreateQueue {
                name: QueueName::parse("q").unwrap(),
                attributes: AttributeValues {
                    delay: Some(Seconds::new(5)),
                    ..AttributeValues::default()
                },
            })
        );
        assert_eq!(
            parse(br#"{"Command": "ListQueues", "Prefix": null, "Extra": 1}"#),
            Ok(Command::ListQueues { prefix: None })
        );
    }

    #[test]
    fn send_message_parses_all_parameters() {
        let line = r#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "hi", "GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 3}"#;
        assert_eq!(
            parse(line.as_bytes()),
            Ok(Command::SendMessage {
                queue: "q.fifo".to_owned(),
                message: Outgoing {
                    body: "hi".to_owned(),
                    delay: Some(Seconds::new(3)),
                    group: Some(GroupId::parse("g").unwrap()),
                    deduplication_id: Some(DeduplicationId::parse("d").unwrap()),
                },
            })
        );
    }

    #[test]
    fn send_message_body_is_limited_in_bytes() {
        let body = |text: &str| {
            format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{text}"}}"#)
        };
        assert_eq!(code(&body("")), ErrorCode::InvalidParameterValue);
        assert_eq!(
            code(&body(&"a".repeat(1025))),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(&body(&"\u{e9}".repeat(513))),
            ErrorCode::InvalidParameterValue
        );
        let limit_body = "\u{e9}".repeat(512);
        assert_eq!(
            parse(body(&limit_body).as_bytes()),
            Ok(Command::SendMessage {
                queue: "q".to_owned(),
                message: Outgoing {
                    body: limit_body,
                    delay: None,
                    group: None,
                    deduplication_id: None,
                },
            })
        );
    }

    #[test]
    fn send_message_identifiers_are_validated() {
        for member in [
            r#""GroupId": "a b""#,
            r#""DeduplicationId": """#,
            r#""GroupId": 5"#,
        ] {
            let line =
                format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "x", {member}}}"#);
            assert_eq!(
                code(&line),
                ErrorCode::InvalidParameterValue,
                "member {member}"
            );
        }
    }

    #[test]
    fn receive_message_defaults_to_one_message() {
        assert_eq!(
            parse(br#"{"Command": "ReceiveMessage", "QueueName": "q"}"#),
            Ok(Command::ReceiveMessage {
                queue: "q".to_owned(),
                max_messages: 1,
                visibility_timeout: None,
            })
        );
    }

    #[test]
    fn dead_letter_members_must_come_together() {
        let only_queue = AttributeValues {
            dead_letter_queue: Some("dlq".to_owned()),
            ..AttributeValues::default()
        };
        assert_eq!(
            only_queue.dead_letter().unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        let both = AttributeValues {
            max_receive_count: Some(3),
            ..only_queue
        };
        assert_eq!(both.dead_letter(), Ok(Some(("dlq", 3))));
        assert_eq!(AttributeValues::default().dead_letter(), Ok(None));
    }
}
