//! Typed access to the members of a command or attribute object.
//!
//! Follows the protocol rules: a `null` member counts as absent, integers must be
//! written without fraction or exponent, and every wrong type or out-of-range value
//! is `InvalidParameterValue`.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode, Result};

pub struct Params<'a> {
    map: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    pub fn new(map: &'a Map<String, Value>) -> Self {
        Self { map }
    }

    fn get(&self, name: &str) -> Option<&'a Value> {
        self.map.get(name).filter(|value| !value.is_null())
    }

    pub fn is_present(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Fails with `MissingParameter` for the first of `names` that is absent.
    pub fn require(&self, names: &[&str]) -> Result<()> {
        match names.iter().find(|name| !self.is_present(name)) {
            Some(name) => Err(Error::new(
                ErrorCode::MissingParameter,
                format!("missing required parameter {name}"),
            )),
            None => Ok(()),
        }
    }

    /// A string parameter whose value must satisfy `accepts`.
    pub fn string_where(
        &self,
        name: &str,
        accepts: impl Fn(&str) -> bool,
        rule: &str,
    ) -> Result<Option<&'a str>> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        match value.as_str() {
            Some(text) if accepts(text) => Ok(Some(text)),
            Some(_) => Err(invalid(format!("{name} must be {rule}"))),
            None => Err(invalid(format!("{name} must be a string"))),
        }
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>> {
        self.string_where(name, |_| true, "any string")
    }

    pub fn integer(&self, name: &str, range: RangeInclusive<i64>) -> Result<Option<i64>> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        // With `arbitrary_precision`, `as_i64` only accepts plain integer literals.
        let number = value
            .as_number()
            .and_then(|number| number.as_i64())
            .ok_or_else(|| invalid(format!("{name} must be an integer")))?;
        if range.contains(&number) {
            Ok(Some(number))
        } else {
            Err(invalid(format!(
                "{name} must be between {} and {}",
                range.start(),
                range.end()
            )))
        }
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(invalid(format!("{name} must be a boolean"))),
        }
    }

    pub fn object(&self, name: &str) -> Result<Option<&'a Map<String, Value>>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Object(map)) => Ok(Some(map)),
            Some(_) => Err(invalid(format!("{name} must be an object"))),
        }
    }
}

pub fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidParameterValue, message)
}
