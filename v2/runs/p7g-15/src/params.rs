//! Typed access to the members of a request object.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::Error;

/// The members of one request. A `null` member counts as absent.
pub struct Params<'a>(pub &'a Map<String, Value>);

impl<'a> Params<'a> {
    pub fn optional(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    pub fn required(&self, name: &str) -> Result<&'a Value, Error> {
        self.optional(name).ok_or_else(|| Error::missing(name))
    }
}

pub fn string<'a>(name: &str, value: &'a Value) -> Result<&'a str, Error> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid(name, "must be a string"))
}

pub fn boolean(name: &str, value: &Value) -> Result<bool, Error> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid(name, "must be a boolean"))
}

/// An integer written without fraction or exponent, within `range`.
pub fn integer(name: &str, value: &Value, range: &RangeInclusive<u32>) -> Result<u32, Error> {
    let Some(number) = value.as_number() else {
        return Err(Error::invalid(name, "must be an integer"));
    };
    if !number.is_u64() && !number.is_i64() {
        return Err(Error::invalid(name, "must be an integer"));
    }
    number
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
        .ok_or_else(|| {
            Error::invalid(
                name,
                format!("must be between {} and {}", range.start(), range.end()),
            )
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn integer_accepts_only_whole_numbers_in_range() {
        let cases = [
            (json!(30), Some(30)),
            (json!(0), Some(0)),
            (json!(100), Some(100)),
            (json!(101), None),
            (json!(-1), None),
            (json!(30.0), None),
            (json!("30"), None),
            (json!(true), None),
            (json!(18_446_744_073_709_551_616.0_f64), None),
        ];
        for (value, expected) in cases {
            assert_eq!(integer("X", &value, &(0..=100)).ok(), expected, "{value}");
        }
    }

    #[test]
    fn null_members_are_absent() {
        let object = json!({"A": null, "B": 1});
        let params = Params(object.as_object().expect("literal is an object"));
        assert_eq!(params.optional("A"), None);
        assert_eq!(params.optional("B"), Some(&json!(1)));
        assert_eq!(
            params.required("A").expect_err("null is absent").code(),
            crate::ErrorCode::MissingParameter
        );
    }
}
