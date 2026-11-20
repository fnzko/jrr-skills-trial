//! Builds the one-line JSON responses of SPEC section 1.

use serde_json::{Map, Value};

use crate::error::Error;

/// A success response: `{"ok": true}` plus the command's result members.
pub fn success(result: Map<String, Value>) -> Value {
    let mut object = Map::with_capacity(result.len() + 1);
    object.insert("ok".to_owned(), Value::Bool(true));
    object.extend(result);
    Value::Object(object)
}

/// A failure response. The `error` object may carry extra members per the
/// spec, but we send only `code` and `message`.
pub fn failure(error: &Error) -> Value {
    let mut detail = Map::new();
    detail.insert("code".to_owned(), Value::from(error.code.as_str()));
    detail.insert("message".to_owned(), Value::from(error.message.as_str()));

    let mut object = Map::new();
    object.insert("ok".to_owned(), Value::Bool(false));
    object.insert("error".to_owned(), Value::Object(detail));
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use serde_json::json;

    #[test]
    fn success_with_no_members_is_just_ok() {
        assert_eq!(success(Map::new()), json!({"ok": true}));
    }

    #[test]
    fn success_carries_result_members() {
        let mut result = Map::new();
        result.insert("Now".to_owned(), json!(42));
        assert_eq!(success(result), json!({"ok": true, "Now": 42}));
    }

    #[test]
    fn failure_has_code_and_message_under_error() {
        let error = Error::new(ErrorCode::QueueDoesNotExist, "no queue named x");
        assert_eq!(
            failure(&error),
            json!({
                "ok": false,
                "error": {"code": "QueueDoesNotExist", "message": "no queue named x"}
            })
        );
    }
}
