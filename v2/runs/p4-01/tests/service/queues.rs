use serde_json::json;

use crate::common::{
    check, counts, create, create_with, delete_message, fail, get_attributes, messages, ok,
    receive, responses, send, sent, tick,
};

fn default_attributes() -> serde_json::Value {
    json!({
        "VisibilityTimeout": 30,
        "DelaySeconds": 0,
        "RetentionSeconds": 345_600,
        "Fifo": false,
        "ContentBasedDeduplication": false,
    })
}

fn set(queue: &str, attributes: serde_json::Value) -> serde_json::Value {
    json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes})
}

#[test]
fn create_queue_rules() {
    check(&[
        (create("q.fifo"), fail("QueueNameInvalid")),
        (
            create_with("q", json!({"Fifo": true})),
            fail("QueueNameInvalid"),
        ),
        (
            create_with("q", json!({"ContentBasedDeduplication": true})),
            fail("InvalidParameterValue"),
        ),
        (
            create_with("q", json!({"MaxReceiveCount": 3})),
            fail("InvalidParameterValue"),
        ),
        (
            create_with("q", json!({"DeadLetterQueue": "d"})),
            fail("InvalidParameterValue"),
        ),
        (
            create_with("q", json!({"Colour": "red"})),
            fail("InvalidParameterValue"),
        ),
        (
            create_with("q", json!({"DelaySeconds": 901})),
            fail("InvalidParameterValue"),
        ),
        (
            create_with("q", json!({"DeadLetterQueue": "d", "MaxReceiveCount": 3})),
            fail("QueueDoesNotExist"),
        ),
        (create("q"), ok()),
        // Same attributes after defaults: succeeds and changes nothing.
        (
            create_with("q", json!({"VisibilityTimeout": 30, "Fifo": false})),
            ok(),
        ),
        (
            create_with("q", json!({"VisibilityTimeout": 31})),
            fail("QueueAlreadyExists"),
        ),
        // The existing queue is compared before the dead-letter queue is looked up.
        (
            create_with(
                "q",
                json!({"DeadLetterQueue": "none", "MaxReceiveCount": 1}),
            ),
            fail("QueueAlreadyExists"),
        ),
        (create_with("d.fifo", json!({"Fifo": true})), ok()),
        (
            create_with(
                "s",
                json!({"DeadLetterQueue": "d.fifo", "MaxReceiveCount": 3}),
            ),
            fail("InvalidParameterValue"),
        ),
        (
            create_with("s", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 3})),
            ok(),
        ),
        (
            create_with("s", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 3})),
            ok(),
        ),
        (
            create_with("s", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 4})),
            fail("QueueAlreadyExists"),
        ),
        (
            create_with("q", json!({"DeadLetterQueue": "s", "MaxReceiveCount": 4})),
            fail("QueueAlreadyExists"),
        ),
        (
            get_attributes("s"),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345_600,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "q", "MaxReceiveCount": 3,
            }, "Counts": counts(0, 0, 0)}),
        ),
    ]);
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut input: String = (0..1000)
        .map(|i| format!("{}\n", create(&format!("q{i}"))))
        .collect();
    input.push_str(&format!("{}\n{}\n", create("q0"), create("extra")));
    let responses = responses(input.as_bytes());
    assert_eq!(responses.len(), 1002);
    assert!(responses[..1000].iter().all(|r| *r == ok()));
    assert_eq!(responses[1000..], [ok(), fail("LimitExceeded")]);
}

#[test]
fn set_queue_attributes_rules() {
    check(&[
        (create("q"), ok()),
        (create("other"), ok()),
        (create_with("f.fifo", json!({"Fifo": true})), ok()),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
            fail("MissingParameter"),
        ),
        (set("nope", json!({})), fail("QueueDoesNotExist")),
        (
            set("q", json!({"Fifo": false})),
            fail("UnsupportedOperation"),
        ),
        (set("q", json!({"Fifo": 1})), fail("InvalidParameterValue")),
        (
            set("q", json!({"ContentBasedDeduplication": true})),
            fail("InvalidParameterValue"),
        ),
        (
            set("q", json!({"MaxReceiveCount": 2})),
            fail("InvalidParameterValue"),
        ),
        (
            set(
                "q",
                json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 2}),
            ),
            fail("QueueDoesNotExist"),
        ),
        (
            set("q", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})),
            fail("InvalidParameterValue"),
        ),
        (
            set(
                "q",
                json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}),
            ),
            fail("InvalidParameterValue"),
        ),
        // A failed command changes nothing, even attributes it could have set.
        (
            set("q", json!({"VisibilityTimeout": 5, "Fifo": false})),
            fail("UnsupportedOperation"),
        ),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": default_attributes(), "Counts": counts(0, 0, 0)}),
        ),
        (set("q", json!({})), ok()),
        (
            set(
                "q",
                json!({"VisibilityTimeout": 5, "DeadLetterQueue": "other", "MaxReceiveCount": 2}),
            ),
            ok(),
        ),
        (
            set(
                "q",
                json!({"DelaySeconds": 7, "DeadLetterQueue": null, "ContentBasedDeduplication": false}),
            ),
            ok(),
        ),
        (
            set("f.fifo", json!({"ContentBasedDeduplication": true})),
            ok(),
        ),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 5, "DelaySeconds": 7, "RetentionSeconds": 345_600,
                "Fifo": false, "ContentBasedDeduplication": false,
                "DeadLetterQueue": "other", "MaxReceiveCount": 2,
            }, "Counts": counts(0, 0, 0)}),
        ),
        (
            get_attributes("f.fifo"),
            json!({"ok": true, "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345_600,
                "Fifo": true, "ContentBasedDeduplication": true,
            }, "Counts": counts(0, 0, 0)}),
        ),
    ]);
}

#[test]
fn lowering_retention_removes_messages_for_good() {
    check(&[
        (create("q"), ok()),
        (send("q", "old"), sent(1)),
        (tick(100), json!({"ok": true, "Now": 100})),
        (send("q", "new"), sent(2)),
        (set("q", json!({"RetentionSeconds": 60})), ok()),
        (set("q", json!({"RetentionSeconds": 1000})), ok()),
        (get_attributes("q"), {
            let mut attributes = default_attributes();
            attributes["RetentionSeconds"] = json!(1000);
            json!({"ok": true, "Attributes": attributes, "Counts": counts(1, 0, 0)})
        }),
    ]);
}

#[test]
fn retention_removes_a_message_exactly_at_its_deadline() {
    check(&[
        (create_with("q", json!({"RetentionSeconds": 60})), ok()),
        (send("q", "m"), sent(1)),
        (tick(59), json!({"ok": true, "Now": 59})),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": {
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 60,
            "Fifo": false, "ContentBasedDeduplication": false,
        }, "Counts": counts(1, 0, 0)}),
        ),
        (tick(1), json!({"ok": true, "Now": 60})),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": {
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 60,
            "Fifo": false, "ContentBasedDeduplication": false,
        }, "Counts": counts(0, 0, 0)}),
        ),
    ]);
}

#[test]
fn list_queues_in_byte_order_with_prefix() {
    let names = ["b", "a_", "a", "A", "a-x", "9", "a.fifo"];
    let mut session: Vec<_> = names
        .iter()
        .map(|&name| {
            let request = if name.ends_with(".fifo") {
                create_with(name, json!({"Fifo": true}))
            } else {
                create(name)
            };
            (request, ok())
        })
        .collect();
    session.extend([
        (
            json!({"Command": "ListQueues"}),
            json!({"ok": true, "QueueNames": ["9", "A", "a", "a-x", "a.fifo", "a_", "b"]}),
        ),
        (
            json!({"Command": "ListQueues", "Prefix": "a"}),
            json!({"ok": true, "QueueNames": ["a", "a-x", "a.fifo", "a_"]}),
        ),
        (
            json!({"Command": "ListQueues", "Prefix": "a-"}),
            json!({"ok": true, "QueueNames": ["a-x"]}),
        ),
        (
            json!({"Command": "ListQueues", "Prefix": "c"}),
            json!({"ok": true, "QueueNames": []}),
        ),
        (
            json!({"Command": "ListQueues", "Prefix": 1}),
            fail("InvalidParameterValue"),
        ),
    ]);
    check(&session);
}

#[test]
fn delete_queue_clears_dead_letter_references_and_forgets_everything() {
    check(&[
        (create("dlq"), ok()),
        (
            create_with("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1})),
            ok(),
        ),
        (send("dlq", "m"), sent(1)),
        (json!({"Command": "DeleteQueue", "QueueName": "dlq"}), ok()),
        (
            json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
            fail("QueueDoesNotExist"),
        ),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": default_attributes(), "Counts": counts(0, 0, 0)}),
        ),
        (create_with("dlq", json!({"VisibilityTimeout": 1})), ok()),
        (get_attributes("dlq"), {
            let mut attributes = default_attributes();
            attributes["VisibilityTimeout"] = json!(1);
            json!({"ok": true, "Attributes": attributes, "Counts": counts(0, 0, 0)})
        }),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": default_attributes(), "Counts": counts(0, 0, 0)}),
        ),
    ]);
}

#[test]
fn purge_removes_messages_in_every_state() {
    check(&[
        (create("q"), ok()),
        (send("q", "a"), sent(1)),
        (
            json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 10}),
            sent(2),
        ),
        (send("q", "c"), sent(3)),
        (
            receive("q", 1),
            messages(json!([{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0,
            }])),
        ),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": default_attributes(), "Counts": counts(1, 1, 1)}),
        ),
        (json!({"Command": "PurgeQueue", "QueueName": "q"}), ok()),
        (
            get_attributes("q"),
            json!({"ok": true, "Attributes": default_attributes(), "Counts": counts(0, 0, 0)}),
        ),
        (delete_message("q", "r-1"), fail("ReceiptHandleExpired")),
    ]);
}
