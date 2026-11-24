//! Turns one input line into a validated [`Request`].
//!
//! Checks run in the order the spec fixes: request shape, then required parameters,
//! then parameter values. Required parameters are all looked up before any value is
//! inspected, which is why every `parse_*` function fetches them first.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{
    Body, Bounded, DeduplicationId, DelaySeconds, GroupId, MaxMessages, MaxReceiveCount, QueueName,
    RetentionSeconds, TickSeconds, VisibilityTimeout,
};

#[derive(Debug)]
pub enum Request {
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
        seconds: TickSeconds,
    },
    SendMessage {
        queue: String,
        message: NewMessage,
    },
    ReceiveMessage {
        queue: String,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: VisibilityTimeout,
    },
}

/// The queue attributes a request names; the ones it leaves out are `None`.
///
/// `dead_letter_queue` is a plain string because the spec accepts any string there and
/// reports a missing queue only when the command runs.
#[derive(Debug, Default)]
pub struct AttributeSet {
    pub visibility_timeout: Option<VisibilityTimeout>,
    pub delay_seconds: Option<DelaySeconds>,
    pub retention_seconds: Option<RetentionSeconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<MaxReceiveCount>,
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

#[derive(Debug)]
pub struct NewMessage {
    pub body: Body,
    pub delay_seconds: Option<DelaySeconds>,
    pub group_id: Option<GroupId>,
    pub deduplication_id: Option<DeduplicationId>,
}

fn error(code: ErrorCode, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

fn invalid_value(name: &str, expected: impl std::fmt::Display) -> Error {
    error(
        ErrorCode::InvalidParameterValue,
        format!("{name} must be {expected}"),
    )
}

/// The members of a JSON object; a `null` member counts as absent.
struct Params<'a>(&'a Map<String, Value>);

/// One present parameter, kept with its name for error messages.
#[derive(Clone, Copy)]
struct Param<'a> {
    name: &'a str,
    value: &'a Value,
}

impl<'a> Params<'a> {
    fn get(&self, name: &'a str) -> Option<Param<'a>> {
        self.0
            .get(name)
            .filter(|value| !value.is_null())
            .map(|value| Param { name, value })
    }

    fn required(&self, name: &'a str) -> Result<Param<'a>, Error> {
        self.get(name)
            .ok_or_else(|| error(ErrorCode::MissingParameter, format!("{name} is required")))
    }

    fn optional<T>(
        &self,
        name: &'a str,
        parse: impl FnOnce(Param<'a>) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        self.get(name).map(parse).transpose()
    }
}

impl<'a> Param<'a> {
    fn string(self) -> Result<&'a str, Error> {
        self.value
            .as_str()
            .ok_or_else(|| invalid_value(self.name, "a string"))
    }

    fn boolean(self) -> Result<bool, Error> {
        self.value
            .as_bool()
            .ok_or_else(|| invalid_value(self.name, "a boolean"))
    }

    fn integer<const MIN: u32, const MAX: u32>(self) -> Result<Bounded<MIN, MAX>, Error> {
        self.value
            .as_i64()
            .and_then(Bounded::new)
            .ok_or_else(|| invalid_value(self.name, format_args!("an integer from {MIN} to {MAX}")))
    }

    fn object(self) -> Result<Params<'a>, Error> {
        self.value
            .as_object()
            .map(Params)
            .ok_or_else(|| invalid_value(self.name, "an object"))
    }

    fn text<T>(self, new: impl FnOnce(&str) -> Option<T>, rule: &str) -> Result<T, Error> {
        new(self.string()?).ok_or_else(|| invalid_value(self.name, rule))
    }
}

impl AttributeSet {
    fn parse(attributes: &Params<'_>) -> Result<Self, Error> {
        if let Some(name) = attributes
            .0
            .iter()
            .find(|(name, value)| !value.is_null() && !ATTRIBUTE_NAMES.contains(&name.as_str()))
            .map(|(name, _)| name)
        {
            return Err(invalid_value(
                "Attributes",
                format_args!("made of known attributes, not {name}"),
            ));
        }
        Ok(Self {
            visibility_timeout: attributes.optional("VisibilityTimeout", Param::integer)?,
            delay_seconds: attributes.optional("DelaySeconds", Param::integer)?,
            retention_seconds: attributes.optional("RetentionSeconds", Param::integer)?,
            fifo: attributes.optional("Fifo", Param::boolean)?,
            content_based_deduplication: attributes
                .optional("ContentBasedDeduplication", Param::boolean)?,
            dead_letter_queue: attributes
                .optional("DeadLetterQueue", |p| p.string().map(str::to_owned))?,
            max_receive_count: attributes.optional("MaxReceiveCount", Param::integer)?,
        })
    }
}

/// Parses one line, without its line feed.
pub fn parse(line: &[u8]) -> Result<Request, Error> {
    let text = std::str::from_utf8(line)
        .map_err(|e| error(ErrorCode::InvalidRequest, format!("line is not UTF-8: {e}")))?;
    let Value::Object(members) = serde_json::from_str(text)
        .map_err(|e| error(ErrorCode::InvalidRequest, format!("line is not JSON: {e}")))?
    else {
        return Err(error(
            ErrorCode::InvalidRequest,
            "line is not a JSON object",
        ));
    };
    let params = Params(&members);
    let command = params
        .get("Command")
        .and_then(|p| p.value.as_str())
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "Command must be a string"))?;
    match command {
        "CreateQueue" => parse_create_queue(&params),
        "SetQueueAttributes" => parse_set_queue_attributes(&params),
        "GetQueueAttributes" => {
            let queue = params.required("QueueName")?;
            Ok(Request::GetQueueAttributes {
                queue: queue.string()?.to_owned(),
            })
        }
        "ListQueues" => Ok(Request::ListQueues {
            prefix: params.optional("Prefix", |p| p.string().map(str::to_owned))?,
        }),
        "DeleteQueue" => {
            let queue = params.required("QueueName")?;
            Ok(Request::DeleteQueue {
                queue: queue.string()?.to_owned(),
            })
        }
        "PurgeQueue" => {
            let queue = params.required("QueueName")?;
            Ok(Request::PurgeQueue {
                queue: queue.string()?.to_owned(),
            })
        }
        "Tick" => Ok(Request::Tick {
            seconds: params.required("Seconds")?.integer()?,
        }),
        "SendMessage" => parse_send_message(&params),
        "ReceiveMessage" => parse_receive_message(&params),
        "DeleteMessage" => {
            let queue = params.required("QueueName")?;
            let receipt_handle = params.required("ReceiptHandle")?;
            Ok(Request::DeleteMessage {
                queue: queue.string()?.to_owned(),
                receipt_handle: receipt_handle.string()?.to_owned(),
            })
        }
        "ChangeMessageVisibility" => {
            let queue = params.required("QueueName")?;
            let receipt_handle = params.required("ReceiptHandle")?;
            let visibility_timeout = params.required("VisibilityTimeout")?;
            Ok(Request::ChangeMessageVisibility {
                queue: queue.string()?.to_owned(),
                receipt_handle: receipt_handle.string()?.to_owned(),
                visibility_timeout: visibility_timeout.integer()?,
            })
        }
        other => Err(error(
            ErrorCode::InvalidRequest,
            format!("unknown command {other}"),
        )),
    }
}

fn parse_create_queue(params: &Params<'_>) -> Result<Request, Error> {
    let name = params.required("QueueName")?;
    let attributes = params.get("Attributes");
    // The name is checked before the attributes, and its syntax has its own code.
    let name = name.string().and_then(|text| {
        QueueName::new(text).ok_or_else(|| {
            error(
                ErrorCode::QueueNameInvalid,
                format!("{text:?} breaks the queue naming rules"),
            )
        })
    })?;
    let attributes = match attributes {
        Some(attributes) => AttributeSet::parse(&attributes.object()?)?,
        None => AttributeSet::default(),
    };
    Ok(Request::CreateQueue { name, attributes })
}

fn parse_set_queue_attributes(params: &Params<'_>) -> Result<Request, Error> {
    let queue = params.required("QueueName")?;
    let attributes = params.required("Attributes")?;
    Ok(Request::SetQueueAttributes {
        queue: queue.string()?.to_owned(),
        attributes: AttributeSet::parse(&attributes.object()?)?,
    })
}

fn parse_send_message(params: &Params<'_>) -> Result<Request, Error> {
    let queue = params.required("QueueName")?;
    let body = params.required("Body")?;
    let queue = queue.string()?.to_owned();
    let body = body.text(Body::new, "a string of 1 to 1024 bytes")?;
    let token = "1 to 128 characters from A-Z a-z 0-9 _ -";
    Ok(Request::SendMessage {
        queue,
        message: NewMessage {
            body,
            delay_seconds: params.optional("DelaySeconds", Param::integer)?,
            group_id: params.optional("GroupId", |p| p.text(GroupId::new, token))?,
            deduplication_id: params
                .optional("DeduplicationId", |p| p.text(DeduplicationId::new, token))?,
        },
    })
}

fn parse_receive_message(params: &Params<'_>) -> Result<Request, Error> {
    let queue = params.required("QueueName")?;
    Ok(Request::ReceiveMessage {
        queue: queue.string()?.to_owned(),
        max_messages: params
            .optional("MaxMessages", Param::integer)?
            .unwrap_or(MaxMessages::constant(1)),
        visibility_timeout: params.optional("VisibilityTimeout", Param::integer)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code()
    }

    #[test]
    fn malformed_lines_are_invalid_requests() {
        let cases: [&[u8]; 8] = [
            b"",
            b"   ",
            b"[]",
            b"{\"Command\": 1}",
            b"{\"Command\": null}",
            b"{\"Command\": \"Nope\"}",
            b"{\"Command\": \"Tick\"} trailing",
            b"{\"Command\": \"Tick\", \"Seconds\": 1, \"x\": \xff}",
        ];
        for line in cases {
            assert_eq!(
                parse(line).unwrap_err().code(),
                ErrorCode::InvalidRequest,
                "{}",
                String::from_utf8_lossy(line)
            );
        }
    }

    #[test]
    fn missing_parameters_are_reported_before_bad_values() {
        assert_eq!(
            code(r#"{"Command":"SendMessage","QueueName":5}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(r#"{"Command":"SendMessage","QueueName":"q","Body":null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn integers_must_be_plain_in_range_integers() {
        for seconds in [
            "30.0",
            "3e1",
            "\"30\"",
            "-1",
            "1209601",
            "true",
            "99999999999999999999",
        ] {
            assert_eq!(
                code(&format!(r#"{{"Command":"Tick","Seconds":{seconds}}}"#)),
                ErrorCode::InvalidParameterValue,
                "{seconds}"
            );
        }
        let request = parse(br#"{"Command":"Tick","Seconds":1209600}"#).unwrap();
        let Request::Tick { seconds } = request else {
            panic!("parsed as another command: {request:?}");
        };
        assert_eq!(seconds, TickSeconds::constant(1_209_600));
    }

    #[test]
    fn create_queue_checks_the_name_before_anything_else() {
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":"a b","Attributes":7}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":7,"Attributes":7}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command":"CreateQueue","QueueName":"q","Attributes":7}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn attributes_reject_unknown_names_and_bad_values() {
        for attributes in [
            r#"{"Colour":"red"}"#,
            r#"{"VisibilityTimeout":43201}"#,
            r#"{"Fifo":"yes"}"#,
            r#"{"DeadLetterQueue":5}"#,
            r#"{"MaxReceiveCount":0}"#,
        ] {
            assert_eq!(
                code(&format!(
                    r#"{{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{attributes}}}"#
                )),
                ErrorCode::InvalidParameterValue,
                "{attributes}"
            );
        }
    }

    #[test]
    fn null_attributes_count_as_absent() {
        let request = parse(
            br#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"Fifo":null,"Colour":null}}"#,
        )
        .unwrap();
        let Request::CreateQueue { attributes, .. } = request else {
            panic!("parsed as another command: {request:?}");
        };
        assert_eq!(attributes.fifo, None);
    }
}
