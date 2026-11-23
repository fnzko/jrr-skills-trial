use crate::common::{run, run_raw};
use serde_json::{Value, json};

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false));
    response["error"]["code"]
        .as_str()
        .expect("failures carry a code")
}

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":3}"#,
        "",
        "not json",
        r#"{"Command":"Tick","Seconds":4}"#,
    ]);
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0], json!({"ok": true, "Now": 3}));
    assert_eq!(error_code(&responses[1]), "InvalidRequest");
    assert_eq!(error_code(&responses[2]), "InvalidRequest");
    assert_eq!(responses[3], json!({"ok": true, "Now": 7}));
}

#[test]
fn final_line_without_newline_counts_only_when_not_empty() {
    let with_tail = run_raw(b"{\"Command\":\"Tick\",\"Seconds\":1}");
    assert_eq!(with_tail, vec![json!({"ok": true, "Now": 1})]);
    assert_eq!(run_raw(b""), Vec::<Value>::new());
    let blank_then_eof = run_raw(b"\n");
    assert_eq!(blank_then_eof.len(), 1);
    assert_eq!(error_code(&blank_then_eof[0]), "InvalidRequest");
}

#[test]
fn invalid_utf8_line_does_not_stop_the_stream() {
    let responses = run_raw(b"{\"Command\":\"\xff\"}\n{\"Command\":\"Tick\",\"Seconds\":2}\n");
    assert_eq!(responses.len(), 2);
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 2}));
}

#[test]
fn failures_carry_a_code_and_a_message() {
    let responses = run(&[r#"{"Command":"Tick"}"#]);
    assert_eq!(responses[0]["ok"], json!(false));
    assert_eq!(responses[0]["error"]["code"], json!("MissingParameter"));
    assert!(responses[0]["error"]["message"].is_string());
}
