use crate::common::*;
use serde_json::json;

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let responses = session(&[
        create_with("q", json!({"VisibilityTimeout": 10})),
        create_with("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 0})),
        create_with("q", json!({"VisibilityTimeout": 11})),
        create("q"),
    ]);
    assert_eq!(responses[0], ok());
    assert_eq!(responses[1], ok());
    assert_error(&responses[2], "QueueAlreadyExists");
    assert_error(&responses[3], "QueueAlreadyExists");
}

#[test]
fn create_rules_apply_in_the_written_order() {
    let responses = session(&[
        create("jobs.fifo"),
        json!({"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": true}}),
        create_with("a", json!({"ContentBasedDeduplication": true})),
        create_with("a", json!({"DeadLetterQueue": "x"})),
        create_with("a", json!({"MaxReceiveCount": 3})),
        create_with("a", json!({"DeadLetterQueue": "x", "MaxReceiveCount": 3})),
        create_with("fifo.fifo", json!({"Fifo": true})),
        create_with("fifo2.fifo", json!({"Fifo": true})),
        create_with(
            "b",
            json!({"DeadLetterQueue": "fifo2.fifo", "MaxReceiveCount": 3}),
        ),
        create("bad name"),
        create_with("c", json!({"Bogus": 1})),
        create_with("c", json!({"VisibilityTimeout": 43201})),
        create_with("c", json!({"VisibilityTimeout": 3.0})),
        json!({"Command": "CreateQueue"}),
        json!({"Command": "CreateQueue", "QueueName": "c", "Attributes": []}),
    ]);
    let codes = [
        Some("QueueNameInvalid"),
        Some("QueueNameInvalid"),
        Some("InvalidParameterValue"),
        Some("InvalidParameterValue"),
        Some("InvalidParameterValue"),
        Some("QueueDoesNotExist"),
        None,
        None,
        Some("InvalidParameterValue"),
        Some("QueueNameInvalid"),
        Some("InvalidParameterValue"),
        Some("InvalidParameterValue"),
        Some("InvalidParameterValue"),
        Some("MissingParameter"),
        Some("InvalidParameterValue"),
    ];
    for (i, (response, code)) in responses.iter().zip(codes).enumerate() {
        match code {
            None => assert_eq!(*response, ok(), "command {i}"),
            Some(code) => assert_error(response, code),
        }
    }
}

#[test]
fn at_most_1000_queues_exist() {
    let mut commands: Vec<_> = (0..1001).map(|i| create(&format!("q{i}"))).collect();
    commands.push(create("q0"));
    let responses = session(&commands);
    assert!(responses[..1000].iter().all(|r| *r == ok()));
    assert_error(&responses[1000], "LimitExceeded");
    assert_eq!(responses[1001], ok());
}

#[test]
fn attributes_show_defaults_and_counts() {
    let responses = session(&[
        create("q"),
        send("q", "a"),
        attributes("q"),
        attributes("nope"),
    ]);
    assert_eq!(
        responses[2],
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 1, "InFlight": 0, "Delayed": 0}
        })
    );
    assert_error(&responses[3], "QueueDoesNotExist");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let names = ["b", "a_", "A", "a-", "a0", "a.fifo", "aa"];
    let mut commands: Vec<_> = names
        .iter()
        .map(|n| match n.strip_suffix(".fifo") {
            Some(_) => create_with(n, json!({"Fifo": true})),
            None => create(n),
        })
        .collect();
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": ""}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zz"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": 1}));
    let responses = session(&commands);
    let n = names.len();
    assert_eq!(
        responses[n],
        json!({"ok": true, "QueueNames": ["A", "a-", "a.fifo", "a0", "a_", "aa", "b"]})
    );
    assert_eq!(
        responses[n + 1],
        json!({"ok": true, "QueueNames": ["a-", "a.fifo", "a0", "a_", "aa"]})
    );
    assert_eq!(responses[n + 2], responses[n]);
    assert_eq!(responses[n + 3], json!({"ok": true, "QueueNames": []}));
    assert_error(&responses[n + 4], "InvalidParameterValue");
}

#[test]
fn set_attributes_follows_its_rules() {
    let responses = session(&[
        create("s"),
        create("other"),
        create_with("f.fifo", json!({"Fifo": true})),
        json!({"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"Fifo": false}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"ContentBasedDeduplication": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"DeadLetterQueue": "s", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {"DeadLetterQueue": "other", "MaxReceiveCount": 2, "DelaySeconds": 5}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "s", "Attributes": {}}),
        attributes("s"),
        json!({"Command": "SetQueueAttributes", "QueueName": "s"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "f.fifo", "Attributes": {"ContentBasedDeduplication": true}}),
    ]);
    assert_error(&responses[3], "UnsupportedOperation");
    assert_error(&responses[4], "InvalidParameterValue");
    assert_error(&responses[5], "InvalidParameterValue");
    assert_error(&responses[6], "QueueDoesNotExist");
    assert_error(&responses[7], "InvalidParameterValue");
    assert_error(&responses[8], "InvalidParameterValue");
    assert_eq!(responses[9], ok());
    assert_eq!(responses[10], ok());
    assert_eq!(
        responses[11]["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 5,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false,
            "DeadLetterQueue": "other",
            "MaxReceiveCount": 2
        })
    );
    assert_error(&responses[12], "MissingParameter");
    assert_error(&responses[13], "QueueDoesNotExist");
    assert_eq!(responses[14], ok());
}

#[test]
fn delete_queue_detaches_dead_letter_configurations_and_forgets_messages() {
    let responses = session(&[
        create("dlq"),
        create_with(
            "src",
            json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
        ),
        send("dlq", "old"),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        attributes("src"),
        create("dlq"),
        receive("dlq"),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
        json!({"Command": "DeleteQueue", "QueueName": "dlq"}),
    ]);
    assert_eq!(responses[3], ok());
    assert_eq!(responses[4]["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(responses[4]["Attributes"].get("MaxReceiveCount"), None);
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_error(&responses[8], "QueueDoesNotExist");
}

#[test]
fn tick_validates_and_reports_the_clock() {
    let responses = session(&[
        tick(0),
        tick(1_209_600),
        tick(1_209_601),
        tick(5),
        json!({"Command": "Tick", "Seconds": -1}),
        json!({"Command": "Tick", "Seconds": "5"}),
    ]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 0}));
    assert_eq!(responses[1], json!({"ok": true, "Now": 1_209_600}));
    assert_error(&responses[2], "InvalidParameterValue");
    assert_eq!(responses[3], json!({"ok": true, "Now": 1_209_605}));
    assert_error(&responses[4], "InvalidParameterValue");
    assert_error(&responses[5], "InvalidParameterValue");
}
