use std::ops::RangeInclusive;
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, ErrorCode};
use crate::params::Params;
use crate::queue::{QueueKind, Settings};
use crate::types::{GroupId, QueueName};

const VISIBILITY_TIMEOUT: RangeInclusive<u64> = 0..=43_200;
const DELAY: RangeInclusive<u64> = 0..=900;
const RETENTION: RangeInclusive<u64> = 60..=1_209_600;
const MAX_RECEIVE_COUNT: RangeInclusive<u64> = 1..=1000;
const TICK: RangeInclusive<u64> = 0..=1_209_600;
const BODY_BYTES: RangeInclusive<usize> = 1..=1024;
const MAX_MESSAGES: RangeInclusive<u64> = 1..=10;

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

/// The queue attributes a request names; absent ones are `None`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AttributeUpdate {
    pub(crate) visibility_timeout: Option<Duration>,
    pub(crate) delay: Option<Duration>,
    pub(crate) retention: Option<Duration>,
    pub(crate) fifo: Option<bool>,
    pub(crate) content_based_deduplication: Option<bool>,
    dead_letter_queue: Option<String>,
    max_receive_count: Option<u32>,
}

impl AttributeUpdate {
    fn parse(attributes: &Params<'_>) -> Result<Self, Error> {
        if let Some(name) = attributes
            .names()
            .find(|name| !ATTRIBUTE_NAMES.contains(name))
        {
            return Err(Error::invalid(
                "Attributes",
                &format!("has the unknown attribute {name}"),
            ));
        }
        Ok(Self {
            visibility_timeout: attributes.duration("VisibilityTimeout", VISIBILITY_TIMEOUT)?,
            delay: attributes.duration("DelaySeconds", DELAY)?,
            retention: attributes.duration("RetentionSeconds", RETENTION)?,
            fifo: attributes.boolean("Fifo")?,
            content_based_deduplication: attributes.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: attributes.string("DeadLetterQueue")?.map(str::to_owned),
            max_receive_count: attributes
                .integer("MaxReceiveCount", MAX_RECEIVE_COUNT)?
                .map(|n| u32::try_from(n).expect("the range of MaxReceiveCount fits in u32")),
        })
    }

    /// The dead-letter queue name and receive count when both are present,
    /// `None` when both are absent.
    pub(crate) fn dead_letter(&self) -> Result<Option<(&str, u32)>, Error> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(count)) => Ok(Some((queue, count))),
            (None, None) => Ok(None),
            _ => Err(Error::invalid(
                "Attributes",
                "must give DeadLetterQueue and MaxReceiveCount together",
            )),
        }
    }

    /// `base` with every attribute this update names replaced. The queue type
    /// is never changed here; `ContentBasedDeduplication` only applies to a
    /// FIFO base.
    pub(crate) fn settings_over(&self, base: Settings) -> Settings {
        let kind = match (base.kind, self.content_based_deduplication) {
            (QueueKind::Fifo { .. }, Some(content_based_deduplication)) => QueueKind::Fifo {
                content_based_deduplication,
            },
            (kind, _) => kind,
        };
        Settings {
            visibility_timeout: self.visibility_timeout.unwrap_or(base.visibility_timeout),
            delay: self.delay.unwrap_or(base.delay),
            retention: self.retention.unwrap_or(base.retention),
            kind,
        }
    }
}

/// A validated command. Checks that need queue state happen at execution.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Request {
    CreateQueue {
        name: QueueName,
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
        seconds: Duration,
    },
    SendMessage {
        queue: String,
        body: String,
        delay: Option<Duration>,
        group: Option<GroupId>,
        deduplication_id: Option<String>,
    },
    ReceiveMessage {
        queue: String,
        max_messages: usize,
        visibility_timeout: Option<Duration>,
    },
    DeleteMessage {
        queue: String,
        receipt_handle: String,
    },
    ChangeMessageVisibility {
        queue: String,
        receipt_handle: String,
        visibility_timeout: Duration,
    },
}

impl Request {
    pub(crate) fn parse(line: &[u8]) -> Result<Self, Error> {
        let value: Value = serde_json::from_slice(line).map_err(|e| {
            Error::new(
                ErrorCode::InvalidRequest,
                format!("the line is not valid JSON: {e}"),
            )
        })?;
        let Value::Object(members) = &value else {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "the line is not a JSON object",
            ));
        };
        let params = Params::new(members);
        let command = params.command()?;
        let queue = || params.required_string("QueueName").map(str::to_owned);

        match command {
            "CreateQueue" => {
                params.require(&["QueueName"])?;
                let name = QueueName::parse(params.required_string("QueueName")?)?;
                let attributes = match params.object("Attributes")? {
                    Some(attributes) => AttributeUpdate::parse(&attributes)?,
                    None => AttributeUpdate::default(),
                };
                Ok(Self::CreateQueue { name, attributes })
            }
            "SetQueueAttributes" => {
                params.require(&["QueueName", "Attributes"])?;
                let queue = queue()?;
                let attributes = AttributeUpdate::parse(&params.required_object("Attributes")?)?;
                Ok(Self::SetQueueAttributes { queue, attributes })
            }
            "GetQueueAttributes" => {
                params.require(&["QueueName"])?;
                Ok(Self::GetQueueAttributes { queue: queue()? })
            }
            "ListQueues" => Ok(Self::ListQueues {
                prefix: params.string("Prefix")?.map(str::to_owned),
            }),
            "DeleteQueue" => {
                params.require(&["QueueName"])?;
                Ok(Self::DeleteQueue { queue: queue()? })
            }
            "PurgeQueue" => {
                params.require(&["QueueName"])?;
                Ok(Self::PurgeQueue { queue: queue()? })
            }
            "Tick" => {
                params.require(&["Seconds"])?;
                Ok(Self::Tick {
                    seconds: params.required_duration("Seconds", TICK)?,
                })
            }
            "SendMessage" => {
                params.require(&["QueueName", "Body"])?;
                let queue = queue()?;
                let body = params.required_string("Body")?;
                if !BODY_BYTES.contains(&body.len()) {
                    return Err(Error::invalid("Body", "must be 1 to 1024 bytes"));
                }
                Ok(Self::SendMessage {
                    queue,
                    body: body.to_owned(),
                    delay: params.duration("DelaySeconds", DELAY)?,
                    group: params.group_id("GroupId")?,
                    deduplication_id: params.token("DeduplicationId")?.map(str::to_owned),
                })
            }
            "ReceiveMessage" => {
                params.require(&["QueueName"])?;
                let queue = queue()?;
                let max_messages = params.integer("MaxMessages", MAX_MESSAGES)?.unwrap_or(1);
                Ok(Self::ReceiveMessage {
                    queue,
                    max_messages: usize::try_from(max_messages)
                        .expect("the range of MaxMessages fits in usize"),
                    visibility_timeout: params.duration("VisibilityTimeout", VISIBILITY_TIMEOUT)?,
                })
            }
            "DeleteMessage" => {
                params.require(&["QueueName", "ReceiptHandle"])?;
                Ok(Self::DeleteMessage {
                    queue: queue()?,
                    receipt_handle: params.required_string("ReceiptHandle")?.to_owned(),
                })
            }
            "ChangeMessageVisibility" => {
                params.require(&["QueueName", "ReceiptHandle", "VisibilityTimeout"])?;
                let queue = queue()?;
                let receipt_handle = params.required_string("ReceiptHandle")?.to_owned();
                Ok(Self::ChangeMessageVisibility {
                    queue,
                    receipt_handle,
                    visibility_timeout: params
                        .required_duration("VisibilityTimeout", VISIBILITY_TIMEOUT)?,
                })
            }
            other => Err(Error::new(
                ErrorCode::InvalidRequest,
                format!("unknown command {other:?}"),
            )),
        }
    }
}
