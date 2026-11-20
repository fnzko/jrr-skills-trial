//! Access to the members of a command object and checks of single values.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};

/// The members of a command object. A `null` member counts as absent.
pub struct Params(Map<String, Value>);

impl Params {
    pub fn new(members: Map<String, Value>) -> Self {
        Self(members)
    }

    /// The named member, or `None` when it is absent or `null`.
    pub fn optional(&self, name: &str) -> Option<&Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// The named members, or `MissingParameter` when any is absent.
    ///
    /// Call this before checking any value: a missing parameter outranks an
    /// invalid one.
    pub fn required<const N: usize>(&self, names: [&str; N]) -> Result<[&Value; N], Error> {
        let mut found = [&Value::Null; N];
        for (slot, name) in found.iter_mut().zip(names) {
            *slot = self.optional(name).ok_or_else(|| {
                Error::new(ErrorCode::MissingParameter, format!("`{name}` is required"))
            })?;
        }
        Ok(found)
    }
}

pub fn invalid(name: &str, requirement: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("`{name}` {requirement}"),
    )
}

pub fn string(name: &str, value: &Value) -> Result<String, Error> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            Err(invalid(name, "must be a string"))
        }
    }
}

pub fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| invalid(name, "must be a boolean"))
}

/// An integer written without fraction or exponent, within `range`.
pub fn integer(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<u64, Error> {
    let Value::Number(number) = value else {
        return Err(invalid(name, "must be an integer"));
    };
    // With arbitrary_precision the text is exactly what the client wrote, so
    // `30.0` and `3e1` fail to parse as integers.
    number
        .to_string()
        .parse::<i64>()
        .ok()
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            invalid(
                name,
                &format!(
                    "must be an integer from {} to {}",
                    range.start(),
                    range.end()
                ),
            )
        })
}
