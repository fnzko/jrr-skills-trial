//! Conversions from JSON parameter values to typed values. Each one reports
//! InvalidParameterValue, the code section 2 gives a value of the wrong type
//! or outside its range.

use serde_json::Value;

use crate::error::{Error, ErrorCode, Result};

pub fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

/// A JSON integer, written without fraction or exponent. Rejects `30.0`,
/// `3e1` and strings, and negative numbers, which no parameter allows.
pub fn integer(value: &Value, name: &str) -> Result<u64> {
    value
        .as_u64()
        .ok_or_else(|| invalid(format!("{name} must be a non-negative integer")))
}

/// An integer in the inclusive range `min..=max`.
pub fn integer_in(value: &Value, name: &str, min: u64, max: u64) -> Result<u64> {
    let number = integer(value, name)?;
    if (min..=max).contains(&number) {
        Ok(number)
    } else {
        Err(invalid(format!("{name} must be between {min} and {max}")))
    }
}

pub fn boolean(value: &Value, name: &str) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| invalid(format!("{name} must be a boolean")))
}

pub fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| invalid(format!("{name} must be a string")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integer_accepts_only_plain_non_negative_integers() {
        assert_eq!(integer(&json!(30), "n"), Ok(30));
        assert_eq!(integer(&json!(0), "n"), Ok(0));
        for value in [json!(30.0), json!(3e1), json!("30"), json!(-1), json!(true)] {
            assert_eq!(
                integer(&value, "n").unwrap_err().code(),
                ErrorCode::InvalidParameterValue,
                "value {value}"
            );
        }
    }

    #[test]
    fn integer_in_checks_both_bounds_inclusively() {
        assert_eq!(integer_in(&json!(0), "n", 0, 10), Ok(0));
        assert_eq!(integer_in(&json!(10), "n", 0, 10), Ok(10));
        assert_eq!(
            integer_in(&json!(11), "n", 0, 10).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
    }
}
