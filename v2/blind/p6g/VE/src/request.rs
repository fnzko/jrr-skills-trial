//! Turns one input line into a validated [`Request`].
//!
//! Checks run in the order of the spec: line shape, then every required
//! parameter, then the value of every present parameter. Rules that need
//! queue state belong to the broker.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::names::{GroupId, QueueName, is_message_token};

/// Attributes named in a `CreateQueue` or `SetQueueAttributes` request. Each
/// one is range-checked; combinations are checked by the broker.
#[derive(Debug, Default)]
pub(crate) struct AttributePatch {
    pub visibility_timeout: Option<u32>,
    pub delay_seconds: Option<u32>,
    pub retention_seconds: Option<u32>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<String>,
    pub max_receive_count: Option<u32>,
}

#[derive(Debug)]
pub(crate) struct NewMessage {
    pub body: String,
    pub delay_seconds: Option<u32>,
    pub group_id: Option<GroupId>,
    pub deduplication_id: Option<String>,
}

#[derive(Debug)]
pub(crate) enum Request {
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
        seconds: u32,
    },
    SendMessage {
        queue: String,
        message: NewMessage,
    },
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

pub(crate) fn parse(line: &[u8]) -> Result<Request, Error> {
    let text = std::str::from_utf8(line).map_err(|e| {
        Error::new(
            ErrorCode::InvalidRequest,
            &format!("line is not valid UTF-8: {e}"),
        )
    })?;
    let value: Value = serde_json::from_str(text).map_err(|e| {
        Error::new(
            ErrorCode::InvalidRequest,
            &format!("line is not valid JSON: {e}"),
        )
    })?;
    let Value::Object(members) = value else {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "line is not a JSON object",
        ));
    };
    let command = members
        .get("Command")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "Command must be a string"))?;
    let params = Params(&members);

    match command {
        "CreateQueue" => create_queue(&params),
        "SetQueueAttributes" => set_queue_attributes(&params),
        "GetQueueAttributes" => Ok(Request::GetQueueAttributes {
            queue: params.queue_name()?,
        }),
        "ListQueues" => {
            let prefix = params
                .optional("Prefix")
                .map(|v| string(v, "Prefix"))
                .transpose()?;
            Ok(Request::ListQueues {
                prefix: prefix.map(str::to_owned),
            })
        }
        "DeleteQueue" => Ok(Request::DeleteQueue {
            queue: params.queue_name()?,
        }),
        "PurgeQueue" => Ok(Request::PurgeQueue {
            queue: params.queue_name()?,
        }),
        "Tick" => {
            let seconds = params.required("Seconds")?;
            Ok(Request::Tick {
                seconds: integer(seconds, "Seconds", 0..=1_209_600)?,
            })
        }
        "SendMessage" => send_message(&params),
        "ReceiveMessage" => receive_message(&params),
        "DeleteMessage" => {
            let (queue, handle) = params.queue_and_handle()?;
            Ok(Request::DeleteMessage {
                queue,
                receipt_handle: handle,
            })
        }
        "ChangeMessageVisibility" => change_message_visibility(&params),
        other => Err(Error::new(
            ErrorCode::InvalidRequest,
            &format!("unknown command {other:?}"),
        )),
    }
}

const VISIBILITY_TIMEOUT: RangeInclusive<u32> = 0..=43_200;
const DELAY_SECONDS: RangeInclusive<u32> = 0..=900;
const RETENTION_SECONDS: RangeInclusive<u32> = 60..=1_209_600;
const GROUP_TOKEN_RULE: &str = "1 to 128 characters from A-Z a-z 0-9 _ -";
const MAX_RECEIVE_COUNT: RangeInclusive<u32> = 1..=1000;

struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    /// The member, treating `null` as absent.
    fn optional(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    fn required(&self, name: &str) -> Result<&'a Value, Error> {
        self.optional(name)
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, &format!("{name} is required")))
    }

    fn queue_name(&self) -> Result<String, Error> {
        string(self.required("QueueName")?, "QueueName").map(str::to_owned)
    }

    fn queue_and_handle(&self) -> Result<(String, String), Error> {
        let queue = self.required("QueueName")?;
        let handle = self.required("ReceiptHandle")?;
        Ok((
            string(queue, "QueueName")?.to_owned(),
            string(handle, "ReceiptHandle")?.to_owned(),
        ))
    }
}

fn invalid(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        &format!("{name} must be {expected}"),
    )
}

fn string<'v>(value: &'v Value, name: &str) -> Result<&'v str, Error> {
    value.as_str().ok_or_else(|| invalid(name, "a string"))
}

fn boolean(value: &Value, name: &str) -> Result<bool, Error> {
    value.as_bool().ok_or_else(|| invalid(name, "a boolean"))
}

/// JSON numbers written with a fraction or exponent are not integers, and
/// `as_u64` rejects them as well as negative numbers.
fn integer(value: &Value, name: &str, range: RangeInclusive<u32>) -> Result<u32, Error> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            invalid(
                name,
                &format!("an integer from {} to {}", range.start(), range.end()),
            )
        })
}

fn token<'v>(value: &'v Value, name: &str) -> Result<&'v str, Error> {
    let text = string(value, name)?;
    if is_message_token(text) {
        Ok(text)
    } else {
        Err(invalid(name, GROUP_TOKEN_RULE))
    }
}

fn attributes(value: &Value) -> Result<AttributePatch, Error> {
    let Value::Object(members) = value else {
        return Err(invalid("Attributes", "an object"));
    };
    let mut patch = AttributePatch::default();
    for (name, value) in members {
        let value = Some(value).filter(|value| !value.is_null());
        match name.as_str() {
            "VisibilityTimeout" => {
                patch.visibility_timeout = value
                    .map(|v| integer(v, name, VISIBILITY_TIMEOUT))
                    .transpose()?;
            }
            "DelaySeconds" => {
                patch.delay_seconds = value.map(|v| integer(v, name, DELAY_SECONDS)).transpose()?;
            }
            "RetentionSeconds" => {
                patch.retention_seconds = value
                    .map(|v| integer(v, name, RETENTION_SECONDS))
                    .transpose()?;
            }
            "Fifo" => patch.fifo = value.map(|v| boolean(v, name)).transpose()?,
            "ContentBasedDeduplication" => {
                patch.content_based_deduplication = value.map(|v| boolean(v, name)).transpose()?;
            }
            "DeadLetterQueue" => {
                patch.dead_letter_queue = value
                    .map(|v| string(v, name).map(str::to_owned))
                    .transpose()?;
            }
            "MaxReceiveCount" => {
                patch.max_receive_count = value
                    .map(|v| integer(v, name, MAX_RECEIVE_COUNT))
                    .transpose()?;
            }
            other => {
                return Err(invalid(
                    "Attributes",
                    &format!("limited to known attributes, not {other:?}"),
                ));
            }
        }
    }
    Ok(patch)
}

fn create_queue(params: &Params) -> Result<Request, Error> {
    let name = params.required("QueueName")?;
    // The name is checked before every other parameter, and by a rule of its own.
    let name = QueueName::parse(string(name, "QueueName")?).ok_or_else(|| {
        Error::new(
            ErrorCode::QueueNameInvalid,
            "queue names are 1 to 80 characters from A-Z a-z 0-9 _ -, optionally followed by .fifo",
        )
    })?;
    let attributes = params.optional("Attributes").map(attributes).transpose()?;
    // Absent attributes mean "all defaults", which an empty patch expresses.
    Ok(Request::CreateQueue {
        name,
        attributes: attributes.unwrap_or_default(),
    })
}

fn set_queue_attributes(params: &Params) -> Result<Request, Error> {
    let queue = params.required("QueueName")?;
    let patch = params.required("Attributes")?;
    Ok(Request::SetQueueAttributes {
        queue: string(queue, "QueueName")?.to_owned(),
        attributes: attributes(patch)?,
    })
}

fn send_message(params: &Params) -> Result<Request, Error> {
    let queue = params.required("QueueName")?;
    let body = params.required("Body")?;

    let queue = string(queue, "QueueName")?.to_owned();
    let body = string(body, "Body")?;
    if !(1..=1024).contains(&body.len()) {
        return Err(invalid("Body", "1 to 1024 bytes of UTF-8"));
    }
    let delay_seconds = params
        .optional("DelaySeconds")
        .map(|v| integer(v, "DelaySeconds", DELAY_SECONDS))
        .transpose()?;
    let group_id = params
        .optional("GroupId")
        .map(|v| {
            GroupId::parse(string(v, "GroupId")?)
                .ok_or_else(|| invalid("GroupId", GROUP_TOKEN_RULE))
        })
        .transpose()?;
    let deduplication_id = params
        .optional("DeduplicationId")
        .map(|v| token(v, "DeduplicationId").map(str::to_owned))
        .transpose()?;

    Ok(Request::SendMessage {
        queue,
        message: NewMessage {
            body: body.to_owned(),
            delay_seconds,
            group_id,
            deduplication_id,
        },
    })
}

fn change_message_visibility(params: &Params) -> Result<Request, Error> {
    let queue = params.required("QueueName")?;
    let handle = params.required("ReceiptHandle")?;
    let timeout = params.required("VisibilityTimeout")?;
    Ok(Request::ChangeMessageVisibility {
        queue: string(queue, "QueueName")?.to_owned(),
        receipt_handle: string(handle, "ReceiptHandle")?.to_owned(),
        visibility_timeout: integer(timeout, "VisibilityTimeout", VISIBILITY_TIMEOUT)?,
    })
}

fn receive_message(params: &Params) -> Result<Request, Error> {
    let queue = params.queue_name()?;
    let max_messages = params
        .optional("MaxMessages")
        .map(|v| integer(v, "MaxMessages", 1..=10))
        .transpose()?
        .unwrap_or(1);
    let visibility_timeout = params
        .optional("VisibilityTimeout")
        .map(|v| integer(v, "VisibilityTimeout", VISIBILITY_TIMEOUT))
        .transpose()?;
    Ok(Request::ReceiveMessage {
        queue,
        max_messages,
        visibility_timeout,
    })
}
