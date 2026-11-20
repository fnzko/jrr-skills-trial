mod common;

use common::{Session, error_code, success};
use serde_json::json;

#[test]
fn create_queue_applies_defaults() {
    let mut s = Session::new();
    s.create("jobs", json!({}));
    assert_eq!(
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"})),
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false,
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.create("a", json!({"VisibilityTimeout": 5}));
    assert_eq!(
        s.run(json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": {"VisibilityTimeout": 5, "DelaySeconds": null}})),
        success()
    );
    for different in [json!({}), json!({"VisibilityTimeout": 6})] {
        let response =
            s.run(json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": different}));
        assert_eq!(error_code(&response), "QueueAlreadyExists", "{different}");
    }
}

#[test]
fn create_queue_error_order() {
    let mut s = Session::new();
    let create = |s: &mut Session, name: &str, attributes| {
        let response =
            s.run(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
        error_code(&response).to_owned()
    };
    // Rule 1: the suffix and Fifo must agree.
    assert_eq!(create(&mut s, "a.fifo", json!({})), "QueueNameInvalid");
    assert_eq!(
        create(&mut s, "a", json!({"Fifo": true})),
        "QueueNameInvalid"
    );
    // Rule 2 before rule 3 before the dead-letter lookups.
    assert_eq!(
        create(
            &mut s,
            "a",
            json!({"ContentBasedDeduplication": true, "DeadLetterQueue": "x"})
        ),
        "InvalidParameterValue"
    );
    assert_eq!(
        create(&mut s, "a", json!({"MaxReceiveCount": 3})),
        "InvalidParameterValue"
    );
    assert_eq!(
        create(
            &mut s,
            "a",
            json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 3})
        ),
        "QueueDoesNotExist"
    );
    // Rule 6: dead-letter queue of the other type.
    s.create("std", json!({}));
    assert_eq!(
        create(
            &mut s,
            "b.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "std", "MaxReceiveCount": 3})
        ),
        "InvalidParameterValue"
    );
}

#[test]
fn existing_queue_is_compared_before_dead_letter_checks() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}));
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}}));
    assert_eq!(response, success());
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "ghost", "MaxReceiveCount": 2}}));
    assert_eq!(error_code(&response), "QueueAlreadyExists");
}

#[test]
fn at_most_1000_queues() {
    let mut s = Session::new();
    for n in 0..1000 {
        s.create(&format!("q{n}"), json!({}));
    }
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "extra"}));
    assert_eq!(error_code(&response), "LimitExceeded");
    // Re-creating an existing queue needs no room.
    assert_eq!(
        s.run(json!({"Command": "CreateQueue", "QueueName": "q7"})),
        success()
    );
}

#[test]
fn unknown_queue_beats_command_rules_but_not_shape_checks() {
    let mut s = Session::new();
    let response = s.run(json!({"Command": "SetQueueAttributes", "QueueName": "ghost", "Attributes": {"Fifo": true}}));
    assert_eq!(error_code(&response), "QueueDoesNotExist");
    let response = s.run(json!({"Command": "SetQueueAttributes", "QueueName": "ghost", "Attributes": {"Fifo": "yes"}}));
    assert_eq!(error_code(&response), "InvalidParameterValue");
    let response = s.run(json!({"Command": "PurgeQueue", "QueueName": "not a valid name!"}));
    assert_eq!(error_code(&response), "QueueDoesNotExist");
}

#[test]
fn set_queue_attributes_rules_in_order() {
    let mut s = Session::new();
    s.create("std", json!({}));
    s.create("other.fifo", json!({"Fifo": true}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |s: &mut Session, queue: &str, attributes| {
        let response = s.run(
            json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes}),
        );
        error_code(&response).to_owned()
    };
    assert_eq!(
        set(&mut s, "std", json!({"Fifo": false})),
        "UnsupportedOperation"
    );
    assert_eq!(
        set(&mut s, "std", json!({"ContentBasedDeduplication": true})),
        "InvalidParameterValue"
    );
    assert_eq!(
        set(&mut s, "std", json!({"MaxReceiveCount": 2})),
        "InvalidParameterValue"
    );
    assert_eq!(
        set(
            &mut s,
            "std",
            json!({"DeadLetterQueue": "ghost", "MaxReceiveCount": 2})
        ),
        "QueueDoesNotExist"
    );
    assert_eq!(
        set(
            &mut s,
            "std",
            json!({"DeadLetterQueue": "std", "MaxReceiveCount": 2})
        ),
        "InvalidParameterValue"
    );
    assert_eq!(
        set(
            &mut s,
            "std",
            json!({"DeadLetterQueue": "other.fifo", "MaxReceiveCount": 2})
        ),
        "InvalidParameterValue"
    );
    // A failed command changes nothing.
    let attributes =
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "std"}))["Attributes"].clone();
    assert_eq!(attributes["VisibilityTimeout"], json!(30));
    assert_eq!(attributes.get("DeadLetterQueue"), None);
}

#[test]
fn set_queue_attributes_changes_only_what_is_given_and_never_removes_dead_letter() {
    let mut s = Session::new();
    s.create("d1", json!({}));
    s.create("d2", json!({}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "d1", "MaxReceiveCount": 4, "DelaySeconds": 7}),
    );
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 9, "DelaySeconds": null}}));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "d2", "MaxReceiveCount": 5}}));
    assert_eq!(
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"],
        json!({
            "VisibilityTimeout": 9,
            "DelaySeconds": 7,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "d2",
            "MaxReceiveCount": 5,
        })
    );
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in ["b", "a_1", "a-1", "A1", "a1", "a1.fifo", "a.fifo"] {
        let attributes = if name.ends_with(".fifo") {
            json!({"Fifo": true})
        } else {
            json!({})
        };
        s.create(name, attributes);
    }
    assert_eq!(
        s.ok(json!({"Command": "ListQueues"})),
        json!({"ok": true, "QueueNames": ["A1", "a-1", "a.fifo", "a1", "a1.fifo", "a_1", "b"]})
    );
    assert_eq!(
        s.ok(json!({"Command": "ListQueues", "Prefix": "a1"}))["QueueNames"],
        json!(["a1", "a1.fifo"])
    );
    assert_eq!(
        s.ok(json!({"Command": "ListQueues", "Prefix": ""}))["QueueNames"]
            .as_array()
            .map(Vec::len),
        Some(7)
    );
    assert_eq!(
        s.ok(json!({"Command": "ListQueues", "Prefix": "zz"}))["QueueNames"],
        json!([])
    );
}

#[test]
fn delete_queue_clears_dead_letter_references_and_forgets_everything() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    s.send("dlq", "old");
    assert_eq!(
        s.run(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        success()
    );
    let attributes =
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert_eq!(
        attributes,
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
        })
    );
    let response = s.run(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    assert_eq!(error_code(&response), "QueueDoesNotExist");
    s.create("dlq", json!({}));
    assert_eq!(
        s.counts("dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn tick_advances_the_clock() {
    let mut s = Session::new();
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 0})),
        json!({"ok": true, "Now": 0})
    );
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 1209600})),
        json!({"ok": true, "Now": 1209600})
    );
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 4})),
        json!({"ok": true, "Now": 1209604})
    );
    assert_eq!(
        error_code(&s.run(json!({"Command": "Tick", "Seconds": 1209601}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        error_code(&s.run(json!({"Command": "Tick"}))),
        "MissingParameter"
    );
}

#[test]
fn retention_removes_a_message_exactly_when_it_is_old_enough() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 100}));
    s.send("q", "a");
    s.tick(60);
    s.send("q", "b");
    s.tick(39);
    assert_eq!(s.counts("q")["Visible"], json!(2));
    s.tick(1);
    assert_eq!(s.counts("q")["Visible"], json!(1));
}

#[test]
fn lowering_retention_removes_at_once_and_raising_it_restores_nothing() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.tick(60);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 61}}));
    assert_eq!(s.counts("q")["Visible"], json!(1));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(s.counts("q")["Visible"], json!(0));
}

#[test]
fn retention_applies_to_messages_in_every_state() {
    let mut s = Session::new();
    s.create(
        "q",
        json!({"RetentionSeconds": 60, "VisibilityTimeout": 1000, "DelaySeconds": 100}),
    );
    s.send("q", "delayed");
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "now", "DelaySeconds": null}));
    s.tick(60);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_removes_messages_whatever_their_state() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.send("q", "a");
    s.send("q", "b");
    let handle = s.receive("q")["Messages"][0]["ReceiptHandle"].clone();
    assert_eq!(
        s.run(json!({"Command": "PurgeQueue", "QueueName": "q"})),
        success()
    );
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let response =
        s.run(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle}));
    assert_eq!(error_code(&response), "ReceiptHandleExpired");
}
