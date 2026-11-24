//! The failure type of every command, with the wire code each variant maps to.

use std::str::Utf8Error;

use serde::Serialize;
use thiserror::Error;

/// The error codes of the protocol. No other code is ever sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid request: {0}")]
    InvalidRequest(#[from] ShapeProblem),
    #[error("missing required parameter {name}")]
    MissingParameter { name: &'static str },
    #[error("invalid value for {parameter}: {problem}")]
    InvalidParameterValue { parameter: String, problem: Problem },
    #[error("invalid queue name {name:?}: {problem}")]
    QueueNameInvalid { name: String, problem: NameProblem },
    #[error("queue {name:?} already exists with different attributes")]
    QueueAlreadyExists { name: String },
    #[error("queue {name:?} does not exist")]
    QueueDoesNotExist { name: String },
    #[error("receipt handle {handle:?} was never issued")]
    ReceiptHandleInvalid { handle: String },
    #[error("receipt handle {handle:?} is no longer usable")]
    ReceiptHandleExpired { handle: String },
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(Unsupported),
    #[error("limit exceeded: {0}")]
    LimitExceeded(Limit),
}

impl Error {
    pub fn code(&self) -> Code {
        match self {
            Self::InvalidRequest(_) => Code::InvalidRequest,
            Self::MissingParameter { .. } => Code::MissingParameter,
            Self::InvalidParameterValue { .. } => Code::InvalidParameterValue,
            Self::QueueNameInvalid { .. } => Code::QueueNameInvalid,
            Self::QueueAlreadyExists { .. } => Code::QueueAlreadyExists,
            Self::QueueDoesNotExist { .. } => Code::QueueDoesNotExist,
            Self::ReceiptHandleInvalid { .. } => Code::ReceiptHandleInvalid,
            Self::ReceiptHandleExpired { .. } => Code::ReceiptHandleExpired,
            Self::UnsupportedOperation(_) => Code::UnsupportedOperation,
            Self::LimitExceeded(_) => Code::LimitExceeded,
        }
    }

    pub fn invalid_value(parameter: &str, problem: Problem) -> Self {
        Self::InvalidParameterValue {
            parameter: parameter.to_owned(),
            problem,
        }
    }
}

/// Why a line is not a command at all.
#[derive(Debug, Error)]
pub enum ShapeProblem {
    #[error("line is not valid UTF-8")]
    NotUtf8(#[source] Utf8Error),
    #[error("line is not valid JSON")]
    NotJson(#[source] serde_json::Error),
    #[error("line is not a JSON object")]
    NotAnObject,
    #[error("no string member Command")]
    MissingCommand,
    #[error("unknown command {0:?}")]
    UnknownCommand(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Problem {
    #[error("expected {expected}")]
    WrongType { expected: &'static str },
    #[error("must be between {min} and {max}")]
    OutOfRange { min: i64, max: i64 },
    #[error("length must be between {min} and {max}")]
    Length { min: usize, max: usize },
    #[error("only the characters A-Z a-z 0-9 _ - are allowed")]
    Characters,
    #[error("not an attribute of a queue")]
    UnknownAttribute,
    #[error("only a FIFO queue can use content based deduplication")]
    NeedsFifoQueue,
    #[error("DeadLetterQueue and MaxReceiveCount must be given together")]
    IncompleteDeadLetter,
    #[error("a queue cannot be its own dead-letter queue")]
    DeadLetterIsSelf,
    #[error("a dead-letter queue must have the same Fifo setting as the queue it serves")]
    DeadLetterKindMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NameProblem {
    #[error("must be 1 to 80 characters of A-Z a-z 0-9 _ -, or 1 to 75 of them plus .fifo")]
    Syntax,
    #[error("the .fifo suffix and the Fifo attribute must agree")]
    FifoMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Unsupported {
    #[error("GroupId and DeduplicationId need a FIFO queue")]
    FifoOnlyParameter,
    #[error("a FIFO queue does not take a per-message delay")]
    DelayOnFifo,
    #[error("the type of a queue cannot change")]
    FifoChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Limit {
    #[error("at most 1000 queues")]
    Queues,
    #[error("at most 100000 messages in one queue")]
    Messages,
}
