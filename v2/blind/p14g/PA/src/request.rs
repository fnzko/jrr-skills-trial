//! Typed access to the members of one command object.
//!
//! The protocol checks required members first and member values second, so
//! the two steps are separate: [`Params::required`] and [`Params::optional`]
//! only look at presence, and the [`Arg`] they return converts the value.

use serde_json::{Map, Value};

use crate::error::ApiError;

/// The members of a command object. A member whose value is `null` counts
/// as absent.
pub struct Params<'a>(&'a Map<String, Value>);

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self(members)
    }

    pub fn optional(&self, name: &'a str) -> Option<Arg<'a>> {
        self.0
            .get(name)
            .filter(|value| !value.is_null())
            .map(|value| Arg { name, value })
    }

    pub fn required(&self, name: &'a str) -> Result<Arg<'a>, ApiError> {
        self.optional(name).ok_or_else(|| ApiError::missing(name))
    }
}

/// One present member, ready to be converted to the type the command needs.
#[derive(Clone, Copy)]
pub struct Arg<'a> {
    name: &'a str,
    value: &'a Value,
}

impl<'a> Arg<'a> {
    pub fn new(name: &'a str, value: &'a Value) -> Self {
        Self { name, value }
    }

    fn invalid(&self, expected: &str) -> ApiError {
        ApiError::invalid_value(self.name, expected)
    }

    pub fn string(self) -> Result<&'a str, ApiError> {
        self.value.as_str().ok_or_else(|| self.invalid("a string"))
    }

    /// A string that `make` accepts, for types with a validity rule.
    pub fn text<T>(self, make: fn(&str) -> Option<T>) -> Result<T, ApiError> {
        make(self.string()?)
            .ok_or_else(|| self.invalid("a string that follows the length and character rules"))
    }

    /// A JSON integer that `make` accepts. `30.0` and `3e1` are not integers
    /// and are rejected like any other wrong type.
    pub fn integer<T>(self, make: fn(u64) -> Option<T>) -> Result<T, ApiError> {
        // The number is read from its source text: `-0` is an integer, while
        // `30.0` and `3e1` are not, which a parsed float cannot tell apart.
        self.value
            .as_number()
            .and_then(|number| number.to_string().parse::<i128>().ok())
            .and_then(|number| u64::try_from(number).ok())
            .and_then(make)
            .ok_or_else(|| self.invalid("an integer inside the allowed range"))
    }

    pub fn boolean(self) -> Result<bool, ApiError> {
        self.value
            .as_bool()
            .ok_or_else(|| self.invalid("true or false"))
    }

    pub fn object(self) -> Result<&'a Map<String, Value>, ApiError> {
        self.value
            .as_object()
            .ok_or_else(|| self.invalid("an object"))
    }
}
