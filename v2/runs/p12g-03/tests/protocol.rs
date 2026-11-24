mod common;

use common::{error_code, exec, run, session};
use serde_json::json;

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":5}"#,
        "",
        r#"{"Command":"Tick","Seconds":7}"#,
    ]);
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0], json!({"ok": true, "Now": 5}));
    assert_eq!(error_code(&responses[1]), Some("InvalidRequest"));
    assert_eq!(responses[2], json!({"ok": true, "Now": 12}));
}

#[test]
fn final_line_without_newline_counts_but_empty_one_does_not() {
    let responses =
        session("{\"Command\":\"Tick\",\"Seconds\":1}\n{\"Command\":\"Tick\",\"Seconds\":1}");
    assert_eq!(responses.len(), 2);
    assert_eq!(session("{\"Command\":\"ListQueues\"}\n").len(), 1);
}

#[test]
fn malformed_lines_are_invalid_requests() {
    let lines = [
        "not json",
        "[1,2]",
        "{}",
        r#"{"Command": 3}"#,
        r#"{"Command": null}"#,
        r#"{"Command": "NoSuchCommand"}"#,
        r#"{"Command": "tick", "Seconds": 1}"#,
        r#"{"Command": "Tick"} trailing"#,
    ];
    for line in lines {
        let responses = run(&[line]);
        assert_eq!(error_code(&responses[0]), Some("InvalidRequest"), "{line}");
    }
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_session_goes_on() {
    let mut input = b"{\"Command\":\"Tick\",\"Seconds\":\xff}\n".to_vec();
    input.extend_from_slice(b"{\"Command\":\"Tick\",\"Seconds\":2}\n");
    let mut output = Vec::new();
    miniq::run(&input[..], &mut output).unwrap();
    let lines: Vec<serde_json::Value> = output
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice(l).unwrap())
        .collect();
    assert_eq!(error_code(&lines[0]), Some("InvalidRequest"));
    assert_eq!(lines[1], json!({"ok": true, "Now": 2}));
}

#[test]
fn tick_checks_its_parameter() {
    let cases = [
        (r#"{"Command":"Tick"}"#, "MissingParameter"),
        (r#"{"Command":"Tick","Seconds":null}"#, "MissingParameter"),
        (
            r#"{"Command":"Tick","Seconds":-1}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":1209601}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":30.0}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":3e1}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":"30"}"#,
            "InvalidParameterValue",
        ),
        (
            r#"{"Command":"Tick","Seconds":true}"#,
            "InvalidParameterValue",
        ),
    ];
    for (line, code) in cases {
        assert_eq!(error_code(&run(&[line])[0]), Some(code), "{line}");
    }
}

#[test]
fn tick_accepts_the_bounds_and_ignores_unknown_members() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":0,"Extra":[1]}"#,
        r#"{"Command":"Tick","Seconds":1209600}"#,
        r#"{"Command":"Tick","Seconds":-0}"#,
    ]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 0}));
    assert_eq!(responses[1], json!({"ok": true, "Now": 1209600}));
    assert_eq!(responses[2], json!({"ok": true, "Now": 1209600}));
}

#[test]
fn error_responses_carry_a_code_and_a_message() {
    let response = &exec(&[json!({"Command": "DeleteQueue", "QueueName": "nope"})])[0];
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["code"], json!("QueueDoesNotExist"));
    assert!(response["error"]["message"].is_string());
}
