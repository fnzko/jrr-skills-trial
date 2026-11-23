mod common;

use common::{error_code, run};
use serde_json::json;

#[test]
fn create_applies_defaults_and_is_idempotent_for_equal_attributes() {
    let out = run(&[
        json!({"Command": "CreateQueue", "QueueName": "jobs"}),
        json!({"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 30}}),
        json!({"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 31}}),
        json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}),
    ]);
    assert_eq!(out[0], json!({"ok": true}));
    assert_eq!(out[1], json!({"ok": true}));
    assert_eq!(error_code(&out[2]), "QueueAlreadyExists");
    assert_eq!(
        out[3],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                "Fifo": false, "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}
        })
    );
}

#[test]
fn queue_name_rules() {
    let long_ok = "a".repeat(80);
    let long_bad = "a".repeat(81);
    let fifo_ok = format!("{}.fifo", "a".repeat(75));
    let fifo_bad = format!("{}.fifo", "a".repeat(76));
    let names = [
        ("", "QueueNameInvalid"),
        ("a b", "QueueNameInvalid"),
        ("a.b", "QueueNameInvalid"),
        (".fifo", "QueueNameInvalid"),
        (long_ok.as_str(), "ok"),
        (long_bad.as_str(), "QueueNameInvalid"),
        (fifo_ok.as_str(), "QueueNameInvalid"), // suffix without Fifo=true
    ];
    for (name, expected) in names {
        let out = run(&[json!({"Command": "CreateQueue", "QueueName": name})]);
        let got = out[0]["error"]["code"].as_str().unwrap_or("ok");
        assert_eq!(got, expected, "name {name:?}");
    }
    let out = run(&[
        json!({"Command": "CreateQueue", "QueueName": fifo_ok, "Attributes": {"Fifo": true}}),
        json!({"Command": "CreateQueue", "QueueName": fifo_bad, "Attributes": {"Fifo": true}}),
        json!({"Command": "CreateQueue", "QueueName": "plain", "Attributes": {"Fifo": true}}),
    ]);
    assert_eq!(out[0], json!({"ok": true}));
    assert_eq!(error_code(&out[1]), "QueueNameInvalid");
    assert_eq!(error_code(&out[2]), "QueueNameInvalid");
}

#[test]
fn name_is_checked_before_other_parameters() {
    let out = run(&[json!({"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 5})]);
    assert_eq!(error_code(&out[0]), "QueueNameInvalid");
    let out = run(&[json!({"Command": "CreateQueue", "QueueName": 5, "Attributes": 5})]);
    assert_eq!(error_code(&out[0]), "InvalidParameterValue");
}

#[test]
fn attribute_values_are_validated() {
    let bad = [
        json!({"VisibilityTimeout": 43201}),
        json!({"VisibilityTimeout": "1"}),
        json!({"DelaySeconds": 901}),
        json!({"RetentionSeconds": 59}),
        json!({"RetentionSeconds": 1209601}),
        json!({"Fifo": "true"}),
        json!({"MaxReceiveCount": 0, "DeadLetterQueue": "x"}),
        json!({"Unknown": 1}),
        json!({"ContentBasedDeduplication": true}),
        json!({"DeadLetterQueue": "x"}),
        json!({"MaxReceiveCount": 3}),
    ];
    for attributes in bad {
        let out =
            run(&[json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes})]);
        assert_eq!(error_code(&out[0]), "InvalidParameterValue", "{attributes}");
    }
    let out = run(&[json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": []})]);
    assert_eq!(error_code(&out[0]), "InvalidParameterValue");
}

#[test]
fn dead_letter_checks_in_create() {
    let out = run(&[
        json!({"Command": "CreateQueue", "QueueName": "std"}),
        json!({"Command": "CreateQueue", "QueueName": "missing-dlq", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}),
        json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true, "DeadLetterQueue": "std", "MaxReceiveCount": 2}}),
        json!({"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"DeadLetterQueue": "std", "MaxReceiveCount": 2}}),
        json!({"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"DeadLetterQueue": "std", "MaxReceiveCount": 2}}),
        json!({"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"DeadLetterQueue": "std", "MaxReceiveCount": 3}}),
        json!({"Command": "GetQueueAttributes", "QueueName": "ok"}),
    ]);
    assert_eq!(error_code(&out[1]), "QueueDoesNotExist");
    assert_eq!(error_code(&out[2]), "InvalidParameterValue");
    assert_eq!(out[3], json!({"ok": true}));
    assert_eq!(out[4], json!({"ok": true}));
    assert_eq!(error_code(&out[5]), "QueueAlreadyExists");
    assert_eq!(out[6]["Attributes"]["DeadLetterQueue"], "std");
    assert_eq!(out[6]["Attributes"]["MaxReceiveCount"], 2);
}

#[test]
fn queue_limit_is_1000() {
    let mut commands: Vec<_> = (0..1000)
        .map(|n| json!({"Command": "CreateQueue", "QueueName": format!("q{n}")}))
        .collect();
    commands.push(json!({"Command": "CreateQueue", "QueueName": "one-too-many"}));
    commands.push(json!({"Command": "CreateQueue", "QueueName": "q5"}));
    let out = run(&commands);
    assert!(out[..1000].iter().all(|r| r["ok"] == true));
    assert_eq!(error_code(&out[1000]), "LimitExceeded");
    assert_eq!(out[1001], json!({"ok": true}));
}

#[test]
fn set_attributes_rules_in_order() {
    let out = run(&[
        json!({"Command": "CreateQueue", "QueueName": "q"}),
        json!({"Command": "CreateQueue", "QueueName": "f.fifo", "Attributes": {"Fifo": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": false}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"ContentBasedDeduplication": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"ContentBasedDeduplication": true, "DelaySeconds": 5}}),
        json!({"Command": "GetQueueAttributes", "QueueName": "f.fifo"}),
    ]);
    let codes: Vec<_> = out[2..10].iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            "UnsupportedOperation",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "InvalidParameterValue",
            "InvalidParameterValue",
            "QueueDoesNotExist",
            "MissingParameter",
        ]
    );
    assert_eq!(out[10], json!({"ok": true}));
    assert_eq!(out[11]["Attributes"]["ContentBasedDeduplication"], true);
    assert_eq!(out[11]["Attributes"]["DelaySeconds"], 5);
}

#[test]
fn list_queues_is_sorted_by_bytes_and_filtered_by_prefix() {
    let names = ["b", "a_1", "a-1", "A1", "a1", "a.fifo", "a"];
    let mut commands: Vec<_> = names
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            json!({"Command": "CreateQueue", "QueueName": name, "Attributes": {"Fifo": fifo}})
        })
        .collect();
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zz"}));
    let out = run(&commands);
    assert_eq!(
        out[7]["QueueNames"],
        json!(["A1", "a", "a-1", "a.fifo", "a1", "a_1", "b"])
    );
    assert_eq!(
        out[8]["QueueNames"],
        json!(["a", "a-1", "a.fifo", "a1", "a_1"])
    );
    assert_eq!(out[9]["QueueNames"], json!([]));
}

#[test]
fn deleting_a_queue_clears_dead_letter_configs_pointing_at_it() {
    let out = run(&[
        json!({"Command": "CreateQueue", "QueueName": "dead"}),
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1}}),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "DeleteQueue", "QueueName": "dead"}),
    ]);
    assert_eq!(out[2], json!({"ok": true}));
    assert!(out[3]["Attributes"].get("DeadLetterQueue").is_none());
    assert!(out[3]["Attributes"].get("MaxReceiveCount").is_none());
    assert_eq!(error_code(&out[4]), "QueueDoesNotExist");
}

#[test]
fn lookups_of_unknown_queues_fail_whatever_the_name_looks_like() {
    let out = run(&[
        json!({"Command": "GetQueueAttributes", "QueueName": ""}),
        json!({"Command": "PurgeQueue", "QueueName": "not a name!"}),
        json!({"Command": "ReceiveMessage", "QueueName": "x", "MaxMessages": 5}),
        json!({"Command": "SendMessage", "QueueName": "x", "Body": "b"}),
    ]);
    let codes: Vec<_> = out.iter().map(error_code).collect();
    assert_eq!(codes, ["QueueDoesNotExist"; 4]);
}
