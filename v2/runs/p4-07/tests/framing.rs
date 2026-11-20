//! Section 1: one response line for every input line, in order.

mod support;

use serde_json::json;
use support::{refused, replies, replies_for_bytes};

#[test]
fn blank_and_unparsable_lines_get_their_own_refusal_and_the_session_goes_on() {
    let input =
        b"{\"Command\":\"Tick\",\"Seconds\":0}\n\nnot json\n{\"Command\":\"Tick\",\"Seconds\":1}";
    assert_eq!(
        replies_for_bytes(input),
        vec![
            json!({"ok": true, "Now": 0}),
            refused("InvalidRequest"),
            refused("InvalidRequest"),
            json!({"ok": true, "Now": 1}),
        ]
    );
}

#[test]
fn a_line_that_is_not_utf8_is_refused_and_the_next_line_is_still_served() {
    let input = b"\xff\n{\"Command\":\"Tick\",\"Seconds\":0}\n";
    assert_eq!(
        replies_for_bytes(input),
        vec![refused("InvalidRequest"), json!({"ok": true, "Now": 0})]
    );
}

#[test]
fn unknown_and_null_members_are_ignored_or_treated_as_absent() {
    assert_eq!(
        replies(&[
            r#"{"Command":"Tick","Seconds":2,"Extra":{"x":[1]}}"#,
            r#"{"Command":"ListQueues","Prefix":null}"#,
        ]),
        vec![
            json!({"ok": true, "Now": 2}),
            json!({"ok": true, "QueueNames": []})
        ]
    );
}

#[test]
fn a_command_name_is_case_sensitive() {
    assert_eq!(
        replies(&[r#"{"Command":"tick","Seconds":1}"#]),
        vec![refused("InvalidRequest")]
    );
}
