//! Decoding one input line into a typed command.

use serde_json::Value;

use crate::attributes::AttributePatch;
use crate::error::{Error, ErrorCode};
use crate::names::{DedupKey, GroupId, QueueName};
use crate::params::{self, Params, invalid};

const MAX_BODY_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRequest {
    pub body: String,
    pub delay: Option<u64>,
    pub group: Option<GroupId>,
    pub dedup: Option<DedupKey>,
}

/// A request that passed the shape and value checks of the protocol. Queue
/// names are plain strings here because lookup, not syntax, decides whether
/// they are good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CreateQueue {
        name: QueueName,
        attributes: AttributePatch,
    },
    SetQueueAttributes {
        queue: String,
        attributes: AttributePatch,
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
        max_messages: u64,
        visibility_timeout: Option<u64>,
    },
    DeleteMessage {
        queue: String,
        handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        handle: String,
        visibility_timeout: u64,
    },
}

fn invalid_request(message: &str) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

/// Decodes one input line (without its line ending).
pub fn parse(line: &[u8]) -> Result<Command, Error> {
    let text = std::str::from_utf8(line).map_err(|_| invalid_request("line is not UTF-8"))?;
    let Value::Object(members) =
        serde_json::from_str(text).map_err(|_| invalid_request("line is not a JSON value"))?
    else {
        return Err(invalid_request("line is not a JSON object"));
    };
    let params = Params::new(&members);
    let Some(Value::String(name)) = members.get("Command") else {
        return Err(invalid_request("Command must be a string"));
    };
    match name.as_str() {
        "CreateQueue" => create_queue(&params),
        "SetQueueAttributes" => set_queue_attributes(&params),
        "GetQueueAttributes" => Ok(Command::GetQueueAttributes {
            queue: queue_name(&params)?,
        }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params
                .optional("Prefix")
                .map(|value| params::string("Prefix", value).map(str::to_owned))
                .transpose()?,
        }),
        "DeleteQueue" => Ok(Command::DeleteQueue {
            queue: queue_name(&params)?,
        }),
        "PurgeQueue" => Ok(Command::PurgeQueue {
            queue: queue_name(&params)?,
        }),
        "Tick" => {
            let seconds = params.required("Seconds")?;
            Ok(Command::Tick {
                seconds: params::integer("Seconds", seconds, 0..=1_209_600)?,
            })
        }
        "SendMessage" => send_message(&params),
        "ReceiveMessage" => receive_message(&params),
        "DeleteMessage" => {
            let (queue, handle) = queue_and_handle(&params)?;
            Ok(Command::DeleteMessage { queue, handle })
        }
        "ChangeMessageVisibility" => change_visibility(&params),
        _ => Err(invalid_request("unknown command")),
    }
}

fn queue_name(params: &Params) -> Result<String, Error> {
    let value = params.required("QueueName")?;
    params::string("QueueName", value).map(str::to_owned)
}

fn create_queue(params: &Params) -> Result<Command, Error> {
    let name = params.required("QueueName")?;
    // The name is checked before every other parameter.
    let name = QueueName::parse(params::string("QueueName", name)?)?;
    let attributes = params
        .optional("Attributes")
        .map(AttributePatch::parse)
        .transpose()?
        .unwrap_or_default();
    Ok(Command::CreateQueue { name, attributes })
}

fn set_queue_attributes(params: &Params) -> Result<Command, Error> {
    let queue = params.required("QueueName")?;
    let attributes = params.required("Attributes")?;
    Ok(Command::SetQueueAttributes {
        queue: params::string("QueueName", queue)?.to_owned(),
        attributes: AttributePatch::parse(attributes)?,
    })
}

fn send_message(params: &Params) -> Result<Command, Error> {
    let queue = params.required("QueueName")?;
    let body = params.required("Body")?;
    let queue = params::string("QueueName", queue)?.to_owned();
    let body = params::string("Body", body)?;
    if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
        return Err(invalid("Body", "must be 1 to 1024 bytes"));
    }
    let delay = params
        .optional("DelaySeconds")
        .map(|value| params::integer("DelaySeconds", value, 0..=900))
        .transpose()?;
    let group = params
        .optional("GroupId")
        .map(|value| {
            let text = params::string("GroupId", value)?;
            GroupId::parse(text).ok_or_else(|| invalid("GroupId", "is not a valid identifier"))
        })
        .transpose()?;
    let dedup = params
        .optional("DeduplicationId")
        .map(|value| {
            let text = params::string("DeduplicationId", value)?;
            DedupKey::from_id(text)
                .ok_or_else(|| invalid("DeduplicationId", "is not a valid identifier"))
        })
        .transpose()?;
    Ok(Command::SendMessage {
        queue,
        request: SendRequest {
            body: body.to_owned(),
            delay,
            group,
            dedup,
        },
    })
}

fn receive_message(params: &Params) -> Result<Command, Error> {
    let queue = queue_name(params)?;
    let max_messages = params
        .optional("MaxMessages")
        .map(|value| params::integer("MaxMessages", value, 1..=10))
        .transpose()?
        .unwrap_or(1);
    let visibility_timeout = params
        .optional("VisibilityTimeout")
        .map(|value| params::integer("VisibilityTimeout", value, 0..=43_200))
        .transpose()?;
    Ok(Command::ReceiveMessage {
        queue,
        max_messages,
        visibility_timeout,
    })
}

fn queue_and_handle(params: &Params) -> Result<(String, String), Error> {
    let queue = params.required("QueueName")?;
    let handle = params.required("ReceiptHandle")?;
    Ok((
        params::string("QueueName", queue)?.to_owned(),
        params::string("ReceiptHandle", handle)?.to_owned(),
    ))
}

fn change_visibility(params: &Params) -> Result<Command, Error> {
    let queue = params.required("QueueName")?;
    let handle = params.required("ReceiptHandle")?;
    let timeout = params.required("VisibilityTimeout")?;
    Ok(Command::ChangeMessageVisibility {
        queue: params::string("QueueName", queue)?.to_owned(),
        handle: params::string("ReceiptHandle", handle)?.to_owned(),
        visibility_timeout: params::integer("VisibilityTimeout", timeout, 0..=43_200)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(line: &str) -> ErrorCode {
        parse(line.as_bytes()).unwrap_err().code()
    }

    #[test]
    fn request_shape_errors_are_invalid_request() {
        let cases = [
            "",
            "   ",
            "not json",
            "[]",
            "\"Tick\"",
            "{}",
            r#"{"Command": 3}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "tick"}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "Tick"} trailing"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\": \"\xff\"}").unwrap_err().code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameters_win_over_bad_values() {
        let cases = [
            r#"{"Command": "Tick"}"#,
            r#"{"Command": "Tick", "Seconds": null}"#,
            r#"{"Command": "SendMessage", "QueueName": 5}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": 5}"#,
            r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": null}"#,
            r#"{"Command": "ChangeMessageVisibility", "QueueName": 5, "ReceiptHandle": 5}"#,
            r#"{"Command": "CreateQueue"}"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::MissingParameter, "{line}");
        }
    }

    #[test]
    fn invalid_values_are_rejected() {
        let cases = [
            r#"{"Command": "Tick", "Seconds": 1.5}"#,
            r#"{"Command": "Tick", "Seconds": 30.0}"#,
            r#"{"Command": "Tick", "Seconds": 3e1}"#,
            r#"{"Command": "Tick", "Seconds": "30"}"#,
            r#"{"Command": "Tick", "Seconds": -1}"#,
            r#"{"Command": "Tick", "Seconds": 1209601}"#,
            r#"{"Command": "ListQueues", "Prefix": 1}"#,
            r#"{"Command": "PurgeQueue", "QueueName": true}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": 4}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a", "DelaySeconds": 901}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a", "GroupId": ""}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "a", "DeduplicationId": "a b"}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 0}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}"#,
            r#"{"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": []}"#,
            r#"{"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Nope": 1}}"#,
            r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": -1}"#,
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidParameterValue, "{line}");
        }
    }

    #[test]
    fn body_limit_counts_bytes_not_characters() {
        let at_limit = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "é".repeat(512)
        );
        assert!(parse(at_limit.as_bytes()).is_ok());
        let over = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}a"}}"#,
            "é".repeat(512)
        );
        assert_eq!(code(&over), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        let line = r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#;
        assert_eq!(code(line), ErrorCode::QueueNameInvalid);
        let wrong_type = r#"{"Command": "CreateQueue", "QueueName": 3}"#;
        assert_eq!(code(wrong_type), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn other_commands_accept_any_queue_name_string() {
        let command = parse(br#"{"Command": "PurgeQueue", "QueueName": "not a name!"}"#).unwrap();
        assert_eq!(
            command,
            Command::PurgeQueue {
                queue: "not a name!".into()
            }
        );
    }

    #[test]
    fn unknown_members_and_nulls_are_ignored() {
        let command = parse(
            br#"{"Command": "ReceiveMessage", "QueueName": "q", "Extra": 1, "MaxMessages": null}"#,
        )
        .unwrap();
        assert_eq!(
            command,
            Command::ReceiveMessage {
                queue: "q".into(),
                max_messages: 1,
                visibility_timeout: None
            }
        );
    }

    #[test]
    fn send_message_carries_all_parameters() {
        let command = parse(
            br#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "hi", "GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 5}"#,
        )
        .unwrap();
        assert_eq!(
            command,
            Command::SendMessage {
                queue: "q.fifo".into(),
                request: SendRequest {
                    body: "hi".into(),
                    delay: Some(5),
                    group: GroupId::parse("g"),
                    dedup: DedupKey::from_id("d"),
                }
            }
        );
    }
}
