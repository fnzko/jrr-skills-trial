use crate::support::{Session, assert_error};
use serde_json::json;

#[test]
fn malformed_requests_are_invalid_and_the_session_continues() {
    let lines: &[&[u8]] = &[
        b"",
        b"   ",
        b"not json",
        b"[1, 2]",
        b"\"Tick\"",
        b"{\"Command\": \"Tick\", \"Seconds\": 1} trailing",
        b"{\"Command\": \"Tick\", \"Seconds\": 1}{}",
        b"{}",
        b"{\"Command\": 7}",
        b"{\"Command\": null}",
        b"{\"Command\": \"tick\"}",
        b"{\"Command\": \"Nope\"}",
        b"{\"Command\": \"Tick\", \"Seconds\": \"\xff\"}",
    ];
    let mut session = Session::default();
    for line in lines {
        assert_error(&session.line(line), "InvalidRequest");
    }
    assert_eq!(
        session.run(json!({"Command": "Tick", "Seconds": 1})),
        json!({"ok": true, "Now": 1})
    );
}

#[test]
fn tick_advances_the_clock_and_reports_it() {
    let mut session = Session::default();
    assert_eq!(
        session.run(json!({"Command": "Tick", "Seconds": 0})),
        json!({"ok": true, "Now": 0})
    );
    assert_eq!(
        session.run(json!({"Command": "Tick", "Seconds": 1209600, "Extra": [1]})),
        json!({"ok": true, "Now": 1209600})
    );
    assert_eq!(
        session.line(br#"{"Command":"Tick","Seconds":5}"#),
        json!({"ok": true, "Now": 1209605})
    );
}

#[test]
fn tick_parameter_errors() {
    let mut session = Session::default();
    session.fails(json!({"Command": "Tick"}), "MissingParameter");
    session.fails(
        json!({"Command": "Tick", "Seconds": null}),
        "MissingParameter",
    );
    session.fails(
        json!({"Command": "Tick", "Seconds": 1209601}),
        "InvalidParameterValue",
    );
    session.fails(
        json!({"Command": "Tick", "Seconds": "5"}),
        "InvalidParameterValue",
    );
    assert_eq!(
        session.run(json!({"Command": "Tick", "Seconds": 0})),
        json!({"ok": true, "Now": 0})
    );
}
