//! Typed access to command members. The spec treats a member whose value is
//! `null` exactly as if it were absent, so every lookup goes through [`member`].

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

/// The member `name` if present and not `null`.
pub fn member<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    object.get(name).filter(|value| !value.is_null())
}

/// Like [`member`], but an absent member is `MissingParameter`.
pub fn required<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a Value> {
    member(object, name)
        .ok_or_else(|| Error::new(ErrorCode::MissingParameter, format!("{name} is required")))
}

/// A JSON integer (no fraction, no exponent) inside `range`.
///
/// `serde_json` keeps `30` as an integer and reads `30.0` and `3e1` as floats,
/// so `as_u64` rejects the forms the spec forbids; negative numbers and values
/// beyond `u64` fail the same way.
///
/// Known gap: `serde_json` reads a literal `-0` as the float `-0.0`, and a parsed
/// `Value` cannot tell it from `-0.0`, which the spec forbids. So `-0` is
/// rejected even though its value is in range. Fixing that means keeping the
/// number's source text (serde_json's `arbitrary_precision` feature).
pub fn integer(value: &Value, name: &str, range: RangeInclusive<u64>) -> Result<u64> {
    value.as_u64().filter(|n| range.contains(n)).ok_or_else(|| {
        invalid(format!(
            "{name} must be an integer from {} to {}",
            range.start(),
            range.end()
        ))
    })
}

pub fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| invalid(format!("{name} must be a string")))
}

pub fn boolean(value: &Value, name: &str) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| invalid(format!("{name} must be a boolean")))
}

pub fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_member_is_absent() {
        let object = json!({"Prefix": null, "Body": "x"});
        let object = object.as_object().unwrap();
        assert!(member(object, "Prefix").is_none());
        assert!(member(object, "Body").is_some());
        assert_eq!(
            required(object, "Prefix").unwrap_err().code,
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn integers_must_be_plain_json_integers_in_range() {
        let range = 0..=30;
        assert_eq!(integer(&json!(30), "n", range.clone()), Ok(30));
        assert_eq!(integer(&json!(0), "n", range.clone()), Ok(0));
        for bad in [json!(31), json!(-1), json!(30.0), json!("30"), json!(true)] {
            assert_eq!(
                integer(&bad, "n", range.clone()).unwrap_err().code,
                ErrorCode::InvalidParameterValue,
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn exponent_form_is_rejected_as_not_an_integer() {
        // serde_json reads `3e1` as a float, which is what the spec forbids.
        let value: Value = serde_json::from_str("3e1").unwrap();
        assert_eq!(
            integer(&value, "n", 0..=100).unwrap_err().code,
            ErrorCode::InvalidParameterValue
        );
    }
}
