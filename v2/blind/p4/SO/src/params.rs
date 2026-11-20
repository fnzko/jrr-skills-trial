//! Typed access to the members of a command object.
//!
//! The spec orders the checks on parameters (section 2): first every
//! required parameter must be present, and only then is each present value
//! checked for type and range. [`Params::require`] implements the first
//! step, so a command parser calls it for all its required names before
//! reading any value. A member whose value is `null` counts as absent.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::{Error, ErrorCode};

/// A borrowed view of the members of one command (or attribute) object.
#[derive(Debug, Clone, Copy)]
pub struct Params<'a> {
    members: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Params { members }
    }

    /// Returns the value of `name`, or `None` if it is absent or `null`.
    pub fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|value| !value.is_null())
    }

    pub fn is_present(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The names of the members that are present (not `null`).
    pub fn names(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.members
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(name, _)| name.as_str())
    }

    /// Step 2 of section 2: every name in `names` must be present.
    pub fn require(&self, names: &[&str]) -> Result<(), Error> {
        for name in names {
            if !self.is_present(name) {
                return Err(missing(name));
            }
        }
        Ok(())
    }

    /// A string parameter that must be present.
    pub fn required_str(&self, name: &str) -> Result<&'a str, Error> {
        self.optional_str(name)?.ok_or_else(|| missing(name))
    }

    /// A string parameter that may be absent.
    pub fn optional_str(&self, name: &str) -> Result<Option<&'a str>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(wrong_type(name, "a string")),
        }
    }

    /// A boolean parameter that may be absent.
    pub fn optional_bool(&self, name: &str) -> Result<Option<bool>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(wrong_type(name, "a boolean")),
        }
    }

    /// An integer parameter that must be present and lie in `range`.
    pub fn required_int(&self, name: &str, range: RangeInclusive<u64>) -> Result<u64, Error> {
        self.optional_int(name, range)?.ok_or_else(|| missing(name))
    }

    /// An integer parameter that may be absent and, if present, lies in
    /// `range`. Only JSON numbers written without fraction or exponent are
    /// integers; `30.0`, `3e1` and `"30"` are not.
    pub fn optional_int(
        &self,
        name: &str,
        range: RangeInclusive<u64>,
    ) -> Result<Option<u64>, Error> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let number = value
            .as_u64()
            .ok_or_else(|| wrong_type(name, "an integer"))?;
        if range.contains(&number) {
            Ok(Some(number))
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

    /// An object parameter that may be absent.
    pub fn optional_object(&self, name: &str) -> Result<Option<Params<'a>>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Object(members)) => Ok(Some(Params::new(members))),
            Some(_) => Err(wrong_type(name, "an object")),
        }
    }
}

/// The error for a required parameter that is absent.
pub fn missing(name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParameter,
        format!("missing parameter {name}"),
    )
}

fn wrong_type(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("{name} must be {expected}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn members(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => panic!("test input must be an object"),
        }
    }

    #[test]
    fn null_counts_as_absent() {
        let map = members(json!({"Body": null}));
        let params = Params::new(&map);
        assert_eq!(
            params.require(&["Body"]).unwrap_err().code(),
            ErrorCode::MissingParameter
        );
        assert_eq!(params.optional_str("Body").unwrap(), None);
    }

    #[test]
    fn require_reports_the_first_absent_name() {
        let map = members(json!({"A": "x"}));
        let err = Params::new(&map).require(&["A", "B", "C"]).unwrap_err();
        assert_eq!(err.code(), ErrorCode::MissingParameter);
        assert!(err.message().contains('B'), "{err}");
    }

    #[test]
    fn integers_must_be_json_integers_in_range() {
        let map = members(
            json!({"Ok": 30, "Frac": 30.0, "Exp": 3e1, "Text": "30", "Big": 11, "Neg": -1}),
        );
        let params = Params::new(&map);
        assert_eq!(params.optional_int("Ok", 0..=30).unwrap(), Some(30));
        assert_eq!(
            params.optional_int("Frac", 0..=60).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            params.optional_int("Exp", 0..=60).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            params.optional_int("Text", 0..=60).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            params.optional_int("Big", 0..=10).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            params.optional_int("Neg", 0..=10).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
    }

    #[test]
    fn ranges_are_inclusive() {
        let map = members(json!({"Low": 0, "High": 900}));
        let params = Params::new(&map);
        assert_eq!(params.optional_int("Low", 0..=900).unwrap(), Some(0));
        assert_eq!(params.optional_int("High", 0..=900).unwrap(), Some(900));
        assert!(params.optional_int("High", 0..=899).is_err());
    }

    #[test]
    fn booleans_and_strings_are_strict() {
        let map = members(json!({"Flag": 1, "Name": true, "Yes": true}));
        let params = Params::new(&map);
        assert_eq!(
            params.optional_bool("Flag").unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(
            params.optional_str("Name").unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert_eq!(params.optional_bool("Yes").unwrap(), Some(true));
    }

    #[test]
    fn objects_are_checked_for_shape() {
        let map = members(json!({"Attributes": [], "Empty": {}}));
        let params = Params::new(&map);
        assert_eq!(
            params.optional_object("Attributes").unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        assert!(params.optional_object("Empty").unwrap().is_some());
        assert!(params.optional_object("Missing").unwrap().is_none());
    }
}
