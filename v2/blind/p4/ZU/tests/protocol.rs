use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Runs `input` through `miniq::run` and returns the parsed response lines.
fn run(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.is_empty() || text.ends_with('\n'), "output {text:?}");
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn code(response: &Value) -> &str {
    response["error"]["code"].as_str().unwrap()
}

#[test]
fn every_line_gets_one_response_in_order() {
    let input = b"{\"Command\": \"Tick\", \"Seconds\": 2}\n\n   \n\xff\xfe\n{\"Command\": \"Tick\", \"Seconds\": 3}\n";
    let responses = run(input);
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0], json!({"ok": true, "Now": 2}));
    for response in &responses[1..4] {
        assert_eq!(code(response), "InvalidRequest", "response {response}");
    }
    assert_eq!(responses[4], json!({"ok": true, "Now": 5}));
}

#[test]
fn final_line_without_newline_counts() {
    let responses =
        run(b"{\"Command\": \"Tick\", \"Seconds\": 1}\n{\"Command\": \"Tick\", \"Seconds\": 1}");
    assert_eq!(
        responses,
        [json!({"ok": true, "Now": 1}), json!({"ok": true, "Now": 2})]
    );
}

#[test]
fn empty_input_gives_no_output() {
    assert_eq!(run(b""), Vec::<Value>::new());
}

#[test]
fn failed_command_changes_no_state() {
    let input = concat!(
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 5, "Bogus": 1}}"#,
        "\n",
        r#"{"Command": "ListQueues"}"#,
        "\n",
    );
    let responses = run(input.as_bytes());
    assert_eq!(code(&responses[0]), "InvalidParameterValue");
    assert_eq!(responses[1], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn binary_answers_stdin_on_stdout_and_exits_with_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = concat!(
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
        "\n",
        r#"{"Command": "SendMessage", "QueueName": "jobs", "Body": "hi"}"#,
        "\n",
        "oops\n",
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], json!({"ok": true}));
    assert_eq!(lines[1], json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(code(&lines[2]), "InvalidRequest");
}
