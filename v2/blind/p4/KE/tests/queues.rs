//! Protocol, error precedence and queue commands (SPEC sections 1, 2, 4 and 5).

mod common;

use common::Session;
use serde_json::json;

#[test]
fn malformed_lines_are_invalid_requests_and_the_session_continues() {
    let mut s = Session::new();
    for line in [
        &b""[..],
        b"  ",
        b"nope",
        b"[1]",
        b"{\"Command\": 3}",
        b"{\"Command\": \"Nope\"}",
        b"\xff",
    ] {
        let response = s.raw(line);
        assert_eq!(response["error"]["code"], "InvalidRequest", "{line:?}");
    }
    s.tick(1);
}

#[test]
fn success_responses_carry_exactly_the_result_members() {
    let mut s = Session::new();
    assert_eq!(
        s.call(json!({"Command": "CreateQueue", "QueueName": "q"})),
        json!({"ok": true})
    );
    assert_eq!(
        s.call(json!({"Command": "Tick", "Seconds": 7})),
        json!({"ok": true, "Now": 7})
    );
}

#[test]
fn error_precedence_follows_section_2() {
    let mut s = Session::new();
    // Missing beats invalid, invalid beats lookup, lookup beats the command's own rules.
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": 1})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "nope", "Body": "b", "GroupId": "g"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "bogus"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {"Fifo": true}})),
        "QueueDoesNotExist"
    );
}

#[test]
fn create_queue_defaults_and_get_attributes() {
    let mut s = Session::new();
    s.create("q", json!({}));
    assert_eq!(
        s.attributes("q"),
        json!({
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false,
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
}

#[test]
fn create_queue_rules_in_order() {
    let mut s = Session::new();
    let create = |name: &str, attributes| json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes});

    assert_eq!(s.err(create("q.fifo", json!({}))), "QueueNameInvalid");
    assert_eq!(
        s.err(create("q", json!({"Fifo": true}))),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(create("q", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create("q", json!({"MaxReceiveCount": 3}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create(
            "q",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3})
        )),
        "QueueDoesNotExist"
    );
    s.create("dlq.fifo", json!({"Fifo": true}));
    assert_eq!(
        s.err(create(
            "q",
            json!({"DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 3})
        )),
        "InvalidParameterValue"
    );
    s.create(
        "q.fifo",
        json!({"Fifo": true, "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 3}),
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 10}));
    s.create("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 0}));
    let again = json!({"Command": "CreateQueue", "QueueName": "q"});
    assert_eq!(s.err(again), "QueueAlreadyExists");
}

#[test]
fn queue_limit_is_1000() {
    let mut s = Session::new();
    for i in 0..1000 {
        s.create(&format!("q{i}"), json!({}));
    }
    s.create("q0", json!({})); // Existing queue with equal attributes still succeeds.
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let mut s = Session::new();
    for name in ["b", "a_x", "aZ", "a-1", "a9", "a.fifo", "ab"] {
        let fifo = name.ends_with(".fifo");
        s.create(name, json!({"Fifo": fifo}));
    }
    let all = s.ok(json!({"Command": "ListQueues"}));
    assert_eq!(
        all["QueueNames"],
        json!(["a-1", "a.fifo", "a9", "aZ", "a_x", "ab", "b"])
    );
    let some = s.ok(json!({"Command": "ListQueues", "Prefix": "a"}));
    assert_eq!(some["QueueNames"].as_array().unwrap().len(), 6);
    let none = s.ok(json!({"Command": "ListQueues", "Prefix": "c"}));
    assert_eq!(none["QueueNames"], json!([]));
}

#[test]
fn set_queue_attributes_rules_in_order() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("other", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |attributes| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": attributes});

    assert_eq!(
        s.err(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(s.err(set(json!({"Fifo": false}))), "UnsupportedOperation");
    assert_eq!(
        s.err(set(json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(json!({"DeadLetterQueue": "other"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            json!({"DeadLetterQueue": "gone", "MaxReceiveCount": 1})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(set(json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1})
        )),
        "InvalidParameterValue"
    );

    s.ok(set(json!({})));
    s.ok(set(
        json!({"VisibilityTimeout": 5, "DeadLetterQueue": "other", "MaxReceiveCount": 2}),
    ));
    let attributes = &s.attributes("q")["Attributes"];
    assert_eq!(attributes["VisibilityTimeout"], 5);
    assert_eq!(attributes["DelaySeconds"], 0);
    assert_eq!(attributes["DeadLetterQueue"], "other");
    assert_eq!(attributes["MaxReceiveCount"], 2);
}

#[test]
fn deleting_a_queue_clears_dead_letter_configs_that_name_it() {
    let mut s = Session::new();
    s.create("dlq", json!({}));
    s.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}));
    s.send("dlq", "old");
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    assert!(
        s.attributes("q")["Attributes"]
            .get("DeadLetterQueue")
            .is_none()
    );
    assert!(
        s.attributes("q")["Attributes"]
            .get("MaxReceiveCount")
            .is_none()
    );

    // A queue recreated under the same name starts empty.
    s.create("dlq", json!({}));
    assert_eq!(s.counts("dlq"), (0, 0, 0));
    assert_eq!(
        s.err(json!({"Command": "DeleteQueue", "QueueName": "missing"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn purge_removes_messages_but_keeps_sequence_and_deduplication() {
    let mut s = Session::new();
    s.create("q.fifo", json!({"Fifo": true}));
    assert_eq!(s.send_fifo("q.fifo", "a", "g")["SequenceNumber"], 1);
    s.receive("q.fifo", 1);
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}));
    assert_eq!(s.counts("q.fifo"), (0, 0, 0));
    // Still a duplicate of the purged message.
    assert_eq!(
        s.send_fifo("q.fifo", "a", "g"),
        json!({"MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(s.send_fifo("q.fifo", "b", "g")["SequenceNumber"], 2);
}

#[test]
fn tick_advances_the_clock() {
    let mut s = Session::new();
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 0})),
        json!({"Now": 0})
    );
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 1209600})),
        json!({"Now": 1209600})
    );
    assert_eq!(
        s.err(json!({"Command": "Tick", "Seconds": 1209601})),
        "InvalidParameterValue"
    );
    assert_eq!(s.err(json!({"Command": "Tick"})), "MissingParameter");
}
