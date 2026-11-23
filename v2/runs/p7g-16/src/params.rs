//! Declarative parameter validation, so that every command applies the checks of
//! SPEC section 2 in the same order: required parameters first, then values.

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::attributes::AttributeUpdate;
use crate::error::{Error, ErrorCode, Result};

/// How the value of a parameter is checked.
#[derive(Debug, Clone, Copy)]
pub enum Rule {
    /// Any string.
    Str,
    /// A string whose UTF-8 encoding is `min..=max` bytes long.
    Bytes { min: usize, max: usize },
    /// 1 to 128 characters from `A-Z a-z 0-9 _ -`.
    Id,
    /// An integer in `min..=max`.
    Int { min: i64, max: i64 },
    /// A queue name for `CreateQueue`; breaking the syntax is `QueueNameInvalid`.
    NewQueueName,
    /// An object of queue attributes.
    Attributes,
}

/// One parameter of a command.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub rule: Rule,
    pub required: bool,
}

impl Param {
    pub const fn required(name: &'static str, rule: Rule) -> Self {
        Self {
            name,
            rule,
            required: true,
        }
    }

    pub const fn optional(name: &'static str, rule: Rule) -> Self {
        Self {
            name,
            rule,
            required: false,
        }
    }
}

#[derive(Debug)]
enum Parsed {
    Str(String),
    Int(i64),
    Attributes(AttributeUpdate),
}

/// The validated parameters of one command.
#[derive(Debug, Default)]
pub struct Params {
    values: HashMap<&'static str, Parsed>,
}

impl Params {
    /// Applies SPEC section 2 steps 2 and 3. Parameters are value-checked in the order of
    /// `spec`, so list `QueueName` of `CreateQueue` first.
    pub fn parse(spec: &[Param], object: &Map<String, Value>) -> Result<Self> {
        let given = |name: &str| object.get(name).filter(|v| !v.is_null());

        for param in spec.iter().filter(|p| p.required) {
            if given(param.name).is_none() {
                return Err(Error::new(
                    ErrorCode::MissingParameter,
                    format!("missing required parameter {}", param.name),
                ));
            }
        }

        let mut values = HashMap::new();
        for param in spec {
            if let Some(value) = given(param.name) {
                values.insert(param.name, check(param, value)?);
            }
        }
        Ok(Self { values })
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        match self.values.get(name) {
            Some(Parsed::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        match self.values.get(name) {
            Some(Parsed::Int(n)) => Some(*n),
            _ => None,
        }
    }

    pub fn attributes(&self) -> Option<&AttributeUpdate> {
        match self.values.get("Attributes") {
            Some(Parsed::Attributes(a)) => Some(a),
            _ => None,
        }
    }

    /// A required string parameter that [`Params::parse`] has already checked.
    pub fn required_str(&self, name: &str) -> &str {
        self.str(name)
            .unwrap_or_else(|| unreachable!("parameter {name} was validated as required"))
    }

    /// A required integer parameter that [`Params::parse`] has already checked.
    pub fn required_int(&self, name: &str) -> i64 {
        self.int(name)
            .unwrap_or_else(|| unreachable!("parameter {name} was validated as required"))
    }
}

fn check(param: &Param, value: &Value) -> Result<Parsed> {
    let name = param.name;
    let invalid = |what: &str| {
        Error::new(
            ErrorCode::InvalidParameterValue,
            format!("parameter {name} {what}"),
        )
    };
    match param.rule {
        Rule::Attributes => match value {
            Value::Object(map) => AttributeUpdate::parse(map).map(Parsed::Attributes),
            _ => Err(invalid("must be an object")),
        },
        Rule::Int { min, max } => parse_int(value, min, max)
            .map(Parsed::Int)
            .ok_or_else(|| invalid(&format!("must be an integer from {min} to {max}"))),
        rule => {
            let s = value.as_str().ok_or_else(|| invalid("must be a string"))?;
            match rule {
                Rule::Bytes { min, max } if !(min..=max).contains(&s.len()) => {
                    Err(invalid(&format!("must be {min} to {max} bytes long")))
                }
                Rule::Id if !is_id(s) => {
                    Err(invalid("must be 1 to 128 characters from A-Z a-z 0-9 _ -"))
                }
                Rule::NewQueueName if !is_valid_queue_name(s) => Err(Error::new(
                    ErrorCode::QueueNameInvalid,
                    format!("invalid queue name {s:?}"),
                )),
                _ => Ok(Parsed::Str(s.to_owned())),
            }
        }
    }
}

/// An integer in `min..=max`, where "integer" means a JSON number without fraction or
/// exponent. Relies on serde_json's `arbitrary_precision`, which keeps the number's text.
pub fn parse_int(value: &Value, min: i64, max: i64) -> Option<i64> {
    let Value::Number(n) = value else { return None };
    let n: i128 = n.to_string().parse().ok()?;
    i64::try_from(n).ok().filter(|n| (min..=max).contains(n))
}

pub fn parse_bool(value: &Value) -> Option<bool> {
    value.as_bool()
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_id(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.chars().all(is_name_char)
}

/// Queue name syntax of SPEC section 4.
pub fn is_valid_queue_name(name: &str) -> bool {
    let plain = |s: &str| s.chars().all(is_name_char);
    match name.strip_suffix(".fifo") {
        Some(stem) => (1..=75).contains(&stem.len()) && plain(stem),
        None => (1..=80).contains(&name.len()) && plain(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integers_must_be_written_without_fraction_or_exponent() {
        let int = |text: &str| parse_int(&serde_json::from_str(text).unwrap(), 0, 100);
        assert_eq!(int("30"), Some(30));
        assert_eq!(int("-0"), Some(0));
        assert_eq!(int("30.0"), None);
        assert_eq!(int("3e1"), None);
        assert_eq!(int("101"), None);
        assert_eq!(int("1e999"), None);
        assert_eq!(parse_int(&json!("30"), 0, 100), None);
    }

    #[test]
    fn queue_name_syntax() {
        assert!(is_valid_queue_name("jobs-1_A"));
        assert!(is_valid_queue_name("jobs.fifo"));
        assert!(is_valid_queue_name(&"a".repeat(80)));
        assert!(is_valid_queue_name(&format!("{}.fifo", "a".repeat(75))));
        assert!(!is_valid_queue_name(""));
        assert!(!is_valid_queue_name(".fifo"));
        assert!(!is_valid_queue_name("a.b"));
        assert!(!is_valid_queue_name("a.fifo.fifo"));
        assert!(!is_valid_queue_name(&"a".repeat(81)));
        assert!(!is_valid_queue_name(&format!("{}.fifo", "a".repeat(76))));
        assert!(!is_valid_queue_name("é"));
    }

    #[test]
    fn missing_is_checked_before_values() {
        let spec = [
            Param::required("A", Rule::Str),
            Param::optional("B", Rule::Int { min: 0, max: 1 }),
        ];
        let obj = json!({"B": 5});
        let err = Params::parse(&spec, obj.as_object().unwrap()).unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingParameter);
    }

    #[test]
    fn null_counts_as_absent() {
        let spec = [Param::required("A", Rule::Str)];
        let obj = json!({"A": null});
        let err = Params::parse(&spec, obj.as_object().unwrap()).unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingParameter);
    }
}
