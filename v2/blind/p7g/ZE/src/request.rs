use serde_json::{Map, Value};

use crate::attributes::RequestedAttributes;
use crate::error::{Error, ErrorCode};
use crate::json::{integer, string};
use crate::types::{
    DeduplicationId, DelaySeconds, GroupId, MaxMessages, QueueName, ReceiptHandle, TickSeconds,
    VisibilityTimeout,
};

/// A command whose parameters have all passed their own type and range checks.
/// Queue names other than the one in `CreateQueue` stay plain strings, since
/// any string can be looked up.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    CreateQueue {
        name: QueueName,
        attributes: RequestedAttributes,
    },
    SetQueueAttributes {
        queue: String,
        attributes: RequestedAttributes,
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
        body: String,
        delay: Option<DelaySeconds>,
        group: Option<GroupId>,
        deduplication_id: Option<DeduplicationId>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: MaxMessages,
        visibility_timeout: Option<VisibilityTimeout>,
    },
    DeleteMessage {
        queue: String,
        /// `None` when the string is not a handle this process could ever issue.
        handle: Option<ReceiptHandle>,
    },
    ChangeMessageVisibility {
        queue: String,
        handle: Option<ReceiptHandle>,
        visibility_timeout: VisibilityTimeout,
    },
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn token<T>(name: &str, value: &Value, parse: fn(&str) -> Option<T>) -> Result<T, Error> {
    parse(string(name, value)?).ok_or_else(|| {
        Error::invalid_value(format!(
            "{name} must be 1 to 128 characters from A-Z a-z 0-9 _ -"
        ))
    })
}

struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    /// A member that is present and not `null`.
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn require(&self, name: &str) -> Result<&'a Value, Error> {
        self.get(name).ok_or_else(|| Error::missing(name))
    }

    fn queue_only(&self) -> Result<String, Error> {
        let queue = self.require("QueueName")?;
        Ok(string("QueueName", queue)?.to_owned())
    }

    fn create_queue(&self) -> Result<Request, Error> {
        let name = string("QueueName", self.require("QueueName")?)?;
        let name = QueueName::parse(name).ok_or_else(|| {
            Error::new(
                ErrorCode::QueueNameInvalid,
                format!("{name:?} is not a valid queue name"),
            )
        })?;
        let attributes = self
            .get("Attributes")
            .map(RequestedAttributes::parse)
            .transpose()?
            .unwrap_or_default();
        Ok(Request::CreateQueue { name, attributes })
    }

    fn set_queue_attributes(&self) -> Result<Request, Error> {
        let (queue, attributes) = (self.require("QueueName")?, self.require("Attributes")?);
        Ok(Request::SetQueueAttributes {
            queue: string("QueueName", queue)?.to_owned(),
            attributes: RequestedAttributes::parse(attributes)?,
        })
    }

    fn list_queues(&self) -> Result<Request, Error> {
        let prefix = self
            .get("Prefix")
            .map(|value| string("Prefix", value).map(str::to_owned))
            .transpose()?;
        Ok(Request::ListQueues { prefix })
    }

    fn tick(&self) -> Result<Request, Error> {
        let seconds = self.require("Seconds")?;
        Ok(Request::Tick {
            seconds: integer("Seconds", seconds)?,
        })
    }

    fn send_message(&self) -> Result<Request, Error> {
        let (queue, body) = (self.require("QueueName")?, self.require("Body")?);
        let queue = string("QueueName", queue)?.to_owned();
        let body = string("Body", body)?;
        if !(1..=1024).contains(&body.len()) {
            return Err(Error::invalid_value("Body must be 1 to 1024 bytes long"));
        }
        Ok(Request::SendMessage {
            queue,
            body: body.to_owned(),
            delay: self
                .get("DelaySeconds")
                .map(|v| integer("DelaySeconds", v))
                .transpose()?,
            group: self
                .get("GroupId")
                .map(|v| token("GroupId", v, GroupId::parse))
                .transpose()?,
            deduplication_id: self
                .get("DeduplicationId")
                .map(|v| token("DeduplicationId", v, DeduplicationId::parse))
                .transpose()?,
        })
    }

    fn receive_message(&self) -> Result<Request, Error> {
        let queue = self.queue_only()?;
        Ok(Request::ReceiveMessage {
            queue,
            max_messages: self
                .get("MaxMessages")
                .map(|v| integer("MaxMessages", v))
                .transpose()?
                .unwrap_or_default(),
            visibility_timeout: self
                .get("VisibilityTimeout")
                .map(|v| integer("VisibilityTimeout", v))
                .transpose()?,
        })
    }

    fn delete_message(&self) -> Result<Request, Error> {
        let (queue, handle) = (self.require("QueueName")?, self.require("ReceiptHandle")?);
        Ok(Request::DeleteMessage {
            queue: string("QueueName", queue)?.to_owned(),
            handle: ReceiptHandle::parse(string("ReceiptHandle", handle)?),
        })
    }

    fn change_message_visibility(&self) -> Result<Request, Error> {
        let (queue, handle, timeout) = (
            self.require("QueueName")?,
            self.require("ReceiptHandle")?,
            self.require("VisibilityTimeout")?,
        );
        Ok(Request::ChangeMessageVisibility {
            queue: string("QueueName", queue)?.to_owned(),
            handle: ReceiptHandle::parse(string("ReceiptHandle", handle)?),
            visibility_timeout: integer("VisibilityTimeout", timeout)?,
        })
    }
}

impl Request {
    /// Parses one input line, applying the check order of the specification:
    /// request shape, required parameters, then parameter values.
    pub fn parse(line: &[u8]) -> Result<Self, Error> {
        let text = std::str::from_utf8(line)
            .map_err(|e| invalid_request(format!("line is not valid UTF-8: {e}")))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|e| invalid_request(format!("line is not valid JSON: {e}")))?;
        let Value::Object(members) = value else {
            return Err(invalid_request("line is not a JSON object"));
        };
        let params = Params(&members);
        let Some(Value::String(command)) = members.get("Command") else {
            return Err(invalid_request("Command is missing or not a string"));
        };
        match command.as_str() {
            "CreateQueue" => params.create_queue(),
            "SetQueueAttributes" => params.set_queue_attributes(),
            "GetQueueAttributes" => Ok(Request::GetQueueAttributes {
                queue: params.queue_only()?,
            }),
            "ListQueues" => params.list_queues(),
            "DeleteQueue" => Ok(Request::DeleteQueue {
                queue: params.queue_only()?,
            }),
            "PurgeQueue" => Ok(Request::PurgeQueue {
                queue: params.queue_only()?,
            }),
            "Tick" => params.tick(),
            "SendMessage" => params.send_message(),
            "ReceiveMessage" => params.receive_message(),
            "DeleteMessage" => params.delete_message(),
            "ChangeMessageVisibility" => params.change_message_visibility(),
            unknown => Err(invalid_request(format!("unknown command {unknown:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Bounded;

    fn code(line: &str) -> ErrorCode {
        Request::parse(line.as_bytes()).unwrap_err().code
    }

    #[test]
    fn rejects_lines_that_are_not_commands() {
        let lines = [
            "",
            "   ",
            "[]",
            "\"x\"",
            "{",
            "{} trailing",
            "{}",
            r#"{"Command": 1}"#,
            r#"{"Command": null}"#,
            r#"{"command": "Tick"}"#,
            r#"{"Command": "Nope"}"#,
            r#"{"Command": "tick", "Seconds": 1}"#,
        ];
        for line in lines {
            assert_eq!(code(line), ErrorCode::InvalidRequest, "{line:?}");
        }
        assert_eq!(
            Request::parse(b"{\"Command\": \"Tick\", \"Seconds\": \xff}")
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn missing_parameters_are_reported_before_bad_values() {
        let cases = [
            (
                r#"{"Command": "SendMessage", "QueueName": 5}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "SendMessage", "Body": "x", "QueueName": null}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "Tick", "Seconds": null}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "SetQueueAttributes", "QueueName": 1}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}"#,
                ErrorCode::MissingParameter,
            ),
            (
                r#"{"Command": "SendMessage", "QueueName": 5, "Body": "x"}"#,
                ErrorCode::InvalidParameterValue,
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(code(line), expected, "{line}");
        }
    }

    #[test]
    fn create_queue_checks_the_name_before_attributes() {
        let cases = [
            (
                r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}"#,
                ErrorCode::QueueNameInvalid,
            ),
            (
                r#"{"Command": "CreateQueue", "QueueName": 3, "Attributes": 3}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (
                r#"{"Command": "CreateQueue", "QueueName": "ok", "Attributes": 3}"#,
                ErrorCode::InvalidParameterValue,
            ),
            (r#"{"Command": "CreateQueue"}"#, ErrorCode::MissingParameter),
        ];
        for (line, expected) in cases {
            assert_eq!(code(line), expected, "{line}");
        }
    }

    #[test]
    fn rejects_values_outside_their_rules() {
        let long_body = format!(
            r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{}"}}"#,
            "x".repeat(1025)
        );
        let cases = [
            r#"{"Command": "Tick", "Seconds": 1209601}"#,
            r#"{"Command": "Tick", "Seconds": 1.0}"#,
            r#"{"Command": "Tick", "Seconds": "1"}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 11}"#,
            r#"{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 43201}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": ""}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "GroupId": "a b"}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DeduplicationId": ""}"#,
            r#"{"Command": "SendMessage", "QueueName": "q", "Body": "x", "DelaySeconds": 901}"#,
            r#"{"Command": "ListQueues", "Prefix": 1}"#,
            r#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": 1}"#,
            long_body.as_str(),
        ];
        for line in cases {
            assert_eq!(code(line), ErrorCode::InvalidParameterValue, "{line}");
        }
    }

    #[test]
    fn counts_body_length_in_utf8_bytes() {
        let body = "\u{e9}".repeat(512);
        let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{body}"}}"#);
        assert!(Request::parse(line.as_bytes()).is_ok());
        let line = format!(r#"{{"Command": "SendMessage", "QueueName": "q", "Body": "{body}x"}}"#);
        assert_eq!(code(&line), ErrorCode::InvalidParameterValue);
    }

    #[test]
    fn parses_a_send_with_every_parameter() {
        let request = Request::parse(
            br#"{"Command": "SendMessage", "QueueName": "q.fifo", "Body": "hi",
                 "GroupId": "g", "DeduplicationId": "d", "Extra": 1, "DelaySeconds": null}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            Request::SendMessage {
                queue: "q.fifo".to_owned(),
                body: "hi".to_owned(),
                delay: None,
                group: GroupId::parse("g"),
                deduplication_id: DeduplicationId::parse("d"),
            }
        );
    }

    #[test]
    fn malformed_receipt_handles_parse_to_none_instead_of_failing() {
        let request = Request::parse(
            br#"{"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-01"}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            Request::DeleteMessage {
                queue: "q".to_owned(),
                handle: None
            }
        );
    }

    #[test]
    fn negative_zero_is_an_integer() {
        let request = Request::parse(br#"{"Command": "Tick", "Seconds": -0}"#).unwrap();
        assert_eq!(
            request,
            Request::Tick {
                seconds: TickSeconds::new(0).unwrap()
            }
        );
    }
}
