//! Typed access to the members of a command object.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};

/// The members of a command. A member whose value is `null` is absent.
pub struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self(members)
    }

    pub fn optional(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    pub fn required(&self, name: &str) -> Result<&'a Value, Error> {
        self.optional(name)
            .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
    }
}

pub fn invalid(name: &str, requirement: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("{name} {requirement}"),
    )
}

pub fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| invalid(name, "must be a string"))
}

pub fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| invalid(name, "must be a boolean"))
}

/// An integer written without fraction or exponent, within `range`.
pub fn integer(name: &str, value: &Value, range: RangeInclusive<u64>) -> Result<u64, Error> {
    // `as_u64` is `None` for negative numbers and for anything written with a
    // fraction or exponent, which are all outside every allowed range.
    value
        .as_u64()
        .filter(|number| range.contains(number))
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
