use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::json;

use crate::common::{check, create, fail, ok, responses, send, sent, tick};

#[test]
fn every_line_gets_one_response_in_order() {
    let input = b"\n[1]\n{\"Command\":\"Tick\",\"Seconds\":2}\n\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":3}";
    assert_eq!(
        responses(input),
        vec![
            fail("InvalidRequest"),
            fail("InvalidRequest"),
            json!({"ok": true, "Now": 2}),
            fail("InvalidRequest"),
            json!({"ok": true, "Now": 5}),
        ]
    );
}

#[test]
fn empty_input_and_trailing_newline_add_no_response() {
    assert_eq!(responses(b""), Vec::<serde_json::Value>::new());
    assert_eq!(
        responses(b"{\"Command\":\"Tick\",\"Seconds\":1}\n"),
        vec![json!({"ok": true, "Now": 1})]
    );
}

#[test]
fn request_shape_errors() {
    check(&[
        (
            json!({"command": "Tick", "Seconds": 1}),
            fail("InvalidRequest"),
        ),
        (json!({"Command": null}), fail("InvalidRequest")),
        (json!({"Command": ["Tick"]}), fail("InvalidRequest")),
        (json!({"Command": "Purge"}), fail("InvalidRequest")),
        (json!("Tick"), fail("InvalidRequest")),
    ]);
}

#[test]
fn unknown_members_are_ignored_and_null_means_absent() {
    check(&[
        (
            json!({"Command": "Tick", "Seconds": 1, "Verbose": true}),
            json!({"ok": true, "Now": 1}),
        ),
        (
            json!({"Command": "Tick", "Seconds": null}),
            fail("MissingParameter"),
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": null}}),
            ok(),
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": null}),
            sent(1),
        ),
    ]);
}

#[test]
fn integers_must_be_written_without_fraction_or_exponent() {
    let input = b"{\"Command\":\"Tick\",\"Seconds\":1.0}\n\
        {\"Command\":\"Tick\",\"Seconds\":1e0}\n\
        {\"Command\":\"Tick\",\"Seconds\":\"1\"}\n\
        {\"Command\":\"Tick\",\"Seconds\":-0}\n\
        {\"Command\":\"Tick\",\"Seconds\":1209600}\n\
        {\"Command\":\"Tick\",\"Seconds\":1209601}\n";
    assert_eq!(
        responses(input),
        vec![
            fail("InvalidParameterValue"),
            fail("InvalidParameterValue"),
            fail("InvalidParameterValue"),
            json!({"ok": true, "Now": 0}),
            json!({"ok": true, "Now": 1_209_600}),
            fail("InvalidParameterValue"),
        ]
    );
}

#[test]
fn checks_run_in_the_specified_order() {
    check(&[
        // Required presence beats a bad value of another parameter.
        (
            json!({"Command": "SendMessage", "QueueName": 1}),
            fail("MissingParameter"),
        ),
        // A bad value beats an unknown queue.
        (
            json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""}),
            fail("InvalidParameterValue"),
        ),
        (send("nope", "b"), fail("QueueDoesNotExist")),
        // Queue lookup beats the receipt handle checks.
        (
            json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "x"}),
            fail("QueueDoesNotExist"),
        ),
        // CreateQueue checks the name before other parameters.
        (
            json!({"Command": "CreateQueue", "QueueName": "a b", "Attributes": 3}),
            fail("QueueNameInvalid"),
        ),
        // Any string is looked up as given.
        (
            json!({"Command": "PurgeQueue", "QueueName": "a b"}),
            fail("QueueDoesNotExist"),
        ),
        (create("q"), ok()),
        (
            json!({"Command": "PurgeQueue", "QueueName": "Q"}),
            fail("QueueDoesNotExist"),
        ),
        (tick(0), json!({"ok": true, "Now": 0})),
    ]);
}

#[test]
fn binary_answers_each_line_and_exits_zero_at_end_of_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":4}\n{\"Command\":\"ListQueues\"}")
        .expect("miniq reads its input");
    let output = child.wait_with_output().expect("miniq runs to completion");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"ok\":true,\"Now\":4}\n{\"ok\":true,\"QueueNames\":[]}\n"
    );
}
