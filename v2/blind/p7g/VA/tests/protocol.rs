mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::Session;
use serde_json::{Value, json};

#[test]
fn malformed_lines_are_invalid_requests() {
    let mut session = Session::new();
    let lines: [&[u8]; 9] = [
        b"",
        b"   ",
        b"not json",
        b"[1]",
        b"\"CreateQueue\"",
        b"{\"Command\": 1}",
        b"{\"Command\": null}",
        b"{\"Command\": \"Nope\"}",
        b"{\"Command\": \"Tick\"} trailing",
    ];
    for line in lines {
        let response = session.raw(line);
        assert_eq!(response["ok"], json!(false), "{line:?}");
        assert_eq!(
            response["error"]["code"],
            json!("InvalidRequest"),
            "{line:?}"
        );
    }
    assert_eq!(
        session.raw(b"{\"Command\": \"Tick\", \"Seconds\": 0xff}")["error"]["code"],
        json!("InvalidRequest")
    );
    assert_eq!(
        session.raw(b"{\"Command\": \"\xff\"}")["error"]["code"],
        json!("InvalidRequest")
    );
}

#[test]
fn command_names_are_case_sensitive_and_extra_members_are_ignored() {
    let mut session = Session::new();
    assert_eq!(
        session.err(json!({"Command": "tick", "Seconds": 1})),
        "InvalidRequest"
    );
    assert_eq!(
        session.ok(json!({"Command": "Tick", "Seconds": 5, "Extra": [1, 2]})),
        json!({"Now": 5})
    );
    assert_eq!(
        session.err(json!({"Command": "Tick", "seconds": 5})),
        "MissingParameter"
    );
}

#[test]
fn null_parameters_count_as_absent() {
    let mut session = Session::new();
    assert_eq!(
        session.err(json!({"Command": "Tick", "Seconds": null})),
        "MissingParameter"
    );
    session.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": null}));
    session.ok(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": null}}));
    assert_eq!(
        session.ok(json!({"Command": "ListQueues", "Prefix": null})),
        json!({"QueueNames": ["q"]})
    );
}

#[test]
fn integers_must_be_written_as_integers() {
    let mut session = Session::new();
    for seconds in ["30.0", "3e1", "\"30\"", "-1", "true", "1209601"] {
        let line = format!("{{\"Command\": \"Tick\", \"Seconds\": {seconds}}}");
        assert_eq!(
            session.raw(line.as_bytes())["error"]["code"],
            json!("InvalidParameterValue"),
            "{seconds}"
        );
    }
    let response = session.raw(b"{\"Command\": \"Tick\", \"Seconds\": -0}");
    assert_eq!(response, json!({"ok": true, "Now": 0}));
}

#[test]
fn checks_run_in_the_documented_order() {
    let mut session = Session::new();
    // Missing parameters beat bad values of other parameters.
    assert_eq!(
        session.err(json!({"Command": "SendMessage", "QueueName": 5})),
        "MissingParameter"
    );
    // Value errors beat the queue lookup.
    assert_eq!(
        session.err(json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""})),
        "InvalidParameterValue"
    );
    // The lookup beats command rules, whatever the name looks like.
    assert_eq!(
        session.err(json!({"Command": "PurgeQueue", "QueueName": "not a name!"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        session.err(
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": "x", "GroupId": "g"})
        ),
        "QueueDoesNotExist"
    );
    // CreateQueue checks the name before anything else.
    assert_eq!(
        session.err(json!({"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3})),
        "QueueNameInvalid"
    );
    assert_eq!(
        session.err(json!({"Command": "CreateQueue", "QueueName": 3, "Attributes": 3})),
        "InvalidParameterValue"
    );
    assert_eq!(
        session.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": 3})),
        "InvalidParameterValue"
    );
}

#[test]
fn failed_commands_change_nothing() {
    let mut session = Session::new();
    session.create("q", json!({}));
    assert_eq!(
        session.err(json!({"Command": "SetQueueAttributes", "QueueName": "q",
            "Attributes": {"VisibilityTimeout": 5, "DelaySeconds": 901}})),
        "InvalidParameterValue"
    );
    let attributes = session.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attributes["Attributes"]["VisibilityTimeout"], json!(30));
}

fn run_binary(input: &[u8]) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        output.status.code(),
    )
}

#[test]
fn binary_answers_every_line_including_blank_ones_and_an_unterminated_last_line() {
    let (stdout, code) =
        run_binary(b"{\"Command\":\"Tick\",\"Seconds\":2}\n\n{\"Command\":\"Tick\",\"Seconds\":3}");
    assert_eq!(code, Some(0));
    let responses: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        responses,
        vec![
            json!({"ok": true, "Now": 2}),
            json!({"ok": false, "error": {"code": "InvalidRequest", "message": responses[1]["error"]["message"]}}),
            json!({"ok": true, "Now": 5}),
        ]
    );
    assert!(stdout.ends_with('\n'));
}

#[test]
fn binary_prints_nothing_for_empty_input_and_ignores_a_trailing_newline() {
    assert_eq!(run_binary(b""), (String::new(), Some(0)));
    assert_eq!(run_binary(b"\n").0.lines().count(), 1);
}

#[test]
fn binary_survives_invalid_utf8() {
    let (stdout, code) = run_binary(b"\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":1}\n");
    assert_eq!(code, Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("InvalidRequest"));
    assert_eq!(
        serde_json::from_str::<Value>(lines[1]).unwrap(),
        json!({"ok": true, "Now": 1})
    );
}
