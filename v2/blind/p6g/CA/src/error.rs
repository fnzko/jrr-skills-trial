//! The failures a command can produce, one variant per protocol error code.

use thiserror::Error;

/// Why a request line is not a command at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RequestFault {
    #[error("the line is not valid UTF-8")]
    NotUtf8,
    #[error("the line is not a JSON document")]
    NotJson,
    #[error("the line is not a JSON object")]
    NotObject,
    #[error("`Command` is absent or not a string")]
    MissingCommand,
    #[error("unknown command")]
    UnknownCommand,
}

/// What is wrong with a parameter that is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Problem {
    #[error("expected {0}")]
    WrongType(&'static str),
    #[error("value is out of range or not allowed")]
    OutOfRange,
    #[error("unknown attribute")]
    UnknownAttribute,
    #[error("{0}")]
    Conflict(&'static str),
}

/// Why a queue name cannot be used by `CreateQueue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NameProblem {
    #[error("a queue name is 1 to 80 characters of A-Z a-z 0-9 _ -, optionally followed by .fifo")]
    Syntax,
    #[error("the `.fifo` suffix and the `Fifo` attribute must agree")]
    FifoMismatch,
}

/// The limit a `LimitExceeded` error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Limit {
    #[error("at most 1000 queues can exist")]
    Queues,
    #[error("a queue holds at most 100000 messages")]
    Messages,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    #[error("invalid request: {0}")]
    InvalidRequest(RequestFault),
    #[error("missing parameter {0}")]
    MissingParameter(&'static str),
    #[error("invalid value for {name}: {problem}")]
    InvalidParameterValue { name: String, problem: Problem },
    #[error("invalid queue name: {0}")]
    QueueNameInvalid(NameProblem),
    #[error("a queue with this name exists with different attributes")]
    QueueAlreadyExists,
    #[error("queue {0:?} does not exist")]
    QueueDoesNotExist(String),
    #[error("the receipt handle was never issued")]
    ReceiptHandleInvalid,
    #[error("the receipt handle is no longer usable")]
    ReceiptHandleExpired,
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(&'static str),
    #[error("limit exceeded: {0}")]
    LimitExceeded(Limit),
}

impl Error {
    /// The protocol error code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Error::InvalidRequest(_) => "InvalidRequest",
            Error::MissingParameter(_) => "MissingParameter",
            Error::InvalidParameterValue { .. } => "InvalidParameterValue",
            Error::QueueNameInvalid(_) => "QueueNameInvalid",
            Error::QueueAlreadyExists => "QueueAlreadyExists",
            Error::QueueDoesNotExist(_) => "QueueDoesNotExist",
            Error::ReceiptHandleInvalid => "ReceiptHandleInvalid",
            Error::ReceiptHandleExpired => "ReceiptHandleExpired",
            Error::UnsupportedOperation(_) => "UnsupportedOperation",
            Error::LimitExceeded(_) => "LimitExceeded",
        }
    }

    pub fn invalid(name: &str, problem: Problem) -> Self {
        Error::InvalidParameterValue {
            name: name.to_owned(),
            problem,
        }
    }
}
