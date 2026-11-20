use crate::support::Session;
use serde_json::{Value, json};

fn create(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

fn list(session: &mut Session, prefix: Option<&str>) -> Value {
    session.run(json!({"Command": "ListQueues", "Prefix": prefix}))
}

fn ok() -> Value {
    json!({"ok": true})
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut session = Session::default();
    for name in ["b", "a_", "aZ", "a9", "a-", "a.fifo", "ab"] {
        let fifo = name.ends_with(".fifo");
        assert_eq!(session.run(create(name, json!({"Fifo": fifo}))), ok());
    }
    assert_eq!(
        list(&mut session, None),
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a9", "aZ", "a_", "ab", "b"]})
    );
    assert_eq!(
        list(&mut session, Some("a")),
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a9", "aZ", "a_", "ab"]})
    );
    assert_eq!(
        list(&mut session, Some("ab")),
        json!({"ok": true, "QueueNames": ["ab"]})
    );
    assert_eq!(
        list(&mut session, Some("c")),
        json!({"ok": true, "QueueNames": []})
    );
    session.fails(
        json!({"Command": "ListQueues", "Prefix": 1}),
        "InvalidParameterValue",
    );
}

#[test]
fn create_queue_errors_follow_the_check_order() {
    let mut session = Session::default();
    assert_eq!(session.run(create("dlq", json!({}))), ok());
    assert_eq!(session.run(create("dlq.fifo", json!({"Fifo": true}))), ok());
    let cases: &[(Value, &str)] = &[
        (json!({"Command": "CreateQueue"}), "MissingParameter"),
        (
            json!({"Command": "CreateQueue", "QueueName": null}),
            "MissingParameter",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": 5}),
            "InvalidParameterValue",
        ),
        // The name is checked before every other parameter.
        (create("bad name", json!(5)), "QueueNameInvalid"),
        (create("", json!({})), "QueueNameInvalid"),
        (create("q", json!(5)), "InvalidParameterValue"),
        (create("q", json!({"Unknown": 1})), "InvalidParameterValue"),
        (
            create("q", json!({"VisibilityTimeout": 43201})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"DelaySeconds": 901})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"RetentionSeconds": 59})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"Fifo": "true"})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"MaxReceiveCount": 0})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"DeadLetterQueue": 1})),
            "InvalidParameterValue",
        ),
        (create("q", json!({"Fifo": true})), "QueueNameInvalid"),
        (create("q.fifo", json!({})), "QueueNameInvalid"),
        (
            create("q", json!({"ContentBasedDeduplication": true})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"DeadLetterQueue": "dlq"})),
            "InvalidParameterValue",
        ),
        (
            create("q", json!({"MaxReceiveCount": 3})),
            "InvalidParameterValue",
        ),
        (
            create(
                "q",
                json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 3}),
            ),
            "QueueDoesNotExist",
        ),
        (
            create(
                "q",
                json!({"DeadLetterQueue": "no pe", "MaxReceiveCount": 3}),
            ),
            "QueueDoesNotExist",
        ),
        (
            create(
                "q",
                json!({"DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 3}),
            ),
            "InvalidParameterValue",
        ),
        (
            create(
                "q.fifo",
                json!({"Fifo": true, "DeadLetterQueue": "dlq", "MaxReceiveCount": 3}),
            ),
            "InvalidParameterValue",
        ),
    ];
    for (command, code) in cases {
        session.fails(command.clone(), code);
    }
    assert_eq!(
        list(&mut session, None),
        json!({"ok": true, "QueueNames": ["dlq", "dlq.fifo"]})
    );
}

#[test]
fn null_attributes_are_absent() {
    let mut session = Session::default();
    let attributes = json!({"Unknown": null, "Fifo": null, "DeadLetterQueue": null});
    assert_eq!(session.run(create("q", attributes)), ok());
    assert_eq!(
        session.run(json!({"Command": "CreateQueue", "QueueName": "r", "Attributes": null})),
        ok()
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let mut session = Session::default();
    assert_eq!(session.run(create("dlq", json!({}))), ok());
    let attributes =
        json!({"VisibilityTimeout": 10, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2});
    assert_eq!(session.run(create("q", attributes.clone())), ok());
    assert_eq!(session.run(create("q", attributes)), ok());
    let explicit_defaults = json!({
        "VisibilityTimeout": 10, "DelaySeconds": 0, "RetentionSeconds": 345600, "Fifo": false,
        "ContentBasedDeduplication": false, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2,
    });
    assert_eq!(session.run(create("q", explicit_defaults)), ok());
    for attributes in [
        json!({}),
        json!({"VisibilityTimeout": 10}),
        json!({"VisibilityTimeout": 10, "DeadLetterQueue": "dlq", "MaxReceiveCount": 3}),
        // A missing dead-letter queue still loses to the existing-queue rule.
        json!({"VisibilityTimeout": 10, "DeadLetterQueue": "gone", "MaxReceiveCount": 2}),
    ] {
        session.fails(create("q", attributes), "QueueAlreadyExists");
    }
    assert_eq!(session.run(create("dlq", json!({}))), ok());
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut session = Session::default();
    for n in 0..1000 {
        assert_eq!(session.run(create(&format!("q{n}"), json!({}))), ok());
    }
    session.fails(create("one-more", json!({})), "LimitExceeded");
    // Re-creating an existing queue is not a new queue.
    assert_eq!(session.run(create("q7", json!({}))), ok());
    assert_eq!(
        session.run(json!({"Command": "DeleteQueue", "QueueName": "q7"})),
        ok()
    );
    assert_eq!(session.run(create("one-more", json!({}))), ok());
}

#[test]
fn set_queue_attributes_errors_follow_the_check_order() {
    let mut session = Session::default();
    for (name, fifo) in [("q", false), ("other", false), ("f.fifo", true)] {
        assert_eq!(session.run(create(name, json!({"Fifo": fifo}))), ok());
    }
    let set = |attributes: Value| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": attributes});
    let cases: &[(Value, &str)] = &[
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SetQueueAttributes", "Attributes": {}}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": 1}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}),
            "QueueDoesNotExist",
        ),
        (set(json!({"Fifo": 1})), "InvalidParameterValue"),
        (set(json!({"Fifo": false})), "UnsupportedOperation"),
        (
            set(json!({"ContentBasedDeduplication": true})),
            "InvalidParameterValue",
        ),
        (set(json!({"MaxReceiveCount": 1})), "InvalidParameterValue"),
        (
            set(json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 1})),
            "QueueDoesNotExist",
        ),
        (
            set(json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})),
            "InvalidParameterValue",
        ),
        (
            set(json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1})),
            "InvalidParameterValue",
        ),
    ];
    for (command, code) in cases {
        session.fails(command.clone(), code);
    }
    assert_eq!(session.run(set(json!({}))), ok());
    assert_eq!(
        session.run(set(json!({"ContentBasedDeduplication": false}))),
        ok()
    );
    assert_eq!(
        session.run(set(
            json!({"DeadLetterQueue": "other", "MaxReceiveCount": 1})
        )),
        ok()
    );
}

#[test]
fn delete_queue_removes_it_and_clears_references() {
    let mut session = Session::default();
    assert_eq!(session.run(create("dlq", json!({}))), ok());
    let attributes = json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2});
    assert_eq!(session.run(create("q", attributes.clone())), ok());
    assert_eq!(
        session.run(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        ok()
    );
    session.fails(
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        "QueueDoesNotExist",
    );
    assert_eq!(
        list(&mut session, None),
        json!({"ok": true, "QueueNames": ["q"]})
    );
    // q lost its dead-letter configuration, so it now equals a plain queue.
    assert_eq!(session.run(create("dlq", json!({}))), ok());
    session.fails(create("q", attributes), "QueueAlreadyExists");
    assert_eq!(session.run(create("q", json!({}))), ok());
}
