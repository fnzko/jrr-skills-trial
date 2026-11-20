mod common;

use common::{Session, counts};
use serde_json::json;

#[test]
fn new_queue_has_the_default_attributes_and_no_messages() {
    let mut s = Session::new();
    s.create("jobs");
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
fn attributes_report_the_dead_letter_configuration_only_when_set() {
    let mut s = Session::new();
    s.create("dead");
    s.create_with(
        "main",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 5, "DelaySeconds": 7}),
    );
    let response = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "main"}));
    assert_eq!(
        response["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 7,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "dead",
            "MaxReceiveCount": 5,
        })
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::new();
    s.create("q");
    s.create("q");
    s.create_with(
        "q",
        json!({"VisibilityTimeout": 30, "Fifo": false, "DelaySeconds": null}),
    );
    for attributes in [
        json!({"VisibilityTimeout": 31}),
        json!({"DelaySeconds": 1}),
        json!({"RetentionSeconds": 61}),
    ] {
        let code =
            s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
        assert_eq!(code, "QueueAlreadyExists", "{attributes}");
    }
}

#[test]
fn create_queue_compares_the_dead_letter_configuration() {
    let mut s = Session::new();
    s.create("dead");
    s.create_with(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3}),
    );
    s.create_with(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3}),
    );
    for attributes in [
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 4}),
        json!({}),
    ] {
        let code =
            s.err(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
        assert_eq!(code, "QueueAlreadyExists", "{attributes}");
    }
}

#[test]
fn create_queue_applies_its_rules_in_order() {
    let mut s = Session::new();
    s.create("standard");
    let create = |name: &str, attributes| json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes});
    assert_eq!(s.err(create("a.fifo", json!({}))), "QueueNameInvalid");
    assert_eq!(
        s.err(create("a", json!({"Fifo": true}))),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.err(create("a", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create("a", json!({"MaxReceiveCount": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create("a", json!({"DeadLetterQueue": "standard"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(create(
            "a",
            json!({"DeadLetterQueue": "ghost", "MaxReceiveCount": 1})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(create(
            "a.fifo",
            json!({"Fifo": true, "DeadLetterQueue": "standard", "MaxReceiveCount": 1})
        )),
        "InvalidParameterValue"
    );
    // A bad name wins over a bad attribute, and a failed command creates nothing.
    assert_eq!(
        s.err(create("a b", json!({"VisibilityTimeout": -1}))),
        "QueueNameInvalid"
    );
    assert_eq!(
        s.ok(json!({"Command": "ListQueues"}))["QueueNames"],
        json!(["standard"])
    );
}

#[test]
fn queue_limit_is_1000_but_recreating_an_existing_queue_still_succeeds() {
    let mut s = Session::new();
    for n in 0..1000 {
        s.create(&format!("q{n}"));
    }
    s.create("q999");
    assert_eq!(
        s.err(json!({"Command": "CreateQueue", "QueueName": "extra"})),
        "LimitExceeded"
    );
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q0"}));
    s.create("extra");
}

#[test]
fn set_queue_attributes_changes_only_the_attributes_given() {
    let mut s = Session::new();
    s.create_with("q", json!({"DelaySeconds": 4}));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"VisibilityTimeout": 9, "RetentionSeconds": null}}));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}));
    let attributes =
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert_eq!(
        attributes,
        json!({
            "VisibilityTimeout": 9,
            "DelaySeconds": 4,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
        })
    );
}

#[test]
fn set_queue_attributes_applies_its_rules_in_order_and_changes_nothing_on_failure() {
    let mut s = Session::new();
    s.create("q");
    s.create_with("f.fifo", json!({"Fifo": true}));
    let set = |queue: &str, attributes| json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes});
    assert_eq!(
        s.err(set("q", json!({"Fifo": false}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        s.err(set("q", json!({"Fifo": "no"}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set("q", json!({"ContentBasedDeduplication": true}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set("q", json!({"MaxReceiveCount": 2}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"VisibilityTimeout": 5, "DeadLetterQueue": "ghost", "MaxReceiveCount": 2})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"DeadLetterQueue": "q", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(set(
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        s.err(json!({"Command": "SetQueueAttributes", "QueueName": "q"})),
        "MissingParameter"
    );
    assert_eq!(
        s.err(set("ghost", json!({"Fifo": true}))),
        "QueueDoesNotExist"
    );
    let attributes =
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert_eq!(attributes["VisibilityTimeout"], json!(30));
    s.ok(set("f.fifo", json!({"ContentBasedDeduplication": true})));
}

#[test]
fn a_dead_letter_configuration_can_be_replaced_but_not_removed() {
    let mut s = Session::new();
    s.create("d1");
    s.create("d2");
    s.create_with("q", json!({"DeadLetterQueue": "d1", "MaxReceiveCount": 2}));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "d2", "MaxReceiveCount": 9}}));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": null, "MaxReceiveCount": null, "DelaySeconds": 1}}));
    let attributes =
        s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"].clone();
    assert_eq!(attributes["DeadLetterQueue"], json!("d2"));
    assert_eq!(attributes["MaxReceiveCount"], json!(9));
    assert_eq!(attributes["DelaySeconds"], json!(1));
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in [
        "b", "a", "ab", "a_", "aB", "a0", "a.fifo", "a-1", "A", "_", "0",
    ] {
        if name.ends_with(".fifo") {
            s.create_with(name, json!({"Fifo": true}));
        } else {
            s.create(name);
        }
    }
    let list = |s: &mut Session, command| s.ok(command)["QueueNames"].clone();
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues"})),
        json!([
            "0", "A", "_", "a", "a-1", "a.fifo", "a0", "aB", "a_", "ab", "b"
        ])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": "a"})),
        json!(["a", "a-1", "a.fifo", "a0", "aB", "a_", "ab"])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": "a."})),
        json!(["a.fifo"])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": "zz"})),
        json!([])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": null}))
            .as_array()
            .unwrap()
            .len(),
        11
    );
    assert_eq!(
        s.err(json!({"Command": "ListQueues", "Prefix": 1})),
        "InvalidParameterValue"
    );
}

#[test]
fn deleting_a_queue_clears_dead_letter_configurations_that_name_it() {
    let mut s = Session::new();
    s.create("dead");
    s.create_with(
        "main",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
    );
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dead"}));
    let response = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "main"}));
    assert_eq!(response["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(response["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(
        s.err(json!({"Command": "DeleteQueue", "QueueName": "dead"})),
        "QueueDoesNotExist"
    );
    s.create("dead");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "main",
        "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}));
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let mut s = Session::new();
    s.create("q");
    s.send_message("q", "old");
    let handle = common::delivered(&s.receive("q"))[0].1.clone();
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q"}));
    s.create("q");
    assert_eq!(s.counts("q"), counts(0, 0, 0));
    assert_eq!(
        s.delete_message("q", &handle)["error"]["code"],
        json!("ReceiptHandleExpired")
    );
}

#[test]
fn purge_removes_messages_in_every_state() {
    let mut s = Session::new();
    s.create_with("q", json!({"DelaySeconds": 10}));
    s.send_message("q", "delayed");
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "visible", "DelaySeconds": 0}));
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "flying", "DelaySeconds": 0}));
    s.receive_up_to("q", 2);
    assert_eq!(s.counts("q"), counts(0, 2, 1));
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(s.counts("q"), counts(0, 0, 0));
    assert_eq!(
        s.err(json!({"Command": "PurgeQueue", "QueueName": "ghost"})),
        "QueueDoesNotExist"
    );
}

#[test]
fn tick_returns_the_new_clock_value() {
    let mut s = Session::new();
    let tick = |seconds| json!({"Command": "Tick", "Seconds": seconds});
    assert_eq!(s.ok(tick(0)), json!({"ok": true, "Now": 0}));
    assert_eq!(s.ok(tick(5)), json!({"ok": true, "Now": 5}));
    assert_eq!(s.ok(tick(1_209_600)), json!({"ok": true, "Now": 1_209_605}));
    assert_eq!(s.err(tick(1_209_601)), "InvalidParameterValue");
    assert_eq!(s.err(json!({"Command": "Tick"})), "MissingParameter");
}

#[test]
fn queue_lookup_uses_the_exact_string_whatever_it_looks_like() {
    let mut s = Session::new();
    for name in ["", "no such queue!", "A.fifo"] {
        for command in [
            "GetQueueAttributes",
            "DeleteQueue",
            "PurgeQueue",
            "ReceiveMessage",
        ] {
            assert_eq!(
                s.err(json!({"Command": command, "QueueName": name})),
                "QueueDoesNotExist",
                "{command} {name:?}"
            );
        }
    }
}
