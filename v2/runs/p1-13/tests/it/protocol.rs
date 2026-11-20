use crate::common::{code, ok, run, run_bytes};
use serde_json::{Value, json};

#[test]
fn every_line_gets_one_response_in_order() {
    let input = b"\n[1]\n{\"Command\": 5}\n{\"Command\":\"Nope\"}\n{\"Command\":\"Tick\",\"Seconds\":1} trailing\n\xff\xfe\n{\"Command\":\"Tick\",\"Seconds\":2}";
    let responses = run_bytes(input);
    let codes: Vec<Option<&str>> = responses
        .iter()
        .map(|r| r["error"]["code"].as_str())
        .collect();
    assert_eq!(
        codes,
        [
            Some("InvalidRequest"),
            Some("InvalidRequest"),
            Some("InvalidRequest"),
            Some("InvalidRequest"),
            Some("InvalidRequest"),
            Some("InvalidRequest"),
            None,
        ]
    );
    assert_eq!(responses[6], json!({"ok": true, "Now": 2}));
}

#[test]
fn empty_input_gets_no_response() {
    assert_eq!(run_bytes(b""), Vec::<Value>::new());
}

#[test]
fn null_and_unknown_members_are_ignored() {
    let responses = run(&[
        json!({"Command": "ListQueues", "Prefix": null, "Extra": 1}),
        json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": null}),
        json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": {"VisibilityTimeout": null}}),
    ]);
    assert_eq!(
        responses,
        [json!({"ok": true, "QueueNames": []}), ok(), ok()]
    );
}

#[test]
fn integers_must_be_written_as_integers() {
    let lines = [
        r#"{"Command":"Tick","Seconds":30.0}"#,
        r#"{"Command":"Tick","Seconds":3e1}"#,
        r#"{"Command":"Tick","Seconds":"30"}"#,
        r#"{"Command":"Tick","Seconds":-1}"#,
        r#"{"Command":"Tick","Seconds":1209601}"#,
        r#"{"Command":"Tick","Seconds":99999999999999999999}"#,
        r#"{"Command":"Tick","Seconds":-0}"#,
    ];
    let responses = run_bytes(lines.join("\n").as_bytes());
    let codes: Vec<Option<&str>> = responses
        .iter()
        .map(|r| r["error"]["code"].as_str())
        .collect();
    let mut expected = vec![Some("InvalidParameterValue"); 6];
    expected.push(None);
    assert_eq!(codes, expected);
    assert_eq!(responses[6], json!({"ok": true, "Now": 0}));
}

#[test]
fn first_failing_check_decides_the_code() {
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (vec![json!({"Command": "Tick"})], "MissingParameter"),
        (
            vec![json!({"Command": "Tick", "Seconds": null})],
            "MissingParameter",
        ),
        (
            vec![json!({"Command": "SendMessage", "Body": 5})],
            "MissingParameter",
        ),
        (
            vec![json!({"Command": "ReceiveMessage", "QueueName": "nope", "MaxMessages": 11})],
            "InvalidParameterValue",
        ),
        (
            vec![json!({"Command": "ReceiveMessage", "QueueName": "no such!"})],
            "QueueDoesNotExist",
        ),
        (
            vec![json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "junk"})],
            "QueueDoesNotExist",
        ),
        (
            vec![json!({"Command": "CreateQueue", "QueueName": "x y", "Attributes": 5})],
            "QueueNameInvalid",
        ),
        (
            vec![json!({"Command": "CreateQueue", "QueueName": 5})],
            "InvalidParameterValue",
        ),
        (
            vec![json!({"Command": "CreateQueue", "QueueName": null})],
            "MissingParameter",
        ),
        (
            vec![json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": []})],
            "InvalidParameterValue",
        ),
        (
            vec![
                json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Bogus": null}}),
            ],
            "InvalidParameterValue",
        ),
        (
            vec![crate::common::create(
                "a",
                json!({"VisibilityTimeout": 43201}),
            )],
            "InvalidParameterValue",
        ),
        (
            vec![crate::common::create("a", json!({"RetentionSeconds": 59}))],
            "InvalidParameterValue",
        ),
        (
            vec![crate::common::create("a.fifo", json!({}))],
            "QueueNameInvalid",
        ),
        (
            vec![crate::common::create("a", json!({"Fifo": true}))],
            "QueueNameInvalid",
        ),
        (
            vec![crate::common::create(
                "a",
                json!({"ContentBasedDeduplication": true}),
            )],
            "InvalidParameterValue",
        ),
        (
            vec![crate::common::create("a", json!({"MaxReceiveCount": 3}))],
            "InvalidParameterValue",
        ),
        (
            vec![crate::common::create(
                "a",
                json!({"DeadLetterQueue": "d", "MaxReceiveCount": 3}),
            )],
            "QueueDoesNotExist",
        ),
        (
            vec![
                crate::common::create("d.fifo", json!({"Fifo": true})),
                crate::common::create(
                    "a",
                    json!({"DeadLetterQueue": "d.fifo", "MaxReceiveCount": 3}),
                ),
            ],
            "InvalidParameterValue",
        ),
        (
            vec![
                crate::common::create("a", json!({})),
                crate::common::create("a", json!({"DelaySeconds": 1})),
            ],
            "QueueAlreadyExists",
        ),
        (
            vec![
                crate::common::create("a", json!({})),
                json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"Fifo": false}}),
            ],
            "UnsupportedOperation",
        ),
        (
            vec![
                crate::common::create("a", json!({})),
                json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"ContentBasedDeduplication": true}}),
            ],
            "InvalidParameterValue",
        ),
        (
            vec![
                crate::common::create("a", json!({})),
                json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DeadLetterQueue": "a", "MaxReceiveCount": 2}}),
            ],
            "InvalidParameterValue",
        ),
        (
            vec![
                crate::common::create("a", json!({})),
                json!({"Command": "SetQueueAttributes", "QueueName": "a"}),
            ],
            "MissingParameter",
        ),
        (
            vec![
                crate::common::create("a", json!({})),
                crate::common::send("a", ""),
            ],
            "InvalidParameterValue",
        ),
        (
            vec![
                crate::common::create("a", json!({})),
                json!({"Command": "SendMessage", "QueueName": "a", "Body": "x", "GroupId": "g"}),
            ],
            "UnsupportedOperation",
        ),
        (
            vec![
                crate::common::create("a.fifo", json!({"Fifo": true})),
                json!({"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "DeduplicationId": "d", "DelaySeconds": 1}),
            ],
            "UnsupportedOperation",
        ),
        (
            vec![
                crate::common::create("a.fifo", json!({"Fifo": true})),
                json!({"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "DeduplicationId": "d"}),
            ],
            "MissingParameter",
        ),
        (
            vec![
                crate::common::create("a.fifo", json!({"Fifo": true})),
                json!({"Command": "SendMessage", "QueueName": "a.fifo", "Body": "x", "GroupId": "g"}),
            ],
            "MissingParameter",
        ),
    ];
    for (commands, expected) in cases {
        let responses = run(&commands);
        let last = responses.last().expect("each case has a command");
        assert_eq!(code(last), expected, "commands {commands:?}");
    }
}

#[test]
fn a_failed_command_changes_no_state() {
    let responses = run(&[
        crate::common::create("a", json!({})),
        crate::common::create("a", json!({"VisibilityTimeout": 5})),
        crate::common::send("a", "x"),
        json!({"Command": "SendMessage", "QueueName": "a", "Body": "y", "GroupId": "g"}),
        crate::common::send("a", "z"),
    ]);
    assert_eq!(responses[3]["error"]["code"], json!("UnsupportedOperation"));
    assert_eq!(responses[4], json!({"ok": true, "MessageId": "m-2"}));
}
