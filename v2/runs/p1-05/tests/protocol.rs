mod common;

use common::{error_code, run, run_raw};
use serde_json::json;

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":1}"#,
        "",
        r#"{"Command":"Tick","Seconds":2}"#,
        "not json",
        r#"{"Command":"Tick","Seconds":3}"#,
    ]);
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0], json!({"ok": true, "Now": 1}));
    assert_eq!(error_code(&responses[1]), "InvalidRequest");
    assert_eq!(responses[2], json!({"ok": true, "Now": 3}));
    assert_eq!(error_code(&responses[3]), "InvalidRequest");
    assert_eq!(responses[4], json!({"ok": true, "Now": 6}));
}

#[test]
fn final_line_without_newline_counts_when_not_empty() {
    let responses =
        run_raw(b"{\"Command\":\"Tick\",\"Seconds\":1}\n{\"Command\":\"Tick\",\"Seconds\":1}");
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[1], json!({"ok": true, "Now": 2}));
}

#[test]
fn input_ending_with_newline_has_no_extra_response() {
    assert_eq!(run_raw(b"{\"Command\":\"Tick\",\"Seconds\":1}\n").len(), 1);
    assert_eq!(run_raw(b"").len(), 0);
}

#[test]
fn blank_lines_are_commands() {
    let responses = run_raw(b"\n\n");
    assert_eq!(responses.len(), 2);
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(error_code(&responses[1]), "InvalidRequest");
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_next_line_still_runs() {
    let responses =
        run_raw(b"{\"Command\":\"Tick\",\"Seconds\":\xff}\n{\"Command\":\"Tick\",\"Seconds\":4}\n");
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 4}));
}

#[test]
fn failure_responses_carry_a_code_and_a_message() {
    let responses = run(&[r#"{"Command":"Nope"}"#]);
    assert_eq!(responses[0]["ok"], false);
    assert_eq!(responses[0]["error"]["code"], "InvalidRequest");
    assert!(responses[0]["error"]["message"].is_string());
}

#[test]
fn unknown_members_are_ignored() {
    let responses = run(&[r#"{"Command":"Tick","Seconds":2,"Colour":"red"}"#]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 2}));
}

#[test]
fn a_failed_command_changes_no_state() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":2}"#,
        r#"{"Command":"Tick","Seconds":1209601}"#,
        r#"{"Command":"Tick","Seconds":0}"#,
    ]);
    assert_eq!(responses[2], json!({"ok": true, "Now": 2}));
}

#[test]
fn error_codes_follow_the_documented_precedence() {
    let responses = run(&[
        // Required parameters come before value checks.
        r#"{"Command":"SendMessage","QueueName":1}"#,
        // Value checks come before the queue lookup.
        r#"{"Command":"SendMessage","QueueName":"nope","Body":""}"#,
        // The lookup comes before the rules of the command.
        r#"{"Command":"SendMessage","QueueName":"nope","Body":"b"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"nope","ReceiptHandle":"junk"}"#,
        // Any string is looked up as is.
        r#"{"Command":"PurgeQueue","QueueName":"not a name"}"#,
    ]);
    let codes: Vec<_> = responses.iter().map(|r| error_code(r)).collect();
    assert_eq!(
        codes,
        [
            "MissingParameter",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "QueueDoesNotExist",
            "QueueDoesNotExist",
        ]
    );
}

#[test]
fn big_numbers_and_exponents_are_values_not_malformed_requests() {
    let responses = run(&[
        r#"{"Command":"Tick","Seconds":1e999}"#,
        r#"{"Command":"Tick","Seconds":123456789012345678901234567890}"#,
    ]);
    assert_eq!(error_code(&responses[0]), "InvalidParameterValue");
    assert_eq!(error_code(&responses[1]), "InvalidParameterValue");
}
