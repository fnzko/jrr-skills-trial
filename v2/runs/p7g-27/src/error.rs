//! Error codes of the protocol and the error value commands fail with.

use std::fmt;

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

/// A failed command: the protocol code and a message for humans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    code: Code,
    message: String,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn missing(param: &str) -> Self {
        Self::new(Code::MissingParameter, format!("{param} is required"))
    }

    pub fn invalid_value(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidParameterValue, message)
    }

    pub fn code(&self) -> Code {
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
