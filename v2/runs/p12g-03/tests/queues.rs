mod common;

use common::{error_code, exec, run};
use serde_json::json;

fn create(name: &str, attributes: &str) -> String {
    format!(r#"{{"Command":"CreateQueue","QueueName":"{name}","Attributes":{attributes}}}"#)
}

#[test]
fn queue_names_are_checked_in_create_only() {
    let long_ok = "a".repeat(80);
    let too_long = "a".repeat(81);
    let long_fifo = format!("{}.fifo", "a".repeat(75));
    let too_long_fifo = format!("{}.fifo", "a".repeat(76));
    let cases = [
        ("jobs", "{}", None),
        (long_ok.as_str(), "{}", None),
        (too_long.as_str(), "{}", Some("QueueNameInvalid")),
        ("", "{}", Some("QueueNameInvalid")),
        ("a b", "{}", Some("QueueNameInvalid")),
        ("a.b", "{}", Some("QueueNameInvalid")),
        ("\u{e9}", "{}", Some("QueueNameInvalid")),
        (".fifo", r#"{"Fifo":true}"#, Some("QueueNameInvalid")),
        (long_fifo.as_str(), r#"{"Fifo":true}"#, None),
        (
            too_long_fifo.as_str(),
            r#"{"Fifo":true}"#,
            Some("QueueNameInvalid"),
        ),
        ("q.fifo", "{}", Some("QueueNameInvalid")),
        ("q", r#"{"Fifo":true}"#, Some("QueueNameInvalid")),
    ];
    for (name, attributes, code) in cases {
        let response = &run(&[&create(name, attributes)])[0];
        assert_eq!(error_code(response), code, "{name:?}");
    }
    let response = &run(&[r#"{"Command":"DeleteQueue","QueueName":".fifo"}"#])[0];
    assert_eq!(error_code(response), Some("QueueDoesNotExist"));
    let response = &run(&[r#"{"Command":"DeleteQueue","QueueName":"a b"}"#])[0];
    assert_eq!(error_code(response), Some("QueueDoesNotExist"));
}

#[test]
fn create_checks_name_before_other_parameters() {
    let responses = exec(&[
        json!({"Command": "CreateQueue", "QueueName": "a b", "Attributes": {"Bogus": 1}}),
        json!({"Command": "CreateQueue", "QueueName": 5}),
        json!({"Command": "CreateQueue", "Attributes": {"Bogus": 1}}),
    ]);
    assert_eq!(error_code(&responses[0]), Some("QueueNameInvalid"));
    assert_eq!(error_code(&responses[1]), Some("InvalidParameterValue"));
    assert_eq!(error_code(&responses[2]), Some("MissingParameter"));
}

#[test]
fn create_attribute_values_are_validated() {
    let cases = [
        r#"{"VisibilityTimeout":43201}"#,
        r#"{"VisibilityTimeout":-1}"#,
        r#"{"VisibilityTimeout":"30"}"#,
        r#"{"DelaySeconds":901}"#,
        r#"{"RetentionSeconds":59}"#,
        r#"{"RetentionSeconds":1209601}"#,
        r#"{"Fifo":"true"}"#,
        r#"{"MaxReceiveCount":0}"#,
        r#"{"MaxReceiveCount":1001,"DeadLetterQueue":"d"}"#,
        r#"{"DeadLetterQueue":5,"MaxReceiveCount":1}"#,
        r#"{"Unknown":1}"#,
        r#"{"ContentBasedDeduplication":true}"#,
        r#"{"DeadLetterQueue":"d"}"#,
        r#"{"MaxReceiveCount":3}"#,
    ];
    for attributes in cases {
        let response = &run(&[&create("q", attributes)])[0];
        assert_eq!(
            error_code(response),
            Some("InvalidParameterValue"),
            "{attributes}"
        );
    }
    let not_object = r#"{"Command":"CreateQueue","QueueName":"q","Attributes":[]}"#;
    assert_eq!(
        error_code(&run(&[not_object])[0]),
        Some("InvalidParameterValue")
    );
}

#[test]
fn null_attributes_count_as_absent() {
    let responses = run(&[
        &create(
            "q",
            r#"{"VisibilityTimeout":null,"Bogus":null,"Fifo":null}"#,
        ),
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":null}"#,
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(responses[1], json!({"ok": true}));
}

#[test]
fn create_is_idempotent_only_for_equal_attributes() {
    let responses = run(&[
        &create("q", r#"{"VisibilityTimeout":10}"#),
        &create("q", r#"{"VisibilityTimeout":10,"DelaySeconds":0}"#),
        &create("q", r#"{"VisibilityTimeout":11}"#),
        &create("q", "{}"),
    ]);
    assert_eq!(responses[0], json!({"ok": true}));
    assert_eq!(responses[1], json!({"ok": true}));
    assert_eq!(error_code(&responses[2]), Some("QueueAlreadyExists"));
    assert_eq!(error_code(&responses[3]), Some("QueueAlreadyExists"));
}

#[test]
fn dead_letter_queue_must_exist_and_match_the_type() {
    let responses = run(&[
        &create("d", "{}"),
        &create("f.fifo", r#"{"Fifo":true}"#),
        &create("q", r#"{"DeadLetterQueue":"missing","MaxReceiveCount":2}"#),
        &create(
            "g.fifo",
            r#"{"Fifo":true,"DeadLetterQueue":"d","MaxReceiveCount":2}"#,
        ),
        &create("q", r#"{"DeadLetterQueue":"d","MaxReceiveCount":2}"#),
        &create("q", r#"{"DeadLetterQueue":"d","MaxReceiveCount":2}"#),
        &create("q", r#"{"DeadLetterQueue":"d","MaxReceiveCount":3}"#),
        &create("r", r#"{"DeadLetterQueue":"f.fifo","MaxReceiveCount":3}"#),
    ]);
    let codes: Vec<_> = responses.iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            None,
            None,
            Some("QueueDoesNotExist"),
            Some("InvalidParameterValue"),
            None,
            None,
            Some("QueueAlreadyExists"),
            Some("InvalidParameterValue"),
        ]
    );
}

#[test]
fn existing_queue_check_comes_before_dead_letter_lookup() {
    let responses = run(&[
        &create("q", "{}"),
        &create("q", r#"{"DeadLetterQueue":"missing","MaxReceiveCount":2}"#),
    ]);
    assert_eq!(error_code(&responses[1]), Some("QueueAlreadyExists"));
}

#[test]
fn queue_count_is_limited_to_1000() {
    let mut commands: Vec<String> = (0..1000).map(|i| create(&format!("q{i}"), "{}")).collect();
    commands.push(create("one-too-many", "{}"));
    commands.push(create("q0", "{}"));
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let responses = run(&refs);
    assert!(responses[..1000].iter().all(|r| r == &json!({"ok": true})));
    assert_eq!(error_code(&responses[1000]), Some("LimitExceeded"));
    assert_eq!(responses[1001], json!({"ok": true}));
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let names = ["b", "a.fifo", "Z", "_x", "a-1", "a1", "a"];
    let mut commands: Vec<String> = names
        .iter()
        .map(|name| {
            let attributes = if name.ends_with(".fifo") {
                r#"{"Fifo":true}"#
            } else {
                "{}"
            };
            create(name, attributes)
        })
        .collect();
    commands.push(r#"{"Command":"ListQueues"}"#.to_owned());
    commands.push(r#"{"Command":"ListQueues","Prefix":"a"}"#.to_owned());
    commands.push(r#"{"Command":"ListQueues","Prefix":"zzz"}"#.to_owned());
    commands.push(r#"{"Command":"ListQueues","Prefix":3}"#.to_owned());
    let refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let responses = run(&refs);
    assert_eq!(
        responses[7],
        json!({"ok": true, "QueueNames": ["Z", "_x", "a", "a-1", "a.fifo", "a1", "b"]})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "QueueNames": ["a", "a-1", "a.fifo", "a1"]})
    );
    assert_eq!(responses[9], json!({"ok": true, "QueueNames": []}));
    assert_eq!(error_code(&responses[10]), Some("InvalidParameterValue"));
}

#[test]
fn set_queue_attributes_rules_in_order() {
    let set = |queue: &str, attributes: &str| {
        format!(
            r#"{{"Command":"SetQueueAttributes","QueueName":"{queue}","Attributes":{attributes}}}"#
        )
    };
    let responses = run(&[
        &create("q", "{}"),
        &create("f.fifo", r#"{"Fifo":true}"#),
        &set("q", "{}"),
        &set("q", r#"{"Fifo":false}"#),
        &set("q", r#"{"ContentBasedDeduplication":true}"#),
        &set("q", r#"{"ContentBasedDeduplication":false}"#),
        &set("q", r#"{"MaxReceiveCount":2}"#),
        &set("q", r#"{"DeadLetterQueue":"nope","MaxReceiveCount":2}"#),
        &set("q", r#"{"DeadLetterQueue":"q","MaxReceiveCount":2}"#),
        &set("q", r#"{"DeadLetterQueue":"f.fifo","MaxReceiveCount":2}"#),
        &set("missing", r#"{"Bogus":1}"#),
        &set("missing", r#"{}"#),
        r#"{"Command":"SetQueueAttributes","QueueName":"q"}"#,
        &set(
            "q",
            r#"{"Fifo":true,"ContentBasedDeduplication":true,"MaxReceiveCount":1}"#,
        ),
    ]);
    let codes: Vec<_> = responses.iter().map(error_code).collect();
    assert_eq!(
        codes,
        [
            None,
            None,
            None,
            Some("UnsupportedOperation"),
            Some("InvalidParameterValue"),
            None,
            Some("InvalidParameterValue"),
            Some("QueueDoesNotExist"),
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
            Some("InvalidParameterValue"),
            Some("QueueDoesNotExist"),
            Some("MissingParameter"),
            Some("UnsupportedOperation"),
        ]
    );
}

#[test]
fn deleting_a_queue_removes_it_and_clears_dead_letter_references() {
    let responses = run(&[
        &create("d", "{}"),
        &create("q", r#"{"DeadLetterQueue":"d","MaxReceiveCount":2}"#),
        r#"{"Command":"DeleteQueue","QueueName":"d"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"d"}"#,
        &create("q", "{}"),
        &create("q", r#"{"DeadLetterQueue":"d","MaxReceiveCount":2}"#),
        r#"{"Command":"ListQueues"}"#,
    ]);
    assert_eq!(responses[2], json!({"ok": true}));
    assert_eq!(error_code(&responses[3]), Some("QueueDoesNotExist"));
    assert_eq!(
        responses[4],
        json!({"ok": true}),
        "dead-letter configuration was cleared"
    );
    assert_eq!(error_code(&responses[5]), Some("QueueAlreadyExists"));
    assert_eq!(responses[6], json!({"ok": true, "QueueNames": ["q"]}));
}
