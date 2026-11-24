//! Queue attributes as given in a request.

use crate::error::{ApiError, ErrorCode};
use crate::params::Params;
use crate::types::{QueueRef, Seconds};

const KNOWN: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

/// The attributes present in a request; absent ones stay `None`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttributeSet {
    pub visibility_timeout: Option<Seconds>,
    pub delay_seconds: Option<Seconds>,
    pub retention_seconds: Option<Seconds>,
    pub fifo: Option<bool>,
    pub content_based_deduplication: Option<bool>,
    pub dead_letter_queue: Option<QueueRef>,
    pub max_receive_count: Option<u32>,
}

impl AttributeSet {
    pub fn parse(params: Params<'_>) -> Result<Self, ApiError> {
        if let Some(name) = params.names().find(|name| !KNOWN.contains(name)) {
            return Err(ApiError::new(
                ErrorCode::InvalidParameterValue,
                format!("unknown attribute {name}"),
            ));
        }
        Ok(Self {
            visibility_timeout: params.seconds("VisibilityTimeout", 0, 43_200)?,
            delay_seconds: params.seconds("DelaySeconds", 0, 900)?,
            retention_seconds: params.seconds("RetentionSeconds", 60, 1_209_600)?,
            fifo: params.boolean("Fifo")?,
            content_based_deduplication: params.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: params.string("DeadLetterQueue")?.map(QueueRef::from),
            max_receive_count: params.integer("MaxReceiveCount", 1, 1000)?,
        })
    }

    /// The dead-letter configuration, which is either complete or absent.
    pub fn dead_letter(&self) -> Result<Option<(&QueueRef, u32)>, ApiError> {
        match (&self.dead_letter_queue, self.max_receive_count) {
            (Some(queue), Some(max)) => Ok(Some((queue, max))),
            (None, None) => Ok(None),
            _ => Err(ApiError::new(
                ErrorCode::InvalidParameterValue,
                "DeadLetterQueue and MaxReceiveCount must be given together",
            )),
        }
    }
}
