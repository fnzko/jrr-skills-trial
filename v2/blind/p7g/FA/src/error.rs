//! The error codes of the protocol and the error value carried to the wire.

use std::fmt;

use serde::Serialize;

/// Every code a failure response can carry.
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

/// A failed command: the code decides the client's reaction, the message is for people.
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

    pub fn missing(parameter: &str) -> Self {
        Self::new(
            ErrorCode::MissingParameter,
            format!("parameter {parameter} is required"),
        )
    }

    pub fn invalid_value(parameter: &str, rule: &str) -> Self {
        Self::new(
            ErrorCode::InvalidParameterValue,
            format!("parameter {parameter} {rule}"),
        )
    }

    pub fn queue_missing(name: &str) -> Self {
        Self::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name:?} does not exist"),
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
