//! Typed access to the members of a command object.
//!
//! A member whose value is `null` is treated as absent everywhere. Required
//! members must all be checked (`require`) before any value is type-checked,
//! because "missing" outranks "invalid" in the error order of SPEC section 2.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, Result};

pub struct Params<'a> {
    members: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    /// The raw value of a member, `None` if absent or `null`.
    pub fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|v| !v.is_null())
    }

    /// Fails with `MissingParameter` if the member is absent.
    pub fn require(&self, name: &str) -> Result<()> {
        self.get(name)
            .map(|_| ())
            .ok_or_else(|| Error::missing(name))
    }

    /// Optional string member (any content).
    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        self.get(name).map(|v| as_string(name, v)).transpose()
    }

    /// Optional integer member within `range`.
    pub fn integer(&self, name: &str, range: RangeInclusive<i64>) -> Result<Option<i64>> {
        self.get(name)
            .map(|v| as_integer(name, v, range))
            .transpose()
    }

    /// A member already known to be present.
    pub fn required_string(&self, name: &str) -> Result<&'a str> {
        self.string(name)?.ok_or_else(|| Error::missing(name))
    }
}

pub fn as_string<'a>(name: &str, value: &'a Value) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a string")))
}

pub fn as_bool(name: &str, value: &Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| Error::invalid_value(format!("{name} must be a boolean")))
}

/// An integer is a JSON number written without fraction or exponent. With
/// serde_json's `arbitrary_precision` the number keeps its literal text, so
/// `30.0` and `3e1` are told apart from `30`.
pub fn as_integer(name: &str, value: &Value, range: RangeInclusive<i64>) -> Result<i64> {
    let parsed = match value {
        Value::Number(n) => n.to_string().parse::<i64>().ok(),
        _ => None,
    };
    match parsed {
        None => Err(Error::invalid_value(format!("{name} must be an integer"))),
        Some(n) if range.contains(&n) => Ok(n),
        Some(_) => Err(Error::invalid_value(format!(
            "{name} must be between {} and {}",
            range.start(),
            range.end()
        ))),
    }
}

/// Checks that `s` is 1 to `max` characters of `A-Z a-z 0-9 _ -`.
pub fn is_identifier(s: &str, max: usize) -> bool {
    (1..=max).contains(&s.chars().count()) && s.chars().all(is_name_char)
}

pub fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn int(text: &str) -> Result<i64> {
        let v: Value = serde_json::from_str(text).unwrap();
        as_integer("X", &v, 0..=100)
    }

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        assert_eq!(int("30"), Ok(30));
        assert_eq!(int("-0"), Ok(0));
        for bad in [
            "30.0",
            "3e1",
            "\"30\"",
            "true",
            "101",
            "-1",
            "99999999999999999999",
        ] {
            assert!(int(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn null_members_are_absent() {
        let v = json!({"A": null, "B": "x"});
        let p = Params::new(v.as_object().unwrap());
        assert!(p.require("A").is_err());
        assert_eq!(p.string("A"), Ok(None));
        assert_eq!(p.string("B"), Ok(Some("x")));
    }

    #[test]
    fn identifiers() {
        assert!(is_identifier("a_B-9", 5));
        assert!(!is_identifier("", 5));
        assert!(!is_identifier("abcdef", 5));
        assert!(!is_identifier("a.b", 5));
        assert!(!is_identifier("é", 5));
    }
}
