//! Typed access to the members of a JSON object. A `null` member counts as
//! absent, and every type or range violation is `InvalidParameterValue`.

use std::ops::RangeInclusive;

use serde_json::{Map, Value};

use crate::error::{Error, ErrorCode};
use crate::types::{AttributeUpdate, Seconds};

pub struct Params<'a> {
    members: &'a Map<String, Value>,
}

fn invalid(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidParameterValue,
        format!("{name} must be {expected}"),
    )
}

impl<'a> Params<'a> {
    pub fn new(members: &'a Map<String, Value>) -> Self {
        Self { members }
    }

    fn get(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|value| !value.is_null())
    }

    /// Fails on the first of `names` that is absent. Called before any
    /// value is read, because a missing parameter outranks an invalid one.
    pub fn require(&self, names: &[&str]) -> Result<(), Error> {
        match names.iter().find(|name| self.get(name).is_none()) {
            Some(name) => Err(Self::missing(name)),
            None => Ok(()),
        }
    }

    fn missing(name: &str) -> Error {
        Error::new(ErrorCode::MissingParameter, format!("{name} is required"))
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(invalid(name, "a string")),
        }
    }

    pub fn required_string(&self, name: &str) -> Result<&'a str, Error> {
        self.string(name)?.ok_or_else(|| Self::missing(name))
    }

    /// A string that `parse` accepts; `what` describes the rule in errors.
    pub fn parsed_string<T>(
        &self,
        name: &str,
        what: &str,
        parse: impl FnOnce(&'a str) -> Option<T>,
    ) -> Result<Option<T>, Error> {
        self.string(name)?
            .map(|text| parse(text).ok_or_else(|| invalid(name, what)))
            .transpose()
    }

    pub fn integer<T: TryFrom<u64>>(
        &self,
        name: &str,
        range: RangeInclusive<u64>,
    ) -> Result<Option<T>, Error> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        let expected = format!("an integer from {} to {}", range.start(), range.end());
        value
            .as_u64()
            .filter(|number| range.contains(number))
            .and_then(|number| T::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| invalid(name, &expected))
    }

    pub fn required_integer<T: TryFrom<u64>>(
        &self,
        name: &str,
        range: RangeInclusive<u64>,
    ) -> Result<T, Error> {
        self.integer(name, range)?
            .ok_or_else(|| Self::missing(name))
    }

    pub fn seconds(
        &self,
        name: &str,
        range: RangeInclusive<u64>,
    ) -> Result<Option<Seconds>, Error> {
        Ok(self.integer(name, range)?.map(Seconds))
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(invalid(name, "a boolean")),
        }
    }

    pub fn object(&self, name: &str) -> Result<Option<&'a Map<String, Value>>, Error> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Object(members)) => Ok(Some(members)),
            Some(_) => Err(invalid(name, "an object")),
        }
    }

    pub fn required_object(&self, name: &str) -> Result<&'a Map<String, Value>, Error> {
        self.object(name)?.ok_or_else(|| Self::missing(name))
    }
}

const ATTRIBUTE_NAMES: [&str; 7] = [
    "VisibilityTimeout",
    "DelaySeconds",
    "RetentionSeconds",
    "Fifo",
    "ContentBasedDeduplication",
    "DeadLetterQueue",
    "MaxReceiveCount",
];

impl<'a> AttributeUpdate<'a> {
    /// Reads and range-checks an `Attributes` object; names outside the
    /// attribute table are rejected.
    pub fn parse(members: &'a Map<String, Value>) -> Result<Self, Error> {
        let unknown = members
            .iter()
            .find(|(name, value)| !value.is_null() && !ATTRIBUTE_NAMES.contains(&name.as_str()));
        if let Some((name, _)) = unknown {
            return Err(Error::new(
                ErrorCode::InvalidParameterValue,
                format!("{name} is not a queue attribute"),
            ));
        }
        let attributes = Params::new(members);
        Ok(Self {
            visibility_timeout: attributes.seconds("VisibilityTimeout", 0..=43_200)?,
            delay: attributes.seconds("DelaySeconds", 0..=900)?,
            retention: attributes.seconds("RetentionSeconds", 60..=1_209_600)?,
            fifo: attributes.boolean("Fifo")?,
            content_based_deduplication: attributes.boolean("ContentBasedDeduplication")?,
            dead_letter_queue: attributes.string("DeadLetterQueue")?,
            max_receive_count: attributes.integer("MaxReceiveCount", 1..=1000)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn members(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(members) => members,
            other => panic!("test input must be an object, got {other}"),
        }
    }

    #[test]
    fn integers_must_be_whole_numbers_in_range() {
        let cases = [
            (json!(30), Some(30)),
            (json!(0), Some(0)),
            (json!(43200), Some(43_200)),
            (json!(43201), None),
            (json!(-1), None),
            (json!(30.0), None),
            (json!("30"), None),
            (json!(true), None),
        ];
        for (value, expected) in cases {
            let object = members(json!({ "N": value }));
            let got = Params::new(&object).integer::<u64>("N", 0..=43_200);
            match expected {
                Some(number) => assert_eq!(got, Ok(Some(number)), "{value}"),
                None => assert_eq!(got.unwrap_err().code(), ErrorCode::InvalidParameterValue),
            }
        }
    }

    #[test]
    fn null_counts_as_absent() {
        let object = members(json!({ "A": null }));
        let params = Params::new(&object);
        assert_eq!(params.string("A"), Ok(None));
        assert_eq!(
            params.require(&["A"]).unwrap_err().code(),
            ErrorCode::MissingParameter
        );
    }

    #[test]
    fn unknown_attribute_is_rejected_but_null_one_is_ignored() {
        let bad = members(json!({ "Colour": 1 }));
        assert_eq!(
            AttributeUpdate::parse(&bad).unwrap_err().code(),
            ErrorCode::InvalidParameterValue
        );
        let ignored = members(json!({ "Colour": null, "DelaySeconds": 5 }));
        let update = AttributeUpdate::parse(&ignored).unwrap();
        assert_eq!(update.delay, Some(Seconds(5)));
    }
}
