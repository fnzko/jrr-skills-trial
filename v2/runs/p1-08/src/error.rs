//! Failures that are reported to the client.

use serde::Serialize;

/// The error codes of the protocol. No other code is ever returned.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    pub fn missing(parameter: &str) -> Self {
        Self::new(
            ErrorCode::MissingParameter,
            format!("{parameter} is required"),
        )
    }

    pub fn invalid_value(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParameterValue, message)
    }
}
