//! Scenario tests through the broker. Each test sends command lines in order and compares every
//! response as a whole JSON value, so the member names and their absence are checked too.

use miniq::broker::Broker;
use serde_json::{json, Value};

/// Sends one command and returns its response as JSON.
fn exec(broker: &mut Broker, command: Value) -> Value {
    let line = command.to_string();
    serde_json::to_value(broker.handle_line(line.as_bytes())).expect("replies serialize to JSON")
}

/// The failure code of a response, if it is a failure.
fn code(reply: &Value) -> Option<&str> {
    reply.get("error")?.get("code")?.as_str()
}

#[test]
fn a_new_queue_reports_default_attributes_and_no_messages() {
    let mut broker = Broker::default();
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "CreateQueue", "QueueName": "jobs"})
        ),
        json!({"ok": true})
    );
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "GetQueueAttributes", "QueueName": "jobs"})
        ),
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn creating_an_existing_queue_succeeds_only_with_identical_attributes() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "jobs"}),
    );
    let same = json!({"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 30}});
    assert_eq!(exec(&mut broker, same), json!({"ok": true}));
    let different = json!({"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 31}});
    assert_eq!(
        code(&exec(&mut broker, different)),
        Some("QueueAlreadyExists")
    );
}

#[test]
fn rejected_commands_report_the_first_failing_check() {
    let create = |name: &str, attributes: Value| json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes});
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (vec![create("a.fifo", json!({}))], "QueueNameInvalid"),
        (vec![create("a", json!({"Fifo": true}))], "QueueNameInvalid"),
        (
            vec![create("a", json!({"ContentBasedDeduplication": true}))],
            "InvalidParameterValue",
        ),
        (
            vec![create("a", json!({"DeadLetterQueue": "d"}))],
            "InvalidParameterValue",
        ),
        (
            vec![create(
                "a",
                json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2}),
            )],
            "QueueDoesNotExist",
        ),
        (
            vec![
                create("std", json!({})),
                create("f.fifo", json!({"Fifo": true})),
                create(
                    "g.fifo",
                    json!({"Fifo": true, "DeadLetterQueue": "std", "MaxReceiveCount": 2}),
                ),
            ],
            "InvalidParameterValue",
        ),
        (
            vec![
                json!({"Command": "SetQueueAttributes", "QueueName": "missing", "Attributes": {}}),
            ],
            "QueueDoesNotExist",
        ),
        (
            vec![
                create("a", json!({})),
                json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"Fifo": true}}),
            ],
            "UnsupportedOperation",
        ),
        (
            vec![
                create("a", json!({})),
                json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": {"DeadLetterQueue": "a", "MaxReceiveCount": 2}}),
            ],
            "InvalidParameterValue",
        ),
        (
            vec![
                create("a", json!({})),
                json!({"Command": "SendMessage", "QueueName": "a", "Body": "x", "GroupId": "g"}),
            ],
            "UnsupportedOperation",
        ),
        (
            vec![
                create(
                    "f.fifo",
                    json!({"Fifo": true, "ContentBasedDeduplication": true}),
                ),
                json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x"}),
            ],
            "MissingParameter",
        ),
        (
            vec![
                create("f.fifo", json!({"Fifo": true})),
                json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"}),
            ],
            "MissingParameter",
        ),
        (
            vec![
                create(
                    "f.fifo",
                    json!({"Fifo": true, "ContentBasedDeduplication": true}),
                ),
                json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g", "DelaySeconds": 1}),
            ],
            "UnsupportedOperation",
        ),
        (
            vec![
                create("a", json!({})),
                json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": "r-1"}),
            ],
            "ReceiptHandleInvalid",
        ),
        (
            vec![
                create("a", json!({})),
                json!({"Command": "DeleteMessage", "QueueName": "a", "ReceiptHandle": "r-01"}),
            ],
            "ReceiptHandleInvalid",
        ),
        (
            vec![json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "r-1"})],
            "QueueDoesNotExist",
        ),
        (
            vec![json!({"Command": "ReceiveMessage", "QueueName": "nope"})],
            "QueueDoesNotExist",
        ),
        (
            vec![
                create("a", json!({})),
                json!({"Command": "ReceiveMessage", "QueueName": "a", "MaxMessages": 11}),
            ],
            "InvalidParameterValue",
        ),
        (
            vec![json!({"Command": "Tick", "Seconds": 1_209_601})],
            "InvalidParameterValue",
        ),
        (vec![json!({"Command": "Nope"})], "InvalidRequest"),
    ];
    for (setup, expected) in cases {
        let (last, before) = setup.split_last().expect("every case has a command");
        let mut broker = Broker::default();
        for command in before {
            exec(&mut broker, command.clone());
        }
        let reply = exec(&mut broker, last.clone());
        assert_eq!(code(&reply), Some(expected), "{last}");
    }
}

#[test]
fn queue_names_are_listed_in_byte_order_and_filtered_by_prefix() {
    let mut broker = Broker::default();
    for name in ["b", "B", "a_1", "a-1", "a1"] {
        exec(
            &mut broker,
            json!({"Command": "CreateQueue", "QueueName": name}),
        );
    }
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true}}),
    );
    assert_eq!(
        exec(&mut broker, json!({"Command": "ListQueues"})),
        json!({"ok": true, "QueueNames": ["B", "a-1", "a.fifo", "a1", "a_1", "b"]})
    );
    assert_eq!(
        exec(&mut broker, json!({"Command": "ListQueues", "Prefix": "a"})),
        json!({"ok": true, "QueueNames": ["a-1", "a.fifo", "a1", "a_1"]})
    );
    assert_eq!(
        exec(&mut broker, json!({"Command": "ListQueues", "Prefix": "z"})),
        json!({"ok": true, "QueueNames": []})
    );
}

#[test]
fn tick_advances_the_clock_and_stops_at_its_limit() {
    let mut broker = Broker::default();
    // 3306 ticks of the largest allowed length, then the remainder that reaches 4000000000.
    for _ in 0..3306 {
        exec(
            &mut broker,
            json!({"Command": "Tick", "Seconds": 1_209_600}),
        );
    }
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "Tick", "Seconds": 1_062_400})
        ),
        json!({"ok": true, "Now": 4_000_000_000u64})
    );
    assert_eq!(
        code(&exec(&mut broker, json!({"Command": "Tick", "Seconds": 1}))),
        Some("InvalidParameterValue")
    );
}

#[test]
fn a_message_is_sent_received_and_deleted_once() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "q"}),
    );
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "hello"})
        ),
        json!({"ok": true, "MessageId": "m-1"})
    );
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "ReceiveMessage", "QueueName": "q"})
        ),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "hello",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 0
        }]})
    );
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "ReceiveMessage", "QueueName": "q"})
        ),
        json!({"ok": true, "Messages": []})
    );
    let delete = json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"});
    assert_eq!(exec(&mut broker, delete.clone()), json!({"ok": true}));
    assert_eq!(
        code(&exec(&mut broker, delete)),
        Some("ReceiptHandleExpired")
    );
}

#[test]
fn a_visibility_timeout_hides_a_message_until_it_ends_and_then_redelivers_it() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 10}}),
    );
    exec(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "hello"}),
    );
    exec(
        &mut broker,
        json!({"Command": "ReceiveMessage", "QueueName": "q"}),
    );
    exec(&mut broker, json!({"Command": "Tick", "Seconds": 9}));
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "ReceiveMessage", "QueueName": "q"})
        ),
        json!({"ok": true, "Messages": []})
    );
    exec(&mut broker, json!({"Command": "Tick", "Seconds": 1}));
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "ReceiveMessage", "QueueName": "q"})
        ),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-2",
            "Body": "hello",
            "ReceiveCount": 2,
            "SentAt": 0,
            "FirstReceivedAt": 0
        }]})
    );
    let old = json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"});
    assert_eq!(code(&exec(&mut broker, old)), Some("ReceiptHandleExpired"));
}

#[test]
fn changing_visibility_to_zero_makes_the_message_visible_at_once() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "q"}),
    );
    exec(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "hello"}),
    );
    exec(
        &mut broker,
        json!({"Command": "ReceiveMessage", "QueueName": "q"}),
    );
    let change = |seconds: u32| {
        json!({
            "Command": "ChangeMessageVisibility",
            "QueueName": "q",
            "ReceiptHandle": "r-1",
            "VisibilityTimeout": seconds
        })
    };
    assert_eq!(exec(&mut broker, change(5)), json!({"ok": true}));
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "ReceiveMessage", "QueueName": "q"})
        ),
        json!({"ok": true, "Messages": []})
    );
    assert_eq!(exec(&mut broker, change(0)), json!({"ok": true}));
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "ReceiveMessage", "QueueName": "q"})
        ),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-2",
            "Body": "hello",
            "ReceiveCount": 2,
            "SentAt": 0,
            "FirstReceivedAt": 0
        }]})
    );
    assert_eq!(
        code(&exec(&mut broker, change(0))),
        Some("ReceiptHandleExpired")
    );
}

#[test]
fn a_fifo_duplicate_returns_the_original_until_the_window_closes() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}),
    );
    let send =
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x", "GroupId": "g"});
    assert_eq!(
        exec(&mut broker, send.clone()),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(
        exec(&mut broker, send.clone()),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    exec(&mut broker, json!({"Command": "Tick", "Seconds": 299}));
    assert_eq!(
        exec(&mut broker, send.clone()),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    exec(&mut broker, json!({"Command": "Tick", "Seconds": 1}));
    assert_eq!(
        exec(&mut broker, send),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn a_fifo_receive_returns_at_most_one_message_per_group_with_its_group_and_sequence() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}),
    );
    for (body, group) in [("a", "g1"), ("b", "g1"), ("c", "g2")] {
        exec(
            &mut broker,
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group}),
        );
    }
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10})
        ),
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g1", "SequenceNumber": 1},
            {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "c", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g2", "SequenceNumber": 3}
        ]})
    );
}

#[test]
fn a_message_past_its_delivery_limit_moves_to_the_dead_letter_queue() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "dead"}),
    );
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1}}),
    );
    exec(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "hello"}),
    );
    let attempt = json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0});
    assert_eq!(
        exec(&mut broker, attempt.clone())["Messages"][0]["ReceiveCount"],
        1
    );
    assert_eq!(
        exec(&mut broker, attempt),
        json!({"ok": true, "Messages": []}),
        "the second attempt moves the message instead of delivering it"
    );
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "ReceiveMessage", "QueueName": "dead"})
        ),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-2",
            "Body": "hello",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 0
        }]})
    );
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_references_to_it() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "dead"}),
    );
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 3}}),
    );
    exec(
        &mut broker,
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
    );
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "GetQueueAttributes", "QueueName": "q"})
        )["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false
        })
    );
}

#[test]
fn lowering_retention_removes_messages_already_past_it() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "q"}),
    );
    exec(
        &mut broker,
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "old"}),
    );
    exec(&mut broker, json!({"Command": "Tick", "Seconds": 100}));
    exec(
        &mut broker,
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}),
    );
    assert_eq!(
        exec(
            &mut broker,
            json!({"Command": "GetQueueAttributes", "QueueName": "q"})
        )["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_standard_queue_takes_at_most_one_hundred_thousand_messages() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "q"}),
    );
    let send = json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"});
    for _ in 0..100_000 {
        assert_eq!(exec(&mut broker, send.clone())["ok"], true);
    }
    assert_eq!(code(&exec(&mut broker, send)), Some("LimitExceeded"));
}

#[test]
fn the_broker_holds_at_most_one_thousand_queues() {
    let mut broker = Broker::default();
    for n in 0..1000 {
        let name = format!("q{n}");
        exec(
            &mut broker,
            json!({"Command": "CreateQueue", "QueueName": name}),
        );
    }
    assert_eq!(
        code(&exec(
            &mut broker,
            json!({"Command": "CreateQueue", "QueueName": "extra"})
        )),
        Some("LimitExceeded")
    );
}

#[test]
fn purging_removes_every_message_and_keeps_counting_sequence_numbers() {
    let mut broker = Broker::default();
    exec(
        &mut broker,
        json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "ContentBasedDeduplication": true}}),
    );
    let send = |body: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g"});
    exec(&mut broker, send("a"));
    exec(
        &mut broker,
        json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}),
    );
    assert_eq!(
        exec(&mut broker, send("b")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}
