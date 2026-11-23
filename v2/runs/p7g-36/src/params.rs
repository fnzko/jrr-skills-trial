//! Typed access to the members of a command object.
//!
//! JSON `null` is treated as absent everywhere (SPEC section 1). The getters
//! only check JSON type and range; which command-specific rules follow is up
//! to the caller.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Code, Error, Result};

/// The members of one JSON object (a command or an `Attributes` value).
pub struct Params<'a> {
    members: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Params { members }
    }

    /// The member, unless it is absent or `null`.
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|v| !v.is_null())
    }

    pub fn is_present(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Fails with `MissingParameter` for the first listed name that is absent.
    pub fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|name| !self.is_present(name)) {
            Some(name) => Err(Error::new(
                Code::MissingParameter,
                format!("missing required parameter {name}"),
            )),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        self.get(name)
            .map(|v| v.as_str().ok_or_else(|| wrong_type(name, "a string")))
            .transpose()
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>> {
        self.get(name)
            .map(|v| v.as_bool().ok_or_else(|| wrong_type(name, "a boolean")))
            .transpose()
    }

    /// An integer in `range`. Numbers written with a fraction or exponent are
    /// not integers, however round their value.
    pub fn integer(&self, name: &str, range: RangeInclusive<u64>) -> Result<Option<u64>> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let number = value
            .as_number()
            .filter(|n| n.is_i64() || n.is_u64())
            .ok_or_else(|| wrong_type(name, "an integer"))?;
        // `i64::try_from` keeps `-0` (an integer by SPEC) and rejects anything
        // negative or too large to be in range.
        number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|n| i64::try_from(n).ok()))
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| range.contains(n))
            .map(Some)
            .ok_or_else(|| {
                Error::new(
                    Code::InvalidParameterValue,
                    format!(
                        "{name} must be between {} and {}",
                        range.start(),
                        range.end()
                    ),
                )
            })
    }

    /// A string of `len` characters, all of them `A-Z a-z 0-9 _ -`.
    pub fn identifier(&self, name: &str, len: RangeInclusive<usize>) -> Result<Option<&'a str>> {
        let Some(s) = self.string(name)? else {
            return Ok(None);
        };
        if !len.contains(&s.chars().count()) || !s.bytes().all(is_name_byte) {
            return Err(Error::new(
                Code::InvalidParameterValue,
                format!(
                    "{name} must be {} to {} characters from A-Z a-z 0-9 _ -",
                    len.start(),
                    len.end()
                ),
            ));
        }
        Ok(Some(s))
    }

    /// An optional JSON object.
    pub fn object(&self, name: &str) -> Result<Option<Params<'a>>> {
        self.get(name)
            .map(|v| {
                v.as_object()
                    .map(Params::new)
                    .ok_or_else(|| wrong_type(name, "an object"))
            })
            .transpose()
    }

    /// Fails with `InvalidParameterValue` for any member not in `known`.
    pub fn deny_unknown(&self, known: &[&str]) -> Result<()> {
        match self.members.keys().find(|k| !known.contains(&k.as_str())) {
            Some(key) => Err(Error::new(
                Code::InvalidParameterValue,
                format!("unknown attribute {key}"),
            )),
            None => Ok(()),
        }
    }
}

/// The characters allowed in queue names, group ids and deduplication ids.
pub fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn wrong_type(name: &str, expected: &str) -> Error {
    Error::new(
        Code::InvalidParameterValue,
        format!("{name} must be {expected}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn with<T>(v: Value, f: impl FnOnce(Params) -> T) -> T {
        f(Params::new(v.as_object().unwrap()))
    }

    #[test]
    fn null_is_absent() {
        with(json!({"A": null}), |p| {
            assert!(!p.is_present("A"));
            assert_eq!(p.require(&["A"]).unwrap_err().code, Code::MissingParameter);
            assert_eq!(p.integer("A", 0..=1).unwrap(), None);
        });
    }

    #[test]
    fn integers_must_be_written_as_integers() {
        let ok: Value = serde_json::from_str(r#"{"A": 30}"#).unwrap();
        with(ok, |p| {
            assert_eq!(p.integer("A", 0..=50).unwrap(), Some(30))
        });
        for text in [
            r#"{"A": 30.0}"#,
            r#"{"A": 3e1}"#,
            r#"{"A": "30"}"#,
            r#"{"A": true}"#,
        ] {
            let v: Value = serde_json::from_str(text).unwrap();
            with(v, |p| {
                assert_eq!(
                    p.integer("A", 0..=50).unwrap_err().code,
                    Code::InvalidParameterValue
                );
            });
        }
    }

    #[test]
    fn negative_zero_is_an_integer() {
        let v: Value = serde_json::from_str(r#"{"A": -0}"#).unwrap();
        with(v, |p| assert_eq!(p.integer("A", 0..=5).unwrap(), Some(0)));
    }

    #[test]
    fn integer_range_is_inclusive() {
        with(json!({"A": 5, "B": 6, "C": -1}), |p| {
            assert_eq!(p.integer("A", 1..=5).unwrap(), Some(5));
            assert!(p.integer("B", 1..=5).is_err());
            assert!(p.integer("C", 0..=5).is_err());
        });
    }

    #[test]
    fn identifier_counts_characters_and_checks_alphabet() {
        with(
            json!({"A": "ok_-9", "B": "bad.id", "C": "", "D": "é"}),
            |p| {
                assert_eq!(p.identifier("A", 1..=5).unwrap(), Some("ok_-9"));
                assert!(p.identifier("A", 1..=4).is_err());
                assert!(p.identifier("B", 1..=9).is_err());
                assert!(p.identifier("C", 1..=9).is_err());
                assert!(p.identifier("D", 1..=9).is_err());
            },
        );
    }
}
