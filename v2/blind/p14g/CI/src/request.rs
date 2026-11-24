//! Parsing of one input line into a command whose parameters are validated
//! up to, but not including, the queue lookup.

use serde_json::Value;

use crate::attributes::AttributeSet;
use crate::error::{ApiError, ErrorCode};
use crate::params::Params;
use crate::queue::SendParams;
use crate::types::{DeduplicationId, GroupId, QueueName, QueueRef, Seconds};

#[derive(Debug)]
pub enum Request {
    CreateQueue {
        name: QueueName,
        attributes: AttributeSet,
    },
    SetQueueAttributes {
        queue: QueueRef,
        attributes: AttributeSet,
    },
    GetQueueAttributes {
        queue: QueueRef,
    },
    ListQueues {
        prefix: Option<String>,
    },
    DeleteQueue {
        queue: QueueRef,
    },
    PurgeQueue {
        queue: QueueRef,
    },
    Tick {
        seconds: Seconds,
    },
    SendMessage {
        queue: QueueRef,
        message: SendParams,
    },
    ReceiveMessage {
        queue: QueueRef,
        max_messages: u32,
        visibility_timeout: Option<Seconds>,
    },
    DeleteMessage {
        queue: QueueRef,
        /// Kept as text: whether it was ever issued depends on broker state.
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: QueueRef,
        receipt_handle: String,
        visibility_timeout: Seconds,
    },
}

fn invalid_request(message: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::InvalidRequest, message)
}

fn queue_ref(params: Params<'_>) -> Result<QueueRef, ApiError> {
    Ok(QueueRef::from(params.required_string("QueueName")?))
}

impl Request {
    pub fn parse(line: &[u8]) -> Result<Self, ApiError> {
        let text = std::str::from_utf8(line)
            .map_err(|error| invalid_request(format!("line is not valid UTF-8: {error}")))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|error| invalid_request(format!("line is not valid JSON: {error}")))?;
        let Value::Object(members) = value else {
            return Err(invalid_request("line is not a JSON object"));
        };
        let params = Params::new(&members);
        let command = params
            .string("Command")
            .map_err(|_| invalid_request("Command is not a string"))?
            .ok_or_else(|| invalid_request("Command is missing"))?;

        match command {
            "CreateQueue" => {
                params.require(&["QueueName"])?;
                let name = params.required_string("QueueName")?;
                let name = QueueName::parse(name).ok_or_else(|| {
                    ApiError::new(
                        ErrorCode::QueueNameInvalid,
                        format!("{name:?} breaks the queue naming rules"),
                    )
                })?;
                Ok(Self::CreateQueue {
                    name,
                    attributes: attributes(params)?.unwrap_or_default(),
                })
            }
            "SetQueueAttributes" => {
                params.require(&["QueueName", "Attributes"])?;
                Ok(Self::SetQueueAttributes {
                    queue: queue_ref(params)?,
                    attributes: attributes(params)?
                        .ok_or_else(|| ApiError::missing("Attributes"))?,
                })
            }
            "GetQueueAttributes" => {
                params.require(&["QueueName"])?;
                Ok(Self::GetQueueAttributes {
                    queue: queue_ref(params)?,
                })
            }
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params.string("Prefix")?.map(str::to_owned),
            }),
            "DeleteQueue" => {
                params.require(&["QueueName"])?;
                Ok(Self::DeleteQueue {
                    queue: queue_ref(params)?,
                })
            }
            "PurgeQueue" => {
                params.require(&["QueueName"])?;
                Ok(Self::PurgeQueue {
                    queue: queue_ref(params)?,
                })
            }
            "Tick" => {
                params.require(&["Seconds"])?;
                Ok(Self::Tick {
                    seconds: params.required_seconds("Seconds", 0, 1_209_600)?,
                })
            }
            "SendMessage" => {
                params.require(&["QueueName", "Body"])?;
                let queue = queue_ref(params)?;
                let body = params.required_string("Body")?;
                if !(1..=1024).contains(&body.len()) {
                    return Err(ApiError::invalid_value("Body", "1 to 1024 bytes long"));
                }
                Ok(Self::SendMessage {
                    queue,
                    message: SendParams {
                        body: body.to_owned(),
                        delay: params.seconds("DelaySeconds", 0, 900)?,
                        group: params.parsed_string("GroupId", GroupId::parse)?,
                        deduplication_id: params
                            .parsed_string("DeduplicationId", DeduplicationId::parse)?,
                    },
                })
            }
            "ReceiveMessage" => {
                params.require(&["QueueName"])?;
                let max_messages = params.integer("MaxMessages", 1, 10)?.unwrap_or(1);
                Ok(Self::ReceiveMessage {
                    queue: queue_ref(params)?,
                    max_messages,
                    visibility_timeout: params.seconds("VisibilityTimeout", 0, 43_200)?,
                })
            }
            "DeleteMessage" => {
                params.require(&["QueueName", "ReceiptHandle"])?;
                Ok(Self::DeleteMessage {
                    queue: queue_ref(params)?,
                    receipt_handle: params.required_string("ReceiptHandle")?.to_owned(),
                })
            }
            "ChangeMessageVisibility" => {
                params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                Ok(Self::ChangeMessageVisibility {
                    queue: queue_ref(params)?,
                    receipt_handle: params.required_string("ReceiptHandle")?.to_owned(),
                    visibility_timeout: params.required_seconds("VisibilityTimeout", 0, 43_200)?,
                })
            }
            other => Err(invalid_request(format!("unknown command {other:?}"))),
        }
    }
}

fn attributes(params: Params<'_>) -> Result<Option<AttributeSet>, ApiError> {
    params
        .object("Attributes")?
        .map(|members| AttributeSet::parse(Params::new(members)))
        .transpose()
}
