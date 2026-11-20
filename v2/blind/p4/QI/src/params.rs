//! Reads parameter values from a parsed command line. Each check names the parameter it failed on
//! so the message points at the right member. A null member is handled by the caller, which
//! treats it as absent before any of these checks run.

use crate::error::{ErrorCode, Failure};
use serde_json::Value;

pub(crate) fn missing(name: &str) -> Failure {
    Failure::new(
        ErrorCode::MissingParameter,
        format!("required parameter {name} is missing"),
    )
}

pub(crate) fn invalid(name: &str, problem: impl std::fmt::Display) -> Failure {
    Failure::new(
        ErrorCode::InvalidParameterValue,
        format!("parameter {name} {problem}"),
    )
}

pub(crate) fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Failure> {
    value
        .as_str()
        .ok_or_else(|| invalid(name, "must be a string"))
}

pub(crate) fn boolean(name: &str, value: &Value) -> Result<bool, Failure> {
    value
        .as_bool()
        .ok_or_else(|| invalid(name, "must be true or false"))
}

/// An integer written without fraction or exponent, in `min..=max`.
pub(crate) fn bounded(name: &str, value: &Value, min: u32, max: u32) -> Result<u32, Failure> {
    value
        .as_i64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| invalid(name, format!("must be an integer from {min} to {max}")))
}
