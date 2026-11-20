//! Typed access to the members of a JSON object, following the protocol rules:
//! a `null` member counts as absent, and a present member of the wrong type or
//! outside its range is `InvalidParameterValue`.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

#[derive(Debug, Clone, Copy)]
pub struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self(members)
    }

    fn get(&self, name: &str) -> Option<&'a Value> {
        self.0.get(name).filter(|value| !value.is_null())
    }

    /// Fails with `MissingParameter` for the first of `names` that is absent.
    /// Called before any value is read, so a missing parameter always wins over
    /// an invalid one.
    pub fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Error::missing(name)),
            None => Ok(()),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        self.get(name)
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| Error::invalid_value(name, "must be a string"))
            })
            .transpose()
    }

    pub fn required_string(&self, name: &str) -> Result<&'a str> {
        self.string(name)?.ok_or_else(|| Error::missing(name))
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>> {
        self.get(name)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| Error::invalid_value(name, "must be a boolean"))
            })
            .transpose()
    }

    pub fn object(&self, name: &str) -> Result<Option<&'a Map<String, Value>>> {
        self.get(name)
            .map(|value| {
                value
                    .as_object()
                    .ok_or_else(|| Error::invalid_value(name, "must be an object"))
            })
            .transpose()
    }

    /// An integer written without fraction or exponent, within `range`.
    pub fn integer(&self, name: &str, range: RangeInclusive<u32>) -> Result<Option<u32>> {
        self.get(name)
            .map(|value| {
                integer_value(value)
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| range.contains(n))
                    .ok_or_else(|| {
                        Error::invalid_value(
                            name,
                            &format!(
                                "must be an integer from {} to {}",
                                range.start(),
                                range.end()
                            ),
                        )
                    })
            })
            .transpose()
    }

    pub fn required_integer(&self, name: &str, range: RangeInclusive<u32>) -> Result<u32> {
        self.integer(name, range)?
            .ok_or_else(|| Error::missing(name))
    }

    /// Fails with `InvalidParameterValue` for a present member not in `known`.
    pub fn reject_unknown(&self, known: &[&str]) -> Result<()> {
        let unknown = self
            .0
            .iter()
            .find(|(name, value)| !value.is_null() && !known.contains(&name.as_str()));
        match unknown {
            Some((name, _)) => Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("{name} is not a known attribute"),
            )),
            None => Ok(()),
        }
    }
}

/// The value of a JSON number written as an integer: digits with an optional
/// minus sign, no fraction or exponent. Relies on `arbitrary_precision`, which
/// keeps the number's original text; without it `-0` would read as a float.
fn integer_value(value: &Value) -> Option<i64> {
    let Value::Number(number) = value else {
        return None;
    };
    let text = number.to_string();
    let digits = text.strip_prefix('-').unwrap_or(&text);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
