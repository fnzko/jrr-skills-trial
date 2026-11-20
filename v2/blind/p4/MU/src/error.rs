use std::fmt;

use serde::Serialize;

use crate::broker::MAX_QUEUES;
use crate::queue::MAX_MESSAGES;
use crate::request::MAX_BODY_BYTES;
use crate::types::{QueueName, ReceiptHandle};

/// The error codes of SPEC.md section 2; each serializes as its variant name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorCode {
    InvalidRequest,
    MissingParameter,
    InvalidParameterValue,
    QueueNameInvalid,
    QueueAlreadyExists,
    QueueDoesNotExist,
    UnsupportedOperation,
    ReceiptHandleInvalid,
    ReceiptHandleExpired,
    LimitExceeded,
}

/// Why a command failed. A failed command changes no state.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("request line is not valid UTF-8")]
    NotUtf8,
    #[error("request line is not valid JSON: {0}")]
    Malformed(serde_json::Error),
    #[error("request line is not a JSON object")]
    NotAnObject,
    #[error("request has no string member Command")]
    NoCommand,
    #[error("unknown command {0:?}")]
    UnknownCommand(String),
    #[error("required parameter {0} is absent")]
    MissingParameter(&'static str),
    #[error("{name} must be {expected}")]
    InvalidValue {
        name: &'static str,
        expected: Expected,
    },
    #[error("unknown attribute {0:?}")]
    UnknownAttribute(String),
    #[error("ContentBasedDeduplication requires a FIFO queue")]
    ContentDeduplicationOnStandard,
    #[error("DeadLetterQueue and MaxReceiveCount must be given together")]
    PartialDeadLetterConfig,
    #[error("a queue cannot be its own dead-letter queue")]
    DeadLetterIsSelf,
    #[error("the dead-letter queue must be of the same type (FIFO or standard)")]
    DeadLetterTypeMismatch,
    #[error("{0:?} is not a valid queue name")]
    QueueNameSyntax(String),
    #[error("a queue name ends in .fifo exactly when the queue is FIFO")]
    FifoSuffixMismatch,
    #[error("queue {0} already exists with different attributes")]
    QueueAlreadyExists(QueueName),
    #[error("queue {0:?} does not exist")]
    QueueDoesNotExist(String),
    #[error("the Fifo attribute of a queue cannot change")]
    FifoIsFixed,
    #[error("the limit of {MAX_QUEUES} queues is reached")]
    TooManyQueues,
    #[error("{0} is only valid on a FIFO queue")]
    FifoParameterOnStandard(&'static str),
    #[error("a FIFO queue does not take a per-message DelaySeconds")]
    DelayOnFifo,
    #[error("the queue already holds {MAX_MESSAGES} messages")]
    QueueFull,
    #[error("receipt handle {0:?} was never issued")]
    ReceiptHandleInvalid(String),
    #[error("receipt handle {0} is not the current handle of a message in flight in this queue")]
    ReceiptHandleExpired(ReceiptHandle),
}

impl Error {
    pub fn code(&self) -> ErrorCode {
        match self {
            Error::NotUtf8
            | Error::Malformed(_)
            | Error::NotAnObject
            | Error::NoCommand
            | Error::UnknownCommand(_) => ErrorCode::InvalidRequest,
            Error::MissingParameter(_) => ErrorCode::MissingParameter,
            Error::InvalidValue { .. }
            | Error::UnknownAttribute(_)
            | Error::ContentDeduplicationOnStandard
            | Error::PartialDeadLetterConfig
            | Error::DeadLetterIsSelf
            | Error::DeadLetterTypeMismatch => ErrorCode::InvalidParameterValue,
            Error::QueueNameSyntax(_) | Error::FifoSuffixMismatch => ErrorCode::QueueNameInvalid,
            Error::QueueAlreadyExists(_) => ErrorCode::QueueAlreadyExists,
            Error::QueueDoesNotExist(_) => ErrorCode::QueueDoesNotExist,
            Error::FifoIsFixed | Error::FifoParameterOnStandard(_) | Error::DelayOnFifo => {
                ErrorCode::UnsupportedOperation
            }
            Error::ReceiptHandleInvalid(_) => ErrorCode::ReceiptHandleInvalid,
            Error::ReceiptHandleExpired(_) => ErrorCode::ReceiptHandleExpired,
            Error::TooManyQueues | Error::QueueFull => ErrorCode::LimitExceeded,
        }
    }
}

/// What a parameter value should have looked like, for the error message.
#[derive(Debug)]
pub enum Expected {
    String,
    Boolean,
    Object,
    Integer { min: u32, max: u32 },
    Body,
    Identifier,
}

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expected::String => f.write_str("a string"),
            Expected::Boolean => f.write_str("a boolean"),
            Expected::Object => f.write_str("an object"),
            Expected::Integer { min, max } => write!(f, "an integer from {min} to {max}"),
            Expected::Body => write!(f, "a string of 1 to {MAX_BODY_BYTES} bytes"),
            Expected::Identifier => f.write_str("1 to 128 characters from A-Z a-z 0-9 _ -"),
        }
    }
}
