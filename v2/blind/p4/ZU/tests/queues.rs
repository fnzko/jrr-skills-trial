mod common;

use common::Client;
use serde_json::json;

#[test]
fn new_queue_reports_default_attributes_and_no_messages() {
    let mut client = Client::default();
    client.create("jobs", json!({}));
    let response = client.call(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(
        response,
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
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let mut client = Client::default();
    client.create("jobs", json!({"VisibilityTimeout": 10}));
    // Equal after defaults, with and without spelling out a default.
    client.create("jobs", json!({"VisibilityTimeout": 10, "DelaySeconds": 0}));
    let code = client.error(json!({"Command": "CreateQueue", "QueueName": "jobs"}));
    assert_eq!(code, "QueueAlreadyExists");
}

#[test]
fn create_queue_rule_order() {
    let mut client = Client::default();
    client.create("std", json!({}));
    client.create("dlq.fifo", json!({"Fifo": true}));
    let cases = [
        (json!({"QueueName": "a.fifo"}), "QueueNameInvalid"),
        (
            json!({"QueueName": "a", "Attributes": {"Fifo": true}}),
            "QueueNameInvalid",
        ),
        (
            json!({"QueueName": "a", "Attributes": {"ContentBasedDeduplication": true}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "a", "Attributes": {"MaxReceiveCount": 2}}),
            "InvalidParameterValue",
        ),
        // An existing queue is compared before the dead-letter queue is looked up.
        (
            json!({"QueueName": "std", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}),
            "QueueAlreadyExists",
        ),
        (
            json!({"QueueName": "a", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}),
            "QueueDoesNotExist",
        ),
        (
            json!({"QueueName": "a", "Attributes": {"DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 2}}),
            "InvalidParameterValue",
        ),
    ];
    for (mut request, want) in cases {
        request["Command"] = json!("CreateQueue");
        assert_eq!(client.error(request.clone()), want, "request {request}");
    }
}

#[test]
fn dead_letter_configuration_is_reported_and_compared() {
    let mut client = Client::default();
    client.create("dlq", json!({}));
    client.create(
        "jobs",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3}),
    );
    client.create(
        "jobs",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3}),
    );
    let code = client.error(json!({"Command": "CreateQueue", "QueueName": "jobs",
        "Attributes": {"DeadLetterQueue": "dlq", "MaxReceiveCount": 4}}));
    assert_eq!(code, "QueueAlreadyExists");
    let response = client.call(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(
        response["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dlq",
            "MaxReceiveCount": 3
        })
    );
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut client = Client::default();
    for name in ["b", "a_", "aZ", "a9", "a-", "ab"] {
        client.create(name, json!({}));
    }
    client.create("a.fifo", json!({"Fifo": true}));
    let response = client.call(json!({"Command": "ListQueues"}));
    assert_eq!(
        response,
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a9", "aZ", "a_", "ab", "b"]})
    );
    let response = client.call(json!({"Command": "ListQueues", "Prefix": "a"}));
    assert_eq!(
        response,
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a9", "aZ", "a_", "ab"]})
    );
    let response = client.call(json!({"Command": "ListQueues", "Prefix": "zz"}));
    assert_eq!(response, json!({"ok": true, "QueueNames": []}));
}

#[test]
fn lookup_fails_for_any_unknown_name() {
    let mut client = Client::default();
    for name in ["missing", "not a valid name", ""] {
        for command in ["GetQueueAttributes", "DeleteQueue", "PurgeQueue"] {
            let code = client.error(json!({"Command": command, "QueueName": name}));
            assert_eq!(code, "QueueDoesNotExist", "{command} {name:?}");
        }
    }
}

#[test]
fn delete_queue_clears_dead_letter_configurations_pointing_at_it() {
    let mut client = Client::default();
    client.create("dlq", json!({}));
    client.create(
        "jobs",
        json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    assert_eq!(
        client.call(json!({"Command": "DeleteQueue", "QueueName": "dlq"})),
        json!({"ok": true})
    );
    let response = client.call(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(response["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(response["Attributes"].get("MaxReceiveCount"), None);
    let response = client.call(json!({"Command": "ListQueues"}));
    assert_eq!(response, json!({"ok": true, "QueueNames": ["jobs"]}));
}

#[test]
fn recreated_queue_shares_nothing_with_the_deleted_one() {
    let mut client = Client::default();
    client.create("jobs", json!({}));
    client.send("jobs", "old");
    let handle = client.receive("jobs", 1)[0]["ReceiptHandle"].clone();
    client.call(json!({"Command": "DeleteQueue", "QueueName": "jobs"}));
    client.create("jobs", json!({"VisibilityTimeout": 5}));
    assert_eq!(client.counts("jobs"), [0, 0, 0]);
    let code = client
        .error(json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": handle}));
    assert_eq!(code, "ReceiptHandleExpired");
}

#[test]
fn set_queue_attributes_rule_order() {
    let mut client = Client::default();
    client.create("std", json!({}));
    client.create("other", json!({}));
    client.create("f.fifo", json!({"Fifo": true}));
    let cases = [
        (
            json!({"QueueName": "std", "Attributes": {"Fifo": false}}),
            "UnsupportedOperation",
        ),
        (
            json!({"QueueName": "std", "Attributes": {"ContentBasedDeduplication": true}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "std", "Attributes": {"DeadLetterQueue": "other"}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "std", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 1}}),
            "QueueDoesNotExist",
        ),
        (
            json!({"QueueName": "std", "Attributes": {"DeadLetterQueue": "std", "MaxReceiveCount": 1}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "std", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1}}),
            "InvalidParameterValue",
        ),
        (json!({"QueueName": "std"}), "MissingParameter"),
        (
            json!({"QueueName": "nope", "Attributes": {"Fifo": true}}),
            "QueueDoesNotExist",
        ),
    ];
    for (mut request, want) in cases {
        request["Command"] = json!("SetQueueAttributes");
        assert_eq!(client.error(request.clone()), want, "request {request}");
    }
}

#[test]
fn set_queue_attributes_changes_only_the_given_attributes() {
    let mut client = Client::default();
    client.create("dlq.fifo", json!({"Fifo": true}));
    client.create("q.fifo", json!({"Fifo": true, "DelaySeconds": 7}));
    let response = client.call(
        json!({"Command": "SetQueueAttributes", "QueueName": "q.fifo", "Attributes": {
        "VisibilityTimeout": 1, "ContentBasedDeduplication": true,
        "DeadLetterQueue": "dlq.fifo", "MaxReceiveCount": 9}}),
    );
    assert_eq!(response, json!({"ok": true}));
    let response = client
        .call(json!({"Command": "SetQueueAttributes", "QueueName": "q.fifo", "Attributes": {}}));
    assert_eq!(response, json!({"ok": true}));
    let response = client.call(json!({"Command": "GetQueueAttributes", "QueueName": "q.fifo"}));
    assert_eq!(
        response["Attributes"],
        json!({
            "VisibilityTimeout": 1,
            "DelaySeconds": 7,
            "RetentionSeconds": 345600,
            "Fifo": true,
            "ContentBasedDeduplication": true,
            "DeadLetterQueue": "dlq.fifo",
            "MaxReceiveCount": 9
        })
    );
}

#[test]
fn at_most_1000_queues_exist() {
    let mut client = Client::default();
    for n in 0..1000 {
        client.create(&format!("q{n}"), json!({}));
    }
    // Re-creating an existing queue is still fine at the limit.
    client.create("q0", json!({}));
    let code = client.error(json!({"Command": "CreateQueue", "QueueName": "one-more"}));
    assert_eq!(code, "LimitExceeded");
    client.call(json!({"Command": "DeleteQueue", "QueueName": "q0"}));
    client.create("one-more", json!({}));
}

#[test]
fn tick_advances_the_clock() {
    let mut client = Client::default();
    assert_eq!(
        client.call(json!({"Command": "Tick", "Seconds": 0})),
        json!({"ok": true, "Now": 0})
    );
    assert_eq!(
        client.call(json!({"Command": "Tick", "Seconds": 1209600})),
        json!({"ok": true, "Now": 1209600})
    );
    assert_eq!(
        client.call(json!({"Command": "Tick", "Seconds": 5})),
        json!({"ok": true, "Now": 1209605})
    );
    assert_eq!(client.error(json!({"Command": "Tick"})), "MissingParameter");
}
