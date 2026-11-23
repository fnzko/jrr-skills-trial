mod common;

use common::{code, codes, create, run_cmds};
use serde_json::{Value, json};

#[test]
fn create_and_get_with_defaults() {
    let r = run_cmds(vec![
        json!({"Command": "CreateQueue", "QueueName": "jobs"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}),
    ]);
    assert_eq!(r[0], json!({"ok": true}));
    assert_eq!(
        r[1],
        json!({"ok": true,
            "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                           "Fifo": false, "ContentBasedDeduplication": false},
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}})
    );
}

#[test]
fn create_validates_names_and_attributes() {
    let r = run_cmds(vec![
        json!({"Command": "CreateQueue", "QueueName": ""}),
        json!({"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5}),
        json!({"Command": "CreateQueue", "QueueName": 5}),
        json!({"Command": "CreateQueue"}),
        create("a.fifo", json!({})),
        create("a", json!({"Fifo": true})),
        create("a", json!({"ContentBasedDeduplication": true})),
        create("a", json!({"MaxReceiveCount": 3})),
        create("a", json!({"DeadLetterQueue": "x"})),
        create("a", json!({"Bogus": 1})),
        create("a", json!({"Bogus": null})),
        create("a", json!({"VisibilityTimeout": 43201})),
        create("a", json!({"RetentionSeconds": 59})),
        create("a", json!({"DelaySeconds": "1"})),
        create("a", json!({"Fifo": 1})),
        create("a", json!({"MaxReceiveCount": 0, "DeadLetterQueue": "x"})),
        create("a", json!(null)),
        create("a", json!([])),
        create(
            "fresh",
            json!({"DeadLetterQueue": "missing", "MaxReceiveCount": 1}),
        ),
    ]);
    assert_eq!(
        codes(&r),
        [
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "MissingParameter",
            "QueueNameInvalid",
            "QueueNameInvalid",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "QueueDoesNotExist",
        ]
    );
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let r = run_cmds(vec![
        create("q", json!({"VisibilityTimeout": 10})),
        create("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 0})),
        create("q", json!({})),
        create(
            "q",
            json!({"VisibilityTimeout": 10, "RetentionSeconds": 61}),
        ),
        create("q.fifo", json!({"Fifo": true})),
        create("q.fifo", json!({"Fifo": true})),
        create(
            "q.fifo",
            json!({"Fifo": true, "ContentBasedDeduplication": true}),
        ),
    ]);
    assert_eq!(
        codes(&r),
        [
            "ok",
            "ok",
            "QueueAlreadyExists",
            "QueueAlreadyExists",
            "ok",
            "ok",
            "QueueAlreadyExists"
        ]
    );
}

#[test]
fn dead_letter_configuration_rules() {
    let r = run_cmds(vec![
        create("dlq", json!({})),
        create("dlq.fifo", json!({"Fifo": true})),
        create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2})),
        create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2})),
        create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3})),
        create(
            "f.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
        ),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
    ]);
    assert_eq!(
        codes(&r),
        [
            "ok",
            "ok",
            "ok",
            "ok",
            "QueueAlreadyExists",
            "InvalidParameterValue",
            "ok",
            "ok",
            "ok"
        ]
    );
    assert_eq!(r[6]["Attributes"]["DeadLetterQueue"], "dlq");
    assert_eq!(r[6]["Attributes"]["MaxReceiveCount"], 2);
    assert!(r[8]["Attributes"].get("DeadLetterQueue").is_none());
    assert!(r[8]["Attributes"].get("MaxReceiveCount").is_none());
}

#[test]
fn set_attributes_rules_in_order() {
    let set = |name: &str, attrs: Value| json!({"Command": "SetQueueAttributes", "QueueName": name, "Attributes": attrs});
    let r = run_cmds(vec![
        create("q", json!({})),
        create("o", json!({})),
        create("f.fifo", json!({"Fifo": true})),
        set(
            "q",
            json!({"VisibilityTimeout": 5, "RetentionSeconds": 100}),
        ),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
        set("q", json!({})),
        json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
        set("nope", json!({"Bogus": 1})),
        set("q", json!({"Fifo": false})),
        set("q", json!({"ContentBasedDeduplication": true})),
        set("f.fifo", json!({"ContentBasedDeduplication": true})),
        set("q", json!({"MaxReceiveCount": 1})),
        set(
            "q",
            json!({"DeadLetterQueue": "ghost", "MaxReceiveCount": 1}),
        ),
        set("q", json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})),
        set(
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1}),
        ),
        set("q", json!({"DeadLetterQueue": "o", "MaxReceiveCount": 1})),
        set(
            "q",
            json!({"DeadLetterQueue": "o", "MaxReceiveCount": 7, "VisibilityTimeout": null}),
        ),
    ]);
    assert_eq!(
        codes(&r),
        [
            "ok",
            "ok",
            "ok",
            "ok",
            "ok",
            "ok",
            "MissingParameter",
            "InvalidParameterValue",
            "UnsupportedOperation",
            "InvalidParameterValue",
            "ok",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "ok",
            "ok",
        ]
    );
    assert_eq!(r[4]["Attributes"]["VisibilityTimeout"], 5);
    assert_eq!(r[4]["Attributes"]["RetentionSeconds"], 100);
    assert_eq!(r[4]["Attributes"]["DelaySeconds"], 0);
}

#[test]
fn queue_lookup_precedes_command_rules() {
    let r = run_cmds(vec![
        json!({"Command": "GetQueueAttributes", "QueueName": "ghost"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "bad name!"}),
        json!({"Command": "GetQueueAttributes", "QueueName": 1}),
        json!({"Command": "PurgeQueue", "QueueName": "ghost"}),
        json!({"Command": "DeleteQueue", "QueueName": "ghost"}),
    ]);
    assert_eq!(
        codes(&r),
        [
            "QueueDoesNotExist",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "QueueDoesNotExist"
        ]
    );
}

#[test]
fn list_queues_sorts_bytewise_and_filters_by_prefix() {
    let mut cmds = vec![];
    for n in ["b", "a-1", "a.fifo", "a1", "A", "_x", "z", "a_"] {
        let attrs = if n.ends_with(".fifo") {
            json!({"Fifo": true})
        } else {
            json!({})
        };
        cmds.push(create(n, attrs));
    }
    cmds.push(json!({"Command": "ListQueues"}));
    cmds.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    cmds.push(json!({"Command": "ListQueues", "Prefix": "a."}));
    cmds.push(json!({"Command": "ListQueues", "Prefix": "nothing"}));
    cmds.push(json!({"Command": "ListQueues", "Prefix": 3}));
    cmds.push(json!({"Command": "ListQueues", "Prefix": null}));
    let r = run_cmds(cmds);
    assert_eq!(
        r[8]["QueueNames"],
        json!(["A", "_x", "a-1", "a.fifo", "a1", "a_", "b", "z"])
    );
    assert_eq!(r[9]["QueueNames"], json!(["a-1", "a.fifo", "a1", "a_"]));
    assert_eq!(r[10]["QueueNames"], json!(["a.fifo"]));
    assert_eq!(r[11]["QueueNames"], json!([]));
    assert_eq!(code(&r[12]), "InvalidParameterValue");
    assert_eq!(r[13]["QueueNames"].as_array().unwrap().len(), 8);
}

#[test]
fn queue_limit_is_1000() {
    let mut cmds: Vec<Value> = (0..1000)
        .map(|i| create(&format!("q{i}"), json!({})))
        .collect();
    cmds.push(create("one-too-many", json!({})));
    cmds.push(create("q5", json!({})));
    cmds.push(json!({"Command": "DeleteQueue", "QueueName": "q5"}));
    cmds.push(create("one-too-many", json!({})));
    let r = run_cmds(cmds);
    assert!(r[..1000].iter().all(|r| r["ok"] == true));
    assert_eq!(code(&r[1000]), "LimitExceeded");
    assert_eq!(r[1001]["ok"], true); // idempotent create does not hit the limit
    assert_eq!(r[1003]["ok"], true);
}
