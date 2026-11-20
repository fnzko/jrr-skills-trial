mod common;

use common::{error_code, run, run_raw};
use serde_json::json;

#[test]
fn tick_advances_the_clock() {
    let out = run(&[
        r#"{"Command": "Tick", "Seconds": 5}"#,
        r#"{"Command": "Tick", "Seconds": 0}"#,
        r#"{"Command": "Tick", "Seconds": 1209600}"#,
    ]);
    assert_eq!(out[0], json!({"ok": true, "Now": 5}));
    assert_eq!(out[1], json!({"ok": true, "Now": 5}));
    assert_eq!(out[2], json!({"ok": true, "Now": 1209605}));
}

#[test]
fn every_line_gets_one_response_including_a_last_line_without_newline() {
    let out =
        run_raw("\n{\"Command\":\"Tick\",\"Seconds\":1}\n{\"Command\":\"Tick\",\"Seconds\":2}");
    assert_eq!(out.len(), 3);
    assert_eq!(error_code(&out[0]), "InvalidRequest");
    assert_eq!(out[1], json!({"ok": true, "Now": 1}));
    assert_eq!(out[2], json!({"ok": true, "Now": 3}));
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run_raw(""), Vec::<serde_json::Value>::new());
}

#[test]
fn malformed_lines_are_invalid_requests_and_processing_continues() {
    let out = run(&[
        "not json",
        "[1]",
        r#"{"Command": 5}"#,
        r#"{"Command": null}"#,
        r#"{"command": "Tick", "Seconds": 1}"#,
        r#"{"Command": "Nope"}"#,
        r#"{"Command": "Tick", "Seconds": 1} trailing"#,
        r#"{"Command": "Tick", "Seconds": 1}"#,
    ]);
    for response in &out[..7] {
        assert_eq!(error_code(response), "InvalidRequest", "{response}");
    }
    assert_eq!(out[7], json!({"ok": true, "Now": 1}));
}

#[test]
fn invalid_utf8_is_an_invalid_request() {
    let mut output = Vec::new();
    miniq::run(
        &b"{\"Command\":\"Tick\",\"Seconds\":1}\xff\n"[..],
        &mut output,
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(error_code(&response), "InvalidRequest");
}

#[test]
fn tick_checks_its_parameter() {
    let out = run(&[
        r#"{"Command": "Tick"}"#,
        r#"{"Command": "Tick", "Seconds": null}"#,
        r#"{"Command": "Tick", "Seconds": 1.0}"#,
        r#"{"Command": "Tick", "Seconds": 1e0}"#,
        r#"{"Command": "Tick", "Seconds": "1"}"#,
        r#"{"Command": "Tick", "Seconds": -1}"#,
        r#"{"Command": "Tick", "Seconds": 1209601}"#,
        r#"{"Command": "Tick", "Seconds": 1, "Extra": true}"#,
    ]);
    assert_eq!(error_code(&out[0]), "MissingParameter");
    assert_eq!(error_code(&out[1]), "MissingParameter");
    for response in &out[2..7] {
        assert_eq!(error_code(response), "InvalidParameterValue", "{response}");
    }
    assert_eq!(out[7], json!({"ok": true, "Now": 1}));
}
