use std::fmt;

use serde_json::{Value, json};

/// The error codes of the protocol. No other code is ever put on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "InvalidRequest",
            Self::MissingParameter => "MissingParameter",
            Self::InvalidParameterValue => "InvalidParameterValue",
            Self::QueueNameInvalid => "QueueNameInvalid",
            Self::QueueAlreadyExists => "QueueAlreadyExists",
            Self::QueueDoesNotExist => "QueueDoesNotExist",
            Self::ReceiptHandleInvalid => "ReceiptHandleInvalid",
            Self::ReceiptHandleExpired => "ReceiptHandleExpired",
            Self::UnsupportedOperation => "UnsupportedOperation",
            Self::LimitExceeded => "LimitExceeded",
        }
    }
}

/// A failed command: a protocol code plus a message for humans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, Error>;

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

    pub fn invalid_value(parameter: &str, reason: &str) -> Self {
        Self::new(
            ErrorCode::InvalidParameterValue,
            format!("{parameter} {reason}"),
        )
    }

    pub fn queue_missing(name: &str) -> Self {
        Self::new(
            ErrorCode::QueueDoesNotExist,
            format!("queue {name:?} does not exist"),
        )
    }

    /// The failure response line for this error.
    pub fn to_response(&self) -> Value {
        json!({
            "ok": false,
            "error": { "code": self.code.as_str(), "message": self.message },
        })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for Error {}
