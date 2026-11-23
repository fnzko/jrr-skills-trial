mod common;

use common::Session;
use serde_json::json;

#[test]
fn create_applies_defaults_and_get_reports_them() {
    let mut s = Session::new();
    s.create("jobs", json!({}));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}));
    assert_eq!(
        r["Attributes"],
        json!({"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
               "Fifo": false, "ContentBasedDeduplication": false})
    );
    assert_eq!(
        r["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn create_name_and_type_rules() {
    let mut s = Session::new();
    let create = |n: &str, a| json!({"Command": "CreateQueue", "QueueName": n, "Attributes": a});
    assert_eq!(s.err(create("a b", json!({}))), "QueueNameInvalid");
    assert_eq!(s.err(create("q.fifo", json!({}))), "QueueNameInvalid");
    assert_eq!(
        s.err(create("q", json!({"Fifo": true}))),
        "QueueNameInvalid"
    );
    // The name is checked before any other parameter value.
    assert_eq!(
        s.err(create("a b", json!({"DelaySeconds": 5000}))),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(create("q", json!({"DelaySeconds": 5000}))),
        "InvalidParameterValue"
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
        s.err(create("q", json!({"Bogus": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": []})),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": 7})),
        "InvalidParameterValue"
    );
    assert_eq!(s.err(json!({"Command": "CreateQueue"})), "MissingParameter");
    s.ok(create(
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    ));
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 5}));
    s.create("q", json!({"DelaySeconds": 5, "VisibilityTimeout": 30}));
    // Null attributes are absent, so this is the default `VisibilityTimeout`.
    s.create("q", json!({"DelaySeconds": 5, "VisibilityTimeout": null}));
    let different = json!({"Command": "CreateQueue", "QueueName": "q"});
    assert_eq!(s.err(different), "QueueAlreadyExists");
}

#[test]
fn dead_letter_configuration_rules() {
    let mut s = Session::new();
    let create = |n: &str, a| json!({"Command": "CreateQueue", "QueueName": n, "Attributes": a});
    let dl = json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2});
    assert_eq!(s.err(create("q", dl.clone())), "QueueDoesNotExist");
    s.create("dlq.fifo", json!({"Fifo": true}));
    s.create("dlq", json!({}));
    s.create("q", dl.clone());
    s.create("q", dl.clone());
    assert_eq!(
        s.err(create(
            "f.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(r["Attributes"]["DeadLetterQueue"], "dlq");
    assert_eq!(r["Attributes"]["MaxReceiveCount"], 2);

    // Deleting the dead-letter queue clears the configuration.
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    let r = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(r["Attributes"].get("DeadLetterQueue").is_none());
    assert!(r["Attributes"].get("MaxReceiveCount").is_none());
}

#[test]
fn queue_limit() {
    let mut s = Session::new();
    for i in 0..1000 {
        s.create(&format!("q{i}"), json!({}));
    }
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
    // Re-creating an existing queue is not a new queue.
    s.create("q1", json!({}));
}

#[test]
fn set_queue_attributes_rules() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.create("d", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |a| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": a});
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
        s.err(set(json!({"DeadLetterQueue": "d"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(json!({"DeadLetterQueue": "x", "MaxReceiveCount": 1}))),
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
    assert_eq!(
        s.err(json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}})),
        "QueueDoesNotExist"
    );
    // A failed command changes nothing.
    assert_eq!(
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"]["DelaySeconds"],
        0
    );

    s.ok(set(
        json!({"DelaySeconds": 7, "DeadLetterQueue": "d", "MaxReceiveCount": 4}),
    ));
    let a = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert_eq!(a["DelaySeconds"], 7);
    assert_eq!(a["VisibilityTimeout"], 30);
    assert_eq!(a["DeadLetterQueue"], "d");
    s.ok(set(json!({})));
}

#[test]
fn list_queues_sorts_by_bytes_and_filters_by_prefix() {
    let mut s = Session::new();
    for n in ["b", "a_", "a-", "a.fifo", "a0", "aZ", "az"] {
        s.create(n, json!({"Fifo": n.ends_with(".fifo")}));
    }
    let names = |s: &mut Session, c| s.ok(c)["QueueNames"].clone();
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues"})),
        json!(["a-", "a.fifo", "a0", "aZ", "a_", "az", "b"])
    );
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues", "Prefix": "a"}))[0],
        "a-"
    );
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues", "Prefix": "b"})),
        json!(["b"])
    );
    assert_eq!(
        names(&mut s, json!({"Command": "ListQueues", "Prefix": "zz"})),
        json!([])
    );
    assert_eq!(
        s.err(json!({"Command": "ListQueues", "Prefix": 1})),
        "InvalidParameterValue"
    );
}

#[test]
fn delete_and_purge() {
    let mut s = Session::new();
    s.create("q", json!({}));
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    assert_eq!(
        s.err(json!({"Command": "DeleteQueue", "QueueName": "q"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(json!({"Command": "PurgeQueue", "QueueName": "q"})),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(json!({"Command": "GetQueueAttributes", "QueueName": "q"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn tick_validates_and_reports_the_clock() {
    let mut s = Session::new();
    assert_eq!(s.ok(json!({"Command": "Tick", "Seconds": 0}))["Now"], 0);
    assert_eq!(
        s.ok(json!({"Command": "Tick", "Seconds": 1209600}))["Now"],
        1209600
    );
    assert_eq!(
        s.err(json!({"Command": "Tick", "Seconds": 1209601})),
        "InvalidParameterValue"
    );
    assert_eq!(s.err(json!({"Command": "Tick"})), "MissingParameter");
    assert_eq!(
        s.err(json!({"Command": "Tick", "Seconds": null})),
        "MissingParameter"
    );
    let r = s.run(serde_json::from_str(r#"{"Command": "Tick", "Seconds": 1.0}"#).unwrap());
    assert_eq!(r["error"]["code"], "InvalidParameterValue");
}

#[test]
fn request_shape_errors() {
    let mut svc = miniq::Service::new();
    for line in [
        "",
        "[]",
        "42",
        "{",
        "{\"Command\": 1}",
        "{\"Command\": \"Nope\"}",
        "{\"command\": \"Tick\"}",
        "{} x",
    ] {
        let r = svc.handle_line(line.as_bytes());
        assert_eq!(r["error"]["code"], "InvalidRequest", "{line:?}");
    }
}
