use crate::common::*;
use serde_json::json;

#[test]
fn every_line_gets_one_response_in_order() {
    let responses = run_text(b"\n{\"Command\": \"Tick\", \"Seconds\": 2}\nnot json\n{\"Command\": \"Tick\", \"Seconds\": 3}");
    assert_eq!(responses.len(), 4);
    assert_error(&responses[0], "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 2}));
    assert_error(&responses[2], "InvalidRequest");
    assert_eq!(responses[3], json!({"ok": true, "Now": 5}));
}

#[test]
fn no_input_gives_no_output() {
    assert_eq!(run_text(b""), Vec::<serde_json::Value>::new());
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_next_line_still_runs() {
    let responses = run_text(
        b"{\"Command\": \"Tick\", \"Seconds\": \xff}\n{\"Command\": \"Tick\", \"Seconds\": 1}\n",
    );
    assert_error(&responses[0], "InvalidRequest");
    assert_eq!(responses[1], json!({"ok": true, "Now": 1}));
}

#[test]
fn unknown_members_are_ignored_and_null_means_absent() {
    let responses = session(&[
        json!({"Command": "Tick", "Seconds": 1, "Extra": [1, 2]}),
        json!({"Command": "ListQueues", "Prefix": null}),
        json!({"Command": "Tick", "Seconds": null}),
    ]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 1}));
    assert_eq!(responses[1], json!({"ok": true, "QueueNames": []}));
    assert_error(&responses[2], "MissingParameter");
}

#[test]
fn the_binary_answers_on_stdout() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\": \"Tick\", \"Seconds\": 7}\n")
        .expect("the binary reads stdin");
    let output = child.wait_with_output().expect("the binary exits");
    assert!(output.status.success());
    let lines: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .expect("UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("JSON"))
        .collect();
    assert_eq!(lines, vec![json!({"ok": true, "Now": 7})]);
}
