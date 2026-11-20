//! Command failures and the wire codes they map to.

use serde::Serialize;

use crate::types::{QueueName, ReceiptHandle};

/// The `code` member of a failure response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorCode {
    InvalidRequest,
    MissingParameter,
    InvalidParameterValue,
    QueueNameInvalid,
    QueueAlreadyExists,
    QueueDoesNotExist,
    ReceiptHandleInvalid,
    ReceiptHandleExpired,
    UnsupportedOperation,
    LimitExceeded,
}

/// Why a command failed. A failed command changes no state.
///
/// The `Display` text becomes the `message` of the failure response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the request line is not valid UTF-8")]
    NotUtf8,
    #[error("the request line is not a JSON object")]
    NotAnObject,
    #[error("the request has no string member `Command`")]
    NoCommand,
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("required parameter `{0}` is missing")]
    MissingParameter(&'static str),
    #[error("parameter `{parameter}` {rule}")]
    InvalidParameterValue {
        parameter: &'static str,
        rule: &'static str,
    },
    #[error("`{0}` is not a queue attribute")]
    UnknownAttribute(String),
    #[error("queue name {rule}")]
    QueueNameInvalid { rule: &'static str },
    #[error("queue `{0}` already exists with different attributes")]
    QueueAlreadyExists(QueueName),
    #[error("queue `{0}` does not exist")]
    QueueDoesNotExist(String),
    #[error("receipt handle `{0}` was never issued")]
    ReceiptHandleInvalid(String),
    #[error("receipt handle `{0}` does not belong to a message in flight in this queue")]
    ReceiptHandleExpired(ReceiptHandle),
    #[error("{0}")]
    UnsupportedOperation(&'static str),
    #[error("cannot add more: {0}")]
    LimitExceeded(&'static str),
}

impl Error {
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotUtf8 | Self::NotAnObject | Self::NoCommand | Self::UnknownCommand(_) => {
                ErrorCode::InvalidRequest
            }
            Self::MissingParameter(_) => ErrorCode::MissingParameter,
            Self::InvalidParameterValue { .. } | Self::UnknownAttribute(_) => {
                ErrorCode::InvalidParameterValue
            }
            Self::QueueNameInvalid { .. } => ErrorCode::QueueNameInvalid,
            Self::QueueAlreadyExists(_) => ErrorCode::QueueAlreadyExists,
            Self::QueueDoesNotExist(_) => ErrorCode::QueueDoesNotExist,
            Self::ReceiptHandleInvalid(_) => ErrorCode::ReceiptHandleInvalid,
            Self::ReceiptHandleExpired(_) => ErrorCode::ReceiptHandleExpired,
            Self::UnsupportedOperation(_) => ErrorCode::UnsupportedOperation,
            Self::LimitExceeded(_) => ErrorCode::LimitExceeded,
        }
    }
}
