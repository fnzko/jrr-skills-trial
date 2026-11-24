//! Error codes and the error type carried by every failed command.

use std::fmt;

/// The closed set of error codes miniq may return (SPEC section 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// A failed command: a code plus a human-readable message.
#[derive(Clone, Debug, PartialEq, Eq)]
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

    pub fn missing_parameter(name: &str) -> Self {
        Self::new(ErrorCode::MissingParameter, format!("`{name}` is required"))
    }

    pub fn invalid_parameter(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParameterValue, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UnsupportedOperation, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
