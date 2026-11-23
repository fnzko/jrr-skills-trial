mod common;

use common::{error_code, run, session};
use serde_json::json;

#[test]
fn every_line_gets_exactly_one_response_in_order() {
    let out = session(&[
        r#"{"Command":"Tick","Seconds":3}"#,
        "",
        "not json",
        r#"{"Command":"Tick","Seconds":4}"#,
    ]);
    assert_eq!(out.len(), 4);
    assert_eq!(out[0], json!({"ok": true, "Now": 3}));
    assert_eq!(error_code(&out[1]), "InvalidRequest");
    assert_eq!(error_code(&out[2]), "InvalidRequest");
    assert_eq!(out[3], json!({"ok": true, "Now": 7}));
}

#[test]
fn final_line_without_newline_counts() {
    let mut output = Vec::new();
    miniq::run(&b"{\"Command\":\"Tick\",\"Seconds\":1}"[..], &mut output).unwrap();
    assert_eq!(output, b"{\"ok\":true,\"Now\":1}\n");
}

#[test]
fn request_shape_errors() {
    let out = session(&[
        "[]",
        r#"{"Command":5}"#,
        r#"{"Command":null}"#,
        r#"{"Seconds":1}"#,
        r#"{"Command":"tick","Seconds":1}"#,
        r#"{"Command":"Nope"}"#,
    ]);
    let codes: Vec<_> = out.iter().map(error_code).collect();
    assert_eq!(codes, ["InvalidRequest"; 6]);
}

#[test]
fn invalid_utf8_is_invalid_request_and_processing_continues() {
    let mut output = Vec::new();
    let input = b"\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":1}\n";
    miniq::run(&input[..], &mut output).unwrap();
    let lines: Vec<_> = std::str::from_utf8(&output).unwrap().lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("InvalidRequest"));
    assert_eq!(lines[1], r#"{"ok":true,"Now":1}"#);
}

#[test]
fn integer_parameters_reject_non_integers() {
    let out = session(&[
        r#"{"Command":"Tick","Seconds":30.0}"#,
        r#"{"Command":"Tick","Seconds":3e1}"#,
        r#"{"Command":"Tick","Seconds":"30"}"#,
        r#"{"Command":"Tick","Seconds":-1}"#,
        r#"{"Command":"Tick","Seconds":1209601}"#,
        r#"{"Command":"Tick","Seconds":1209600}"#,
        r#"{"Command":"Tick"}"#,
        r#"{"Command":"Tick","Seconds":null}"#,
    ]);
    let codes: Vec<_> = out
        .iter()
        .map(|r| r["error"]["code"].as_str().unwrap_or("ok"))
        .collect();
    assert_eq!(
        codes,
        [
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "MissingParameter",
            "MissingParameter",
        ]
    );
}

#[test]
fn unknown_members_are_ignored_and_null_means_absent() {
    let out = run(&[
        json!({"Command": "CreateQueue", "QueueName": "q", "Extra": 1, "Attributes": {"DelaySeconds": null}}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
    ]);
    assert_eq!(out[0], json!({"ok": true}));
    assert_eq!(out[1]["Attributes"]["DelaySeconds"], 0);
}

#[test]
fn missing_parameter_beats_invalid_value() {
    let out = session(&[
        r#"{"Command":"SendMessage","QueueName":5}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":99999}"#,
    ]);
    assert_eq!(error_code(&out[0]), "MissingParameter");
    assert_eq!(error_code(&out[1]), "InvalidParameterValue");
}
