//! Error codes of the protocol (SPEC section 2).

use std::fmt;

/// The closed set of error codes miniq may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
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

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::InvalidRequest => "InvalidRequest",
            Code::MissingParameter => "MissingParameter",
            Code::InvalidParameterValue => "InvalidParameterValue",
            Code::QueueNameInvalid => "QueueNameInvalid",
            Code::QueueAlreadyExists => "QueueAlreadyExists",
            Code::QueueDoesNotExist => "QueueDoesNotExist",
            Code::ReceiptHandleInvalid => "ReceiptHandleInvalid",
            Code::ReceiptHandleExpired => "ReceiptHandleExpired",
            Code::UnsupportedOperation => "UnsupportedOperation",
            Code::LimitExceeded => "LimitExceeded",
        }
    }
}

/// A failed command: a code plus a human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: Code,
    pub message: String,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
