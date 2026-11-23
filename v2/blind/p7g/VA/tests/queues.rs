mod common;

use common::Session;
use serde_json::json;

#[test]
fn create_queue_name_and_type_must_agree() {
    let mut session = Session::new();
    assert_eq!(
        session.err(json!({"Command": "CreateQueue", "QueueName": "a.fifo"})),
        "QueueNameInvalid"
    );
    assert_eq!(
        session
            .err(json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": true}})),
        "QueueNameInvalid"
    );
    session.create("a.fifo", json!({"Fifo": true}));
    session.create("b", json!({"Fifo": false}));
}

#[test]
fn create_queue_attribute_rules() {
    let mut session = Session::new();
    session.create("dlq", json!({}));
    let cases = [
        (
            json!({"ContentBasedDeduplication": true}),
            "InvalidParameterValue",
        ),
        (json!({"MaxReceiveCount": 3}), "InvalidParameterValue"),
        (json!({"DeadLetterQueue": "dlq"}), "InvalidParameterValue"),
        (
            json!({"DeadLetterQueue": "gone", "MaxReceiveCount": 3}),
            "QueueDoesNotExist",
        ),
        (
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1001}),
            "InvalidParameterValue",
        ),
        (json!({"Bogus": 1}), "InvalidParameterValue"),
        (json!({"VisibilityTimeout": "5"}), "InvalidParameterValue"),
        (json!({"RetentionSeconds": 59}), "InvalidParameterValue"),
        (json!({"DelaySeconds": 901}), "InvalidParameterValue"),
    ];
    for (attributes, code) in cases {
        let command = json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes});
        assert_eq!(session.err(command), code, "{attributes}");
    }
    session.create("f.fifo", json!({"Fifo": true}));
    assert_eq!(
        session.err(json!({"Command": "CreateQueue", "QueueName": "g.fifo",
            "Attributes": {"Fifo": true, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}})),
        "InvalidParameterValue",
        "dead-letter queue of another type"
    );
}

#[test]
fn create_queue_again_is_idempotent_only_for_equal_attributes() {
    let mut session = Session::new();
    session.create("dlq", json!({}));
    let attributes =
        json!({"VisibilityTimeout": 7, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2});
    session.create("q", attributes.clone());
    session.create("q", attributes);
    assert_eq!(
        session.err(json!({"Command": "CreateQueue", "QueueName": "q"})),
        "QueueAlreadyExists"
    );
    session.create("plain", json!({}));
    session.create("plain", json!({"VisibilityTimeout": 30, "Fifo": false}));
    assert_eq!(
        session.err(json!({"Command": "CreateQueue", "QueueName": "plain",
            "Attributes": {"DelaySeconds": 1}})),
        "QueueAlreadyExists"
    );
}

#[test]
fn create_queue_stops_at_one_thousand() {
    let mut session = Session::new();
    for n in 0..1000 {
        session.create(&format!("q{n}"), json!({}));
    }
    assert_eq!(
        session.err(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
    session.create("q5", json!({}));
}

#[test]
fn get_queue_attributes_lists_defaults_and_set_dead_letter_config() {
    let mut session = Session::new();
    session.create("q", json!({}));
    assert_eq!(
        session.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"})),
        json!({
            "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false},
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );
    session.create("d", json!({}));
    session.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 4, "DelaySeconds": 9}}));
    let response = session.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        response["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 9, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "d", "MaxReceiveCount": 4})
    );
}

#[test]
fn set_queue_attributes_rules() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.create("f.fifo", json!({"Fifo": true}));
    let set = |queue: &str, attributes: serde_json::Value| json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes});
    let cases = [
        ("q", json!({"Fifo": false}), "UnsupportedOperation"),
        ("q", json!({"Fifo": 5}), "InvalidParameterValue"),
        (
            "q",
            json!({"ContentBasedDeduplication": true}),
            "InvalidParameterValue",
        ),
        ("q", json!({"MaxReceiveCount": 2}), "InvalidParameterValue"),
        (
            "q",
            json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 2}),
            "QueueDoesNotExist",
        ),
        (
            "q",
            json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2}),
            "InvalidParameterValue",
        ),
        (
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}),
            "InvalidParameterValue",
        ),
        ("gone", json!({}), "QueueDoesNotExist"),
    ];
    for (queue, attributes, code) in cases {
        assert_eq!(
            session.err(set(queue, attributes.clone())),
            code,
            "{queue} {attributes}"
        );
    }
    session.ok(set("q", json!({"ContentBasedDeduplication": false})));
    session.ok(set("f.fifo", json!({"ContentBasedDeduplication": true})));
    assert_eq!(
        session.err(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut session = Session::new();
    for name in ["b", "B", "_x", "a.fifo", "a-b", "a1", "ab"] {
        let attributes = json!({"Fifo": name.ends_with(".fifo")});
        session.create(name, attributes);
    }
    assert_eq!(
        session.ok(json!({"Command": "ListQueues"})),
        json!({"QueueNames": ["B", "_x", "a-b", "a.fifo", "a1", "ab", "b"]})
    );
    assert_eq!(
        session.ok(json!({"Command": "ListQueues", "Prefix": "a"})),
        json!({"QueueNames": ["a-b", "a.fifo", "a1", "ab"]})
    );
    assert_eq!(
        session.ok(json!({"Command": "ListQueues", "Prefix": "zz"})),
        json!({"QueueNames": []})
    );
    assert_eq!(
        session.err(json!({"Command": "ListQueues", "Prefix": 1})),
        "InvalidParameterValue"
    );
}

#[test]
fn delete_queue_clears_dead_letter_config_of_queues_pointing_at_it() {
    let mut session = Session::new();
    session.create("d", json!({}));
    session.create("q", json!({"DeadLetterQueue": "d", "MaxReceiveCount": 1}));
    session.ok(json!({"Command": "DeleteQueue", "QueueName": "d"}));
    let response = session.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(response["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(response["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(
        session.err(json!({"Command": "DeleteQueue", "QueueName": "d"})),
        "QueueDoesNotExist"
    );
    // The name is free again, and the new queue starts empty.
    session.create("d", json!({}));
    assert_eq!(
        session.counts("d"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn purge_removes_every_state_but_keeps_sequence_numbers_and_dedup() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true}));
    let send = |session: &mut Session, id: &str, delay: Option<u32>| {
        let mut command = json!({"Command": "SendMessage", "QueueName": "f.fifo",
            "Body": "b", "GroupId": "g", "DeduplicationId": id});
        if let Some(delay) = delay {
            command["DelaySeconds"] = json!(delay);
        }
        session.call(command)
    };
    assert_eq!(send(&mut session, "one", None)["SequenceNumber"], json!(1));
    session.receive("f.fifo");
    session.ok(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(
        session.counts("f.fifo"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let again = send(&mut session, "one", None);
    assert_eq!(
        again,
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    assert_eq!(send(&mut session, "two", None)["SequenceNumber"], json!(2));
}

#[test]
fn tick_reports_the_clock() {
    let mut session = Session::new();
    assert_eq!(
        session.ok(json!({"Command": "Tick", "Seconds": 0})),
        json!({"Now": 0})
    );
    assert_eq!(
        session.ok(json!({"Command": "Tick", "Seconds": 1209600})),
        json!({"Now": 1209600})
    );
    assert_eq!(
        session.ok(json!({"Command": "Tick", "Seconds": 1})),
        json!({"Now": 1209601})
    );
}
