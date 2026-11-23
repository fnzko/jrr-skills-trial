//! Error codes of the protocol and the error type shared by every layer.

use std::fmt;

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

/// A failed command: the code is part of the protocol, the message is for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    code: ErrorCode,
    message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
