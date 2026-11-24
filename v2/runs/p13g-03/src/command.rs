//! Turns a request object into a typed command, applying the shape,
//! required-parameter and parameter-value checks in the order of the spec.

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::params::Params;
use crate::types::{AttributeUpdate, GroupId, QueueName, Seconds, deduplication_id};

#[derive(Debug)]
pub enum Command<'a> {
    CreateQueue {
        name: QueueName,
        attributes: AttributeUpdate<'a>,
    },
    SetQueueAttributes {
        queue: &'a str,
        attributes: AttributeUpdate<'a>,
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
        seconds: Seconds,
    },
    SendMessage {
        queue: &'a str,
        body: &'a str,
        delay: Option<Seconds>,
        group: Option<GroupId>,
        deduplication_id: Option<&'a str>,
    },
    ReceiveMessage {
        queue: &'a str,
        max_messages: usize,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: &'a str,
        receipt_handle: &'a str,
    },
    ChangeMessageVisibility {
        queue: &'a str,
        receipt_handle: &'a str,
        visibility_timeout: Seconds,
    },
}

impl<'a> Command<'a> {
    pub fn parse(request: &'a Map<String, Value>) -> Result<Self, Error> {
        let Some(Value::String(name)) = request.get("Command") else {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "Command must be a string",
            ));
        };
        let params = Params::new(request);
        match name.as_str() {
            "CreateQueue" => {
                params.require(&["QueueName"])?;
                let name = QueueName::parse(params.required_string("QueueName")?)?;
                let attributes = parse_attributes(&params)?;
                Ok(Self::CreateQueue { name, attributes })
            }
            "SetQueueAttributes" => {
                params.require(&["QueueName", "Attributes"])?;
                let queue = params.required_string("QueueName")?;
                let attributes = AttributeUpdate::parse(params.required_object("Attributes")?)?;
                Ok(Self::SetQueueAttributes { queue, attributes })
            }
            "GetQueueAttributes" => Ok(Self::GetQueueAttributes {
                queue: queue_only(&params)?,
            }),
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params.string("Prefix")?,
            }),
            "DeleteQueue" => Ok(Self::DeleteQueue {
                queue: queue_only(&params)?,
            }),
            "PurgeQueue" => Ok(Self::PurgeQueue {
                queue: queue_only(&params)?,
            }),
            "Tick" => {
                params.require(&["Seconds"])?;
                let seconds = params.required_integer("Seconds", 0..=1_209_600)?;
                Ok(Self::Tick {
                    seconds: Seconds(seconds),
                })
            }
            "SendMessage" => {
                params.require(&["QueueName", "Body"])?;
                let queue = params.required_string("QueueName")?;
                let body = params
                    .parsed_string("Body", "1 to 1024 bytes", |text| {
                        (1..=1024).contains(&text.len()).then_some(text)
                    })?
                    .ok_or_else(|| Error::new(ErrorCode::MissingParameter, "Body is required"))?;
                Ok(Self::SendMessage {
                    queue,
                    body,
                    delay: params.seconds("DelaySeconds", 0..=900)?,
                    group: params.parsed_string(
                        "GroupId",
                        "1 to 128 characters from A-Z a-z 0-9 _ -",
                        GroupId::parse,
                    )?,
                    deduplication_id: params.parsed_string(
                        "DeduplicationId",
                        "1 to 128 characters from A-Z a-z 0-9 _ -",
                        deduplication_id,
                    )?,
                })
            }
            "ReceiveMessage" => {
                params.require(&["QueueName"])?;
                let queue = params.required_string("QueueName")?;
                Ok(Self::ReceiveMessage {
                    queue,
                    max_messages: params.integer("MaxMessages", 1..=10)?.unwrap_or(1),
                    visibility_timeout: params.seconds("VisibilityTimeout", 0..=43_200)?,
                })
            }
            "DeleteMessage" => {
                params.require(&["QueueName", "ReceiptHandle"])?;
                Ok(Self::DeleteMessage {
                    queue: params.required_string("QueueName")?,
                    receipt_handle: params.required_string("ReceiptHandle")?,
                })
            }
            "ChangeMessageVisibility" => {
                params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                Ok(Self::ChangeMessageVisibility {
                    queue: params.required_string("QueueName")?,
                    receipt_handle: params.required_string("ReceiptHandle")?,
                    visibility_timeout: Seconds(
                        params.required_integer("VisibilityTimeout", 0..=43_200)?,
                    ),
                })
            }
            unknown => Err(Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command {unknown:?}"),
            )),
        }
    }
}

fn queue_only<'a>(params: &Params<'a>) -> Result<&'a str, Error> {
    params.require(&["QueueName"])?;
    params.required_string("QueueName")
}

fn parse_attributes<'a>(params: &Params<'a>) -> Result<AttributeUpdate<'a>, Error> {
    match params.object("Attributes")? {
        Some(members) => AttributeUpdate::parse(members),
        None => Ok(AttributeUpdate::default()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse_error(request: Value) -> ErrorCode {
        let Value::Object(members) = &request else {
            panic!("test request must be an object");
        };
        Command::parse(members).unwrap_err().code()
    }

    #[test]
    fn error_precedence_follows_the_spec() {
        let cases = [
            (json!({}), ErrorCode::InvalidRequest),
            (json!({"Command": 5}), ErrorCode::InvalidRequest),
            (json!({"Command": "Nope"}), ErrorCode::InvalidRequest),
            (
                json!({"Command": "SendMessage", "QueueName": 7}),
                ErrorCode::MissingParameter,
            ),
            (
                json!({"Command": "SendMessage", "QueueName": 7, "Body": "x"}),
                ErrorCode::InvalidParameterValue,
            ),
            (
                json!({"Command": "SendMessage", "QueueName": "q", "Body": ""}),
                ErrorCode::InvalidParameterValue,
            ),
            (
                json!({"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}),
                ErrorCode::QueueNameInvalid,
            ),
            (
                json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": 3}),
                ErrorCode::InvalidParameterValue,
            ),
            (
                json!({"Command": "CreateQueue", "QueueName": 3}),
                ErrorCode::InvalidParameterValue,
            ),
        ];
        for (request, expected) in cases {
            assert_eq!(parse_error(request.clone()), expected, "{request}");
        }
    }
}
