//! Turning a request line into a validated [`Command`].
//!
//! Parsing implements checks 1 to 3 of the spec's error order: request shape,
//! required parameters, then the value of each parameter. Everything that needs
//! queue state happens later, in the broker.

use serde_json::Value;

use crate::attributes::AttributeUpdate;
use crate::error::{Error, ErrorCode, Result};
use crate::params::Params;

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
        seconds: u32,
    },
    SendMessage(SendRequest),
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
pub struct SendRequest {
    pub queue: String,
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
}

const MAX_VISIBILITY_TIMEOUT: u32 = 43_200;
const MAX_BODY_BYTES: usize = 1024;

/// Parses one input line.
pub fn parse(line: &[u8]) -> Result<Command> {
    let text =
        std::str::from_utf8(line).map_err(|_| Error::invalid_request("line is not valid UTF-8"))?;
    let Value::Object(members) = serde_json::from_str(text)
        .map_err(|e| Error::invalid_request(format!("line is not valid JSON: {e}")))?
    else {
        return Err(Error::invalid_request("line is not a JSON object"));
    };
    let params = Params::new(&members);
    let Some(Value::String(name)) = members.get("Command") else {
        return Err(Error::invalid_request("Command must be a string"));
    };
    match name.as_str() {
        "CreateQueue" => create_queue(params),
        "SetQueueAttributes" => set_queue_attributes(params),
        "GetQueueAttributes" => queue_only(params, |queue| Command::GetQueueAttributes { queue }),
        "ListQueues" => Ok(Command::ListQueues {
            prefix: params.string("Prefix")?.map(str::to_owned),
        }),
        "DeleteQueue" => queue_only(params, |queue| Command::DeleteQueue { queue }),
        "PurgeQueue" => queue_only(params, |queue| Command::PurgeQueue { queue }),
        "Tick" => {
            params.require(&["Seconds"])?;
            Ok(Command::Tick {
                seconds: params.required_integer("Seconds", 0..=1_209_600)?,
            })
        }
        "SendMessage" => send_message(params),
        "ReceiveMessage" => receive_message(params),
        "DeleteMessage" => {
            params.require(&["QueueName", "ReceiptHandle"])?;
            Ok(Command::DeleteMessage {
                queue: params.required_string("QueueName")?.to_owned(),
                receipt_handle: params.required_string("ReceiptHandle")?.to_owned(),
            })
        }
        "ChangeMessageVisibility" => {
            params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
            Ok(Command::ChangeMessageVisibility {
                queue: params.required_string("QueueName")?.to_owned(),
                receipt_handle: params.required_string("ReceiptHandle")?.to_owned(),
                visibility_timeout: params
                    .required_integer("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?,
            })
        }
        other => Err(Error::invalid_request(format!("unknown command {other:?}"))),
    }
}

fn queue_only(params: Params<'_>, build: fn(String) -> Command) -> Result<Command> {
    params.require(&["QueueName"])?;
    Ok(build(params.required_string("QueueName")?.to_owned()))
}

fn create_queue(params: Params<'_>) -> Result<Command> {
    params.require(&["QueueName"])?;
    let name = params.required_string("QueueName")?;
    if !is_valid_queue_name(name) {
        return Err(Error::new(
            ErrorCode::QueueNameInvalid,
            format!("{name:?} is not a valid queue name"),
        ));
    }
    let attributes = match params.object("Attributes")? {
        Some(members) => AttributeUpdate::parse(Params::new(members))?,
        None => AttributeUpdate::default(),
    };
    Ok(Command::CreateQueue {
        name: name.to_owned(),
        attributes,
    })
}

fn set_queue_attributes(params: Params<'_>) -> Result<Command> {
    params.require(&["QueueName", "Attributes"])?;
    let queue = params.required_string("QueueName")?.to_owned();
    let members = params
        .object("Attributes")?
        .ok_or_else(|| Error::missing("Attributes"))?;
    Ok(Command::SetQueueAttributes {
        queue,
        attributes: AttributeUpdate::parse(Params::new(members))?,
    })
}

fn send_message(params: Params<'_>) -> Result<Command> {
    params.require(&["QueueName", "Body"])?;
    let queue = params.required_string("QueueName")?.to_owned();
    let body = params.required_string("Body")?;
    if !(1..=MAX_BODY_BYTES).contains(&body.len()) {
        return Err(Error::invalid_value(
            "Body",
            "must be 1 to 1024 bytes of UTF-8",
        ));
    }
    Ok(Command::SendMessage(SendRequest {
        queue,
        body: body.to_owned(),
        delay_seconds: params.integer("DelaySeconds", 0..=900)?,
        group_id: message_id_string(params, "GroupId")?,
        deduplication_id: message_id_string(params, "DeduplicationId")?,
    }))
}

fn message_id_string(params: Params<'_>, name: &str) -> Result<Option<String>> {
    let Some(value) = params.string(name)? else {
        return Ok(None);
    };
    if (1..=128).contains(&value.len()) && value.chars().all(is_name_char) {
        Ok(Some(value.to_owned()))
    } else {
        Err(Error::invalid_value(
            name,
            "must be 1 to 128 characters from A-Z a-z 0-9 _ -",
        ))
    }
}

fn receive_message(params: Params<'_>) -> Result<Command> {
    params.require(&["QueueName"])?;
    Ok(Command::ReceiveMessage {
        queue: params.required_string("QueueName")?.to_owned(),
        max_messages: params.integer("MaxMessages", 1..=10)?.unwrap_or(1),
        visibility_timeout: params.integer("VisibilityTimeout", 0..=MAX_VISIBILITY_TIMEOUT)?,
    })
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// 1 to 80 characters of `A-Z a-z 0-9 _ -`, or 1 to 75 of them plus `.fifo`.
fn is_valid_queue_name(name: &str) -> bool {
    let (stem, max_len) = match name.strip_suffix(".fifo") {
        Some(stem) => (stem, 75),
        None => (name, 80),
    };
    (1..=max_len).contains(&stem.len()) && stem.chars().all(is_name_char)
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
        let longest_fifo = format!("{}.fifo", "a".repeat(75));
        let too_long_fifo = format!("{}.fifo", "a".repeat(76));
        for ok in ["a", "A_z-9", "x.fifo", long.as_str(), longest_fifo.as_str()] {
            assert!(is_valid_queue_name(ok), "{ok:?}");
        }
        for bad in [
            "",
            ".fifo",
            "a.b",
            "a b",
            "é",
            "a.fifo.fifo",
            too_long_fifo.as_str(),
            &"a".repeat(81),
        ] {
            assert!(!is_valid_queue_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn request_shape_errors() {
        let bad_lines = [
            "",
            "   ",
            "[]",
            "\"Tick\"",
            "{",
            "{} trailing",
            "{}",
            r#"{"Command": 5}"#,
            r#"{"Command": null}"#,
            r#"{"Command": "tick", "Seconds": 1}"#,
            r#"{"Command": "Nope"}"#,
        ];
        for line in bad_lines {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            parse(b"{\"Command\":\"Tick\",\"Seconds\":1}\xff")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_wins_over_invalid_values() {
        assert_eq!(
            code(r#"{"Command": "SendMessage", "QueueName": 5}"#),
            ErrorCode::MissingParameter
        );
        assert_eq!(
            code(r#"{"Command": "SendMessage", "QueueName": 5, "Body": "x"}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "PurgeQueue", "QueueName": null}"#),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn integers_must_be_plain_in_range_integers() {
        for seconds in ["30.0", "3e1", "\"30\"", "-1", "1209601", "true", "[]"] {
            let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
            assert_eq!(code(&line), ErrorCode::InvalidParameterValue, "{seconds}");
        }
        assert_eq!(
            parse(br#"{"Command": "Tick", "Seconds": 1209600}"#),
            Ok(Command::Tick { seconds: 1_209_600 })
        );
    }

    #[test]
    fn create_queue_checks_the_name_before_other_parameters() {
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#),
            ErrorCode::QueueNameInvalid
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": 3, "Attributes": {}}"#),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            code(r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Bogus": 1}}"#),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn null_attributes_are_absent() {
        let command = parse(
            br#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": null, "Bogus": null, "DelaySeconds": 5}}"#,
        )
        .unwrap();
        let Command::CreateQueue { attributes, .. } = command else {
            panic!("wrong command");
        };
        assert_eq!(
            attributes,
            AttributeUpdate {
                delay_seconds: Some(5),
                ..AttributeUpdate::default()
            }
        );
    }

    #[test]
    fn send_message_parameter_limits() {
        let long_body = "x".repeat(1025);
        let two_byte_chars = "é".repeat(512);
        let send =
            |extra: &str| format!(r#"{{"Command": "SendMessage", "QueueName": "q", {extra}}}"#);
        let accepted = |body: &str| {
            Ok(Command::SendMessage(SendRequest {
                queue: "q".into(),
                body: body.into(),
                delay_seconds: None,
                group_id: None,
                deduplication_id: None,
            }))
        };
        assert_eq!(parse(send(r#""Body": "é""#).as_bytes()), accepted("é"));
        assert_eq!(
            parse(send(&format!(r#""Body": "{two_byte_chars}""#)).as_bytes()),
            accepted(&two_byte_chars)
        );
        for bad in [
            r#""Body": """#.to_owned(),
            format!(r#""Body": "{long_body}""#),
            format!(r#""Body": "{two_byte_chars}x""#),
            r#""Body": "x", "GroupId": """#.to_owned(),
            r#""Body": "x", "GroupId": "a b""#.to_owned(),
            r#""Body": "x", "DeduplicationId": 1"#.to_owned(),
            format!(r#""Body": "x", "DeduplicationId": "{}""#, "a".repeat(129)),
            r#""Body": "x", "DelaySeconds": 901"#.to_owned(),
        ] {
            assert_eq!(code(&send(&bad)), ErrorCode::InvalidParameterValue, "{bad}");
        }
    }
}
