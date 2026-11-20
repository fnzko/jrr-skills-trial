mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::{fail, ok, run_lines};
use serde_json::json;

#[test]
fn every_line_gets_exactly_one_response_including_blank_and_final_unterminated() {
    let responses = run_lines(
        "\n{\"Command\":\"Tick\",\"Seconds\":1}\nnot json\n{\"Command\":\"Tick\",\"Seconds\":2}",
    );
    assert_eq!(
        responses,
        [
            fail("InvalidRequest"),
            json!({"ok": true, "Now": 1}),
            fail("InvalidRequest"),
            json!({"ok": true, "Now": 3}),
        ]
    );
}

#[test]
fn an_empty_input_gets_no_response_but_a_lone_newline_gets_one() {
    assert_eq!(run_lines(""), Vec::<serde_json::Value>::new());
    assert_eq!(run_lines("\n"), [fail("InvalidRequest")]);
}

#[test]
fn invalid_utf8_is_an_invalid_request_and_the_next_line_still_runs() {
    let mut input = b"{\"Command\":\"Tick\",\"Seconds\":1}\xff\n".to_vec();
    input.extend_from_slice(b"{\"Command\":\"Tick\",\"Seconds\":1}\n");
    let mut output = Vec::new();
    miniq::serve(&input[..], &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("\"InvalidRequest\""));
    assert_eq!(lines[1], r#"{"Now":1,"ok":true}"#);
}

#[test]
fn unknown_members_are_ignored_and_a_failed_command_changes_nothing() {
    let responses = run_lines(concat!(
        "{\"Command\":\"CreateQueue\",\"QueueName\":\"q\",\"Extra\":[1]}\n",
        "{\"Command\":\"SetQueueAttributes\",\"QueueName\":\"q\",\"Attributes\":{\"DelaySeconds\":7,\"Bogus\":1}}\n",
        "{\"Command\":\"GetQueueAttributes\",\"QueueName\":\"q\"}\n",
    ));
    assert_eq!(responses[0], ok());
    assert_eq!(responses[1], fail("InvalidParameterValue"));
    assert_eq!(responses[2]["Attributes"]["DelaySeconds"], 0);
}

#[test]
fn the_binary_answers_on_stdout_and_exits_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":3}\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"{\"Now\":3,\"ok\":true}\n");
}
