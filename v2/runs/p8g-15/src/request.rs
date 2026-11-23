//! Turns one input line into a typed [`Request`], applying the validation order of the spec:
//! request shape, then required parameters, then parameter values.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{
    Body, Bounded, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, QueueName,
    RetentionSeconds, TickSeconds, VisibilityTimeout,
};

/// The dead-letter attributes as the caller gave them. They are only valid together;
/// each command reports a lone one at its own place in the rule order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeadLetterRequest {
    Unset,
    Complete {
        queue: QueueName,
        max_receive_count: MaxReceiveCount,
    },
    Incomplete,
}

/// The attributes named in a `CreateQueue` or `SetQueueAttributes` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSet {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay: Option<DelaySeconds>,
    pub retention: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter: DeadLetterRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub queue: QueueName,
    pub body: Body,
    pub delay: Option<DelaySeconds>,
    pub group: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveMessage {
    pub queue: QueueName,
    pub max_messages: MaxMessages,
    pub visibility_timeout: Option<VisibilityTimeout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    CreateQueue {
        name: QueueName,
        attributes: AttributeSet,
    },
    SetQueueAttributes {
        name: QueueName,
        attributes: AttributeSet,
    },
    GetQueueAttributes {
        name: QueueName,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        name: QueueName,
    },
    PurgeQueue {
        name: QueueName,
    },
    Tick {
        seconds: TickSeconds,
    },
    SendMessage(SendMessage),
    ReceiveMessage(ReceiveMessage),
    DeleteMessage {
        queue: QueueName,
        /// Kept as text: whether it was ever issued is decided after the queue lookup.
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: QueueName,
        receipt_handle: String,
        visibility_timeout: VisibilityTimeout,
    },
}

#[derive(Clone, Copy)]
enum Command {
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

impl Command {
    fn from_name(name: &str) -> Option<Self> {
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

/// Read access to the members of a JSON object, where `null` counts as absent.
struct Params<'a>(&'a Map<String, Value>);

impl Params<'_> {
    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// Fails with `MissingParameter` for the first listed name that is absent.
    fn require(&self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::missing(name)),
            None => Ok(()),
        }
    }

    fn string(&self, name: &str) -> Result<Option<&str>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(Error::invalid_value(name, "a string")),
        }
    }

    fn boolean(&self, name: &str) -> Result<Option<bool>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(Error::invalid_value(name, "a boolean")),
        }
    }

    /// An integer within the range of `T`. Numbers with a fraction or exponent are not integers.
    fn bounded<T: Bounded>(&self, name: &str) -> Result<Option<T>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Number(number)) => number
                .as_i64()
                .and_then(T::from_i64)
                .map(Some)
                .ok_or_else(|| Error::invalid_value(name, "an integer within its range")),
            Some(_) => Err(Error::invalid_value(name, "an integer")),
        }
    }

    fn object(&self, name: &str) -> Result<Option<&Map<String, Value>>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Object(members)) => Ok(Some(members)),
            Some(_) => Err(Error::invalid_value(name, "an object")),
        }
    }

    fn required_string(&self, name: &str) -> Result<&str, Error> {
        self.string(name)?.ok_or_else(|| Error::missing(name))
    }

    fn required_bounded<T: Bounded>(&self, name: &str) -> Result<T, Error> {
        self.bounded(name)?.ok_or_else(|| Error::missing(name))
    }

    fn queue_name(&self) -> Result<QueueName, Error> {
        self.required_string("QueueName")
            .map(|name| QueueName::new(name.to_owned()))
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

fn parse_attributes(members: &Map<String, Value>) -> Result<AttributeSet, Error> {
    if let Some(unknown) = members
        .keys()
        .find(|key| !ATTRIBUTE_NAMES.contains(&key.as_str()))
    {
        return Err(Error::invalid_value(
            "Attributes",
            &format!("made of known attributes, not {unknown}"),
        ));
    }
    let attrs = Params(members);
    let dead_letter = match (
        attrs.string("DeadLetterQueue")?,
        attrs.bounded::<MaxReceiveCount>("MaxReceiveCount")?,
    ) {
        (None, None) => DeadLetterRequest::Unset,
        (Some(queue), Some(max_receive_count)) => DeadLetterRequest::Complete {
            queue: QueueName::new(queue.to_owned()),
            max_receive_count,
        },
        _ => DeadLetterRequest::Incomplete,
    };
    Ok(AttributeSet {
        visibility_timeout: attrs.bounded("VisibilityTimeout")?,
        delay: attrs.bounded("DelaySeconds")?,
        retention: attrs.bounded("RetentionSeconds")?,
        fifo: attrs.boolean("Fifo")?,
        content_based_deduplication: attrs.boolean("ContentBasedDeduplication")?,
        dead_letter,
    })
}

fn optional_attributes(params: &Params) -> Result<AttributeSet, Error> {
    match params.object("Attributes")? {
        Some(members) => parse_attributes(members),
        None => parse_attributes(&Map::new()),
    }
}

/// Parses one input line (without its line terminator).
pub fn parse(line: &[u8]) -> Result<Request, Error> {
    let object: Map<String, Value> = serde_json::from_slice(line).map_err(|source| {
        Error::new(
            ErrorCode::InvalidRequest,
            format!("line is not a JSON object: {source}"),
        )
    })?;
    let params = Params(&object);
    let command = match params.get("Command") {
        Some(Value::String(name)) => Command::from_name(name).ok_or_else(|| {
            Error::new(ErrorCode::InvalidRequest, format!("unknown command {name}"))
        })?,
        _ => {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "member Command must be a string",
            ));
        }
    };
    parse_command(command, &params)
}

fn parse_command(command: Command, params: &Params) -> Result<Request, Error> {
    match command {
        Command::CreateQueue => {
            params.require(&["QueueName"])?;
            let name = params.queue_name()?;
            if !name.has_valid_syntax() {
                return Err(Error::new(
                    ErrorCode::QueueNameInvalid,
                    format!("{:?} breaks the queue naming rules", name.as_str()),
                ));
            }
            let attributes = optional_attributes(params)?;
            Ok(Request::CreateQueue { name, attributes })
        }
        Command::SetQueueAttributes => {
            params.require(&["QueueName", "Attributes"])?;
            let name = params.queue_name()?;
            let attributes = optional_attributes(params)?;
            Ok(Request::SetQueueAttributes { name, attributes })
        }
        Command::GetQueueAttributes => {
            params.require(&["QueueName"])?;
            Ok(Request::GetQueueAttributes {
                name: params.queue_name()?,
            })
        }
        Command::ListQueues => Ok(Request::ListQueues {
            prefix: params.string("Prefix")?.map(str::to_owned),
        }),
        Command::DeleteQueue => {
            params.require(&["QueueName"])?;
            Ok(Request::DeleteQueue {
                name: params.queue_name()?,
            })
        }
        Command::PurgeQueue => {
            params.require(&["QueueName"])?;
            Ok(Request::PurgeQueue {
                name: params.queue_name()?,
            })
        }
        Command::Tick => {
            params.require(&["Seconds"])?;
            Ok(Request::Tick {
                seconds: params.required_bounded("Seconds")?,
            })
        }
        Command::SendMessage => {
            params.require(&["QueueName", "Body"])?;
            let queue = params.queue_name()?;
            let body = Body::new(params.required_string("Body")?.to_owned())
                .ok_or_else(|| Error::invalid_value("Body", "1 to 1024 bytes long"))?;
            let token_error =
                |name| Error::invalid_value(name, "1 to 128 characters of A-Z a-z 0-9 _ -");
            let group = params
                .string("GroupId")?
                .map(|id| GroupId::new(id.to_owned()).ok_or_else(|| token_error("GroupId")))
                .transpose()?;
            let deduplication_id = params
                .string("DeduplicationId")?
                .map(|id| {
                    DeduplicationId::new(id.to_owned())
                        .ok_or_else(|| token_error("DeduplicationId"))
                })
                .transpose()?;
            Ok(Request::SendMessage(SendMessage {
                queue,
                body,
                delay: params.bounded("DelaySeconds")?,
                group,
                deduplication_id,
            }))
        }
        Command::ReceiveMessage => {
            params.require(&["QueueName"])?;
            Ok(Request::ReceiveMessage(ReceiveMessage {
                queue: params.queue_name()?,
                max_messages: params.bounded("MaxMessages")?.unwrap_or_default(),
                visibility_timeout: params.bounded("VisibilityTimeout")?,
            }))
        }
        Command::DeleteMessage => {
            params.require(&["QueueName", "ReceiptHandle"])?;
            Ok(Request::DeleteMessage {
                queue: params.queue_name()?,
                receipt_handle: params.required_string("ReceiptHandle")?.to_owned(),
            })
        }
        Command::ChangeMessageVisibility => {
            params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
            Ok(Request::ChangeMessageVisibility {
                queue: params.queue_name()?,
                receipt_handle: params.required_string("ReceiptHandle")?.to_owned(),
                visibility_timeout: params.required_bounded("VisibilityTimeout")?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(line: &str) -> ErrorCode {
        match parse(line.as_bytes()) {
            Err(error) => error.code(),
            Ok(request) => panic!("{line} parsed as {request:?}"),
        }
    }

    #[test]
    fn rejects_lines_that_are_not_commands() {
        for line in [
            "",
            "   ",
            "[]",
            "\"CreateQueue\"",
            "{",
            "{} {}",
            "{\"Command\": 1}",
            "{\"Command\": null}",
            "{\"command\": \"Tick\"}",
            "{\"Command\": \"tick\"}",
            "{\"Command\": \"Nope\"}",
        ] {
            assert_eq!(code_of(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\":\"Tick\"\xff}").unwrap_err().code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameters_win_over_bad_values() {
        assert_eq!(
            code_of(
                r#"{"Command":"ChangeMessageVisibility","QueueName":5,"VisibilityTimeout":"x"}"#
            ),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code_of(r#"{"Command":"Tick","Seconds":null}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code_of(r#"{"Command":"SetQueueAttributes","QueueName":"q"}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        for value in [
            "30.0",
            "3e1",
            "\"30\"",
            "true",
            "-1",
            "1209601",
            "18446744073709551616",
        ] {
            let line = format!(r#"{{"Command":"Tick","Seconds":{value}}}"#);
            assert_eq!(code_of(&line), ErrorCode::InvalidParameterValue, "{value}");
        }
        assert_eq!(
            parse(br#"{"Command":"Tick","Seconds":1209600}"#),
            Ok(Request::Tick {
                seconds: TickSeconds::from_i64(1_209_600).unwrap()
            })
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        assert_eq!(
            code_of(r#"{"Command":"CreateQueue","QueueName":"a b","Attributes":3}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code_of(r#"{"Command":"CreateQueue","QueueName":7,"Attributes":3}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code_of(r#"{"Command":"CreateQueue","QueueName":"a","Attributes":3}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn attributes_are_checked_by_name_type_and_range() {
        for attributes in [
            r#"{"Colour":1}"#,
            r#"{"Colour":null}"#,
            r#"{"VisibilityTimeout":43201}"#,
            r#"{"Fifo":"true"}"#,
            r#"{"DeadLetterQueue":3,"MaxReceiveCount":2}"#,
            r#"{"MaxReceiveCount":0}"#,
        ] {
            let line =
                format!(r#"{{"Command":"CreateQueue","QueueName":"q","Attributes":{attributes}}}"#);
            assert_eq!(
                code_of(&line),
                ErrorCode::InvalidParameterValue,
                "{attributes}"
            );
        }
    }

    #[test]
    fn null_members_count_as_absent() {
        let parsed = parse(
            br#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"Fifo":null,"DeadLetterQueue":null}}"#,
        );
        assert_eq!(
            parsed,
            Ok(Request::CreateQueue {
                name: QueueName::new("q".to_owned()),
                attributes: AttributeSet {
                    visibility_timeout: None,
                    delay: None,
                    retention: None,
                    fifo: None,
                    content_based_deduplication: None,
                    dead_letter: DeadLetterRequest::Unset,
                },
            })
        );
    }

    #[test]
    fn a_lone_dead_letter_attribute_parses_as_incomplete() {
        let parsed = parse(
            br#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":3}}"#,
        );
        let Ok(Request::SetQueueAttributes { attributes, .. }) = parsed else {
            panic!("did not parse: {parsed:?}");
        };
        assert_eq!(attributes.dead_letter, DeadLetterRequest::Incomplete);
    }

    #[test]
    fn send_message_checks_body_and_tokens() {
        for member in [
            r#""Body":"""#,
            r#""Body":1"#,
            r#""Body":"b","GroupId":"""#,
            r#""Body":"b","DeduplicationId":"a b""#,
            r#""Body":"b","DelaySeconds":901"#,
        ] {
            let line = format!(r#"{{"Command":"SendMessage","QueueName":"q",{member}}}"#);
            assert_eq!(code_of(&line), ErrorCode::InvalidParameterValue, "{member}");
        }
    }

    #[test]
    fn receive_message_defaults_to_one_message() {
        assert_eq!(
            parse(br#"{"Command":"ReceiveMessage","QueueName":"q","Extra":true}"#),
            Ok(Request::ReceiveMessage(ReceiveMessage {
                queue: QueueName::new("q".to_owned()),
                max_messages: MaxMessages::default(),
                visibility_timeout: None,
            }))
        );
    }
}
