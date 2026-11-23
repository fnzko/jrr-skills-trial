//! Typed access to the members of a command object.
//!
//! A member whose value is `null` counts as absent. Presence is checked for
//! all required members first (`require`), then each value is checked.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::types::Seconds;

pub struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self(members)
    }

    pub fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|v| !v.is_null())
    }

    /// Fails with `MissingParameter` for the first of `names` that is absent.
    pub fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::new(
                ErrorCode::MissingParameter,
                format!("{name} is required"),
            )),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        self.get(name).map(|v| as_str(v, name)).transpose()
    }

    pub fn integer(&self, name: &str, range: RangeInclusive<i64>) -> Result<Option<i64>> {
        self.get(name)
            .map(|v| as_integer(v, name, range))
            .transpose()
    }

    pub fn seconds(&self, name: &str, range: RangeInclusive<u32>) -> Result<Option<Seconds>> {
        self.get(name)
            .map(|v| as_seconds(v, name, range))
            .transpose()
    }

    pub fn object(&self, name: &str) -> Result<Option<&'a Map<String, Value>>> {
        self.get(name)
            .map(|v| v.as_object().ok_or_else(|| wrong_type(name, "an object")))
            .transpose()
    }

    /// A string member that `require` has already shown to be present.
    pub fn required_string(&self, name: &str) -> Result<&'a str> {
        self.string(name)?.ok_or_else(|| missing(name))
    }

    /// A seconds member that `require` has already shown to be present.
    pub fn required_seconds(&self, name: &str, range: RangeInclusive<u32>) -> Result<Seconds> {
        self.seconds(name, range)?.ok_or_else(|| missing(name))
    }
}

fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParameter, format!("{name} is required"))
}

fn wrong_type(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("{name} must be {expected}"),
    )
}

pub fn as_str<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value.as_str().ok_or_else(|| wrong_type(name, "a string"))
}

pub fn as_bool(value: &Value, name: &str) -> Result<bool> {
    value.as_bool().ok_or_else(|| wrong_type(name, "a boolean"))
}

/// Accepts only numbers written as plain integers (`30`, not `30.0` or `3e1`).
/// Integers too large for `i64` saturate, which is out of every range anyway.
pub fn as_integer(value: &Value, name: &str, range: RangeInclusive<i64>) -> Result<i64> {
    let Value::Number(number) = value else {
        return Err(wrong_type(name, "an integer"));
    };
    let text = number.to_string();
    let digits = text.strip_prefix('-').unwrap_or(&text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(wrong_type(name, "an integer"));
    }
    let parsed = text.parse::<i64>().unwrap_or(if text.starts_with('-') {
        i64::MIN
    } else {
        i64::MAX
    });
    if range.contains(&parsed) {
        Ok(parsed)
    } else {
        Err(Error::new(
            ErrorCode::InvalidParameterValue,
            format!(
                "{name} must be between {} and {}",
                range.start(),
                range.end()
            ),
        ))
    }
}

/// An integer in `range`, as a number of seconds.
pub fn as_seconds(value: &Value, name: &str, range: RangeInclusive<u32>) -> Result<Seconds> {
    let bounds = i64::from(*range.start())..=i64::from(*range.end());
    let n = as_integer(value, name, bounds)?;
    // The bounds are non-negative, so `unsigned_abs` does not change the value.
    Ok(Seconds::new(n.unsigned_abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        let accepted = ["30", "0", "-0", "-5"];
        for text in accepted {
            let value: Value = serde_json::from_str(text).unwrap();
            let expected: i64 = text.parse().unwrap();
            assert_eq!(as_integer(&value, "X", -10..=100), Ok(expected), "{text}");
        }
        let rejected = ["30.0", "3e1", "3E1", "1.5", "\"30\"", "true", "[3]"];
        for text in rejected {
            let value: Value = serde_json::from_str(text).unwrap();
            let err = as_integer(&value, "X", 0..=100).unwrap_err();
            assert_eq!(err.code(), ErrorCode::InvalidParameterValue, "{text}");
        }
    }

    #[test]
    fn integers_outside_the_range_are_rejected() {
        for text in ["101", "-1", "123456789012345678901234567890"] {
            let value: Value = serde_json::from_str(text).unwrap();
            let err = as_integer(&value, "X", 0..=100).unwrap_err();
            assert_eq!(err.code(), ErrorCode::InvalidParameterValue, "{text}");
        }
    }

    #[test]
    fn null_members_are_absent() {
        let members = json!({"A": null, "B": 1});
        let params = Params::new(members.as_object().unwrap());
        assert_eq!(
            params.require(&["A"]).unwrap_err().code(),
            ErrorCode::MissingParameter
        );
        assert_eq!(params.require(&["B"]), Ok(()));
    }
}
