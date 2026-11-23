use serde_json::json;

use crate::common::*;

#[test]
fn every_line_gets_exactly_one_response() {
    let responses = run_raw(b"\n   \nnot json\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(responses.len(), 4);
    assert_eq!(code(&responses[0]), "InvalidRequest");
    assert_eq!(code(&responses[1]), "InvalidRequest");
    assert_eq!(code(&responses[2]), "InvalidRequest");
    assert_eq!(responses[3], json!({"ok": true, "Now": 1}));
}

#[test]
fn final_line_without_newline_counts_only_if_not_empty() {
    let with_line = run_raw(b"{\"Command\":\"Tick\",\"Seconds\":2}");
    assert_eq!(with_line, vec![json!({"ok": true, "Now": 2})]);
    assert_eq!(run_raw(b"").len(), 0);
    assert_eq!(run_raw(b"{\"Command\":\"Tick\",\"Seconds\":2}\n").len(), 1);
}

#[test]
fn invalid_utf8_is_one_invalid_request_and_processing_continues() {
    let responses = run_raw(b"{\"Command\":\"\xff\"}\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(code(&responses[0]), "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn error_responses_carry_a_code_and_a_message() {
    let responses = run(&[json!({"Command": "Nope"})]);
    assert_eq!(responses[0]["ok"], json!(false));
    assert!(responses[0]["error"]["message"].is_string());
    assert_eq!(responses[0].as_object().map(|o| o.len()), Some(2));
}

#[test]
fn unknown_members_and_null_parameters_are_ignored() {
    let responses = run(&[
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": null, "Extra": [1]}),
        json!({"Command": "ListQueues", "Prefix": null}),
    ]);
    assert_eq!(responses[0], ok());
    assert_eq!(responses[1], json!({"ok": true, "QueueNames": ["q"]}));
}

#[test]
fn required_parameter_errors_precede_value_errors() {
    let responses = run(&[json!({"Command": "SendMessage", "QueueName": 5})]);
    assert_eq!(code(&responses[0]), "MissingParameter");
}

#[test]
fn filling_a_queue_with_100000_messages_is_fast_and_the_next_send_fails() {
    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"big\"}\n");
    for _ in 0..100_000 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"big\",\"Body\":\"x\"}\n");
    }
    input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"big\",\"Body\":\"x\"}\n");
    input.push_str("{\"Command\":\"GetQueueAttributes\",\"QueueName\":\"big\"}\n");
    let responses = run_raw(input.as_bytes());
    assert_eq!(code(&responses[100_001]), "LimitExceeded");
    assert_eq!(
        counts(&responses[100_002]),
        json!({"Visible": 100_000, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn the_binary_answers_on_stdout_and_exits_with_zero() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":3}\n")
        .expect("miniq reads stdin");
    let output = child.wait_with_output().expect("miniq exits");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"{\"ok\":true,\"Now\":3}\n");
}
