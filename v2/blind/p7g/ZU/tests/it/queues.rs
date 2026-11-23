use crate::common::{assert_responses, err, ok, run};
use serde_json::json;

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    assert_responses(
        run(&[
            r#"{"Command":"CreateQueue","QueueName":"jobs"}"#,
            r#"{"Command":"CreateQueue","QueueName":"jobs","Attributes":{}}"#,
            r#"{"Command":"CreateQueue","QueueName":"jobs","Attributes":{"VisibilityTimeout":30,"DelaySeconds":0,"RetentionSeconds":345600,"Fifo":false,"ContentBasedDeduplication":null}}"#,
            r#"{"Command":"CreateQueue","QueueName":"jobs","Attributes":{"VisibilityTimeout":31}}"#,
            r#"{"Command":"CreateQueue","QueueName":"jobs","Attributes":{"ContentBasedDeduplication":false}}"#,
        ]),
        vec![ok(), ok(), ok(), err("QueueAlreadyExists"), ok()],
    );
}

#[test]
fn create_queue_name_rules_come_first() {
    let long_fifo = format!("{}.fifo", "a".repeat(76));
    let create_long = format!(
        r#"{{"Command":"CreateQueue","QueueName":"{long_fifo}","Attributes":{{"Fifo":true}}}}"#
    );
    assert_responses(
        run(&[
            r#"{"Command":"CreateQueue","QueueName":"bad name","Attributes":"not an object"}"#,
            r#"{"Command":"CreateQueue","QueueName":"jobs.fifo"}"#,
            r#"{"Command":"CreateQueue","QueueName":"jobs","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"jobs.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
            &create_long,
            r#"{"Command":"CreateQueue","QueueName":7}"#,
            r#"{"Command":"CreateQueue"}"#,
        ]),
        vec![
            err("QueueNameInvalid"),
            err("QueueNameInvalid"),
            err("QueueNameInvalid"),
            ok(),
            err("QueueNameInvalid"),
            err("InvalidParameterValue"),
            err("MissingParameter"),
        ],
    );
}

#[test]
fn create_queue_validates_attributes() {
    let bad = [
        r#""Attributes": []"#,
        r#""Attributes": {"Unknown": 1}"#,
        r#""Attributes": {"VisibilityTimeout": 43201}"#,
        r#""Attributes": {"VisibilityTimeout": 3.0}"#,
        r#""Attributes": {"DelaySeconds": "1"}"#,
        r#""Attributes": {"RetentionSeconds": 59}"#,
        r#""Attributes": {"Fifo": "true"}"#,
        r#""Attributes": {"MaxReceiveCount": 0, "DeadLetterQueue": "dead"}"#,
        r#""Attributes": {"DeadLetterQueue": 5}"#,
        r#""Attributes": {"ContentBasedDeduplication": true}"#,
        r#""Attributes": {"DeadLetterQueue": "dead"}"#,
        r#""Attributes": {"MaxReceiveCount": 3}"#,
    ];
    let lines: Vec<String> = bad
        .iter()
        .map(|attributes| format!(r#"{{"Command":"CreateQueue","QueueName":"q",{attributes}}}"#))
        .collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    assert_responses(run(&lines), vec![err("InvalidParameterValue"); bad.len()]);
}

#[test]
fn dead_letter_queue_must_exist_and_match_the_type() {
    assert_responses(
        run(&[
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":2}}"#,
            r#"{"Command":"CreateQueue","QueueName":"dead.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"dead.fifo","MaxReceiveCount":2}}"#,
            r#"{"Command":"CreateQueue","QueueName":"dead"}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":2}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":2}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":3}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"gone","MaxReceiveCount":2}}"#,
        ]),
        vec![
            err("QueueDoesNotExist"),
            ok(),
            err("InvalidParameterValue"),
            ok(),
            ok(),
            ok(),
            err("QueueAlreadyExists"),
            err("QueueAlreadyExists"),
        ],
    );
}

#[test]
fn at_most_1000_queues_can_exist() {
    let mut lines: Vec<String> = (0..1000)
        .map(|n| format!(r#"{{"Command":"CreateQueue","QueueName":"q{n}"}}"#))
        .collect();
    lines.push(r#"{"Command":"CreateQueue","QueueName":"one-too-many"}"#.to_owned());
    lines.push(r#"{"Command":"CreateQueue","QueueName":"q0"}"#.to_owned());
    lines.push(r#"{"Command":"DeleteQueue","QueueName":"q0"}"#.to_owned());
    lines.push(r#"{"Command":"CreateQueue","QueueName":"one-too-many"}"#.to_owned());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut responses = run(&lines);
    let tail = responses.split_off(1000);
    assert_eq!(responses, vec![ok(); 1000]);
    assert_responses(tail, vec![err("LimitExceeded"), ok(), ok(), ok()]);
}

#[test]
fn set_queue_attributes_applies_rules_in_order() {
    assert_responses(
        run(&[
            r#"{"Command":"SetQueueAttributes","QueueName":"missing","Attributes":{}}"#,
            r#"{"Command":"CreateQueue","QueueName":"std"}"#,
            r#"{"Command":"CreateQueue","QueueName":"other"}"#,
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std"}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":null}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"Fifo":false}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"Fifo":"x"}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"MaxReceiveCount":2}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":2}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"DeadLetterQueue":"std","MaxReceiveCount":2}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"DeadLetterQueue":"f.fifo","MaxReceiveCount":2}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"DeadLetterQueue":"other","MaxReceiveCount":2,"VisibilityTimeout":5}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"f.fifo","Attributes":{"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"std","Attributes":{"ContentBasedDeduplication":false}}"#,
        ]),
        vec![
            err("QueueDoesNotExist"),
            ok(),
            ok(),
            ok(),
            err("MissingParameter"),
            err("MissingParameter"),
            err("UnsupportedOperation"),
            err("InvalidParameterValue"),
            err("InvalidParameterValue"),
            err("InvalidParameterValue"),
            err("QueueDoesNotExist"),
            err("InvalidParameterValue"),
            err("InvalidParameterValue"),
            ok(),
            ok(),
            ok(),
        ],
    );
}

#[test]
fn set_queue_attributes_changes_what_create_queue_compares() {
    assert_responses(
        run(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":99,"DelaySeconds":5}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":99,"DelaySeconds":5}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
        ]),
        vec![ok(), ok(), ok(), err("QueueAlreadyExists")],
    );
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    let names = ["b", "a_1", "a-1", "A1", "a1", "a", "a.fifo"];
    let mut lines: Vec<String> = names
        .iter()
        .map(|name| {
            let fifo = name.ends_with(".fifo");
            format!(
                r#"{{"Command":"CreateQueue","QueueName":"{name}","Attributes":{{"Fifo":{fifo}}}}}"#
            )
        })
        .collect();
    lines.extend(
        [
            r#"{"Command":"ListQueues"}"#,
            r#"{"Command":"ListQueues","Prefix":null}"#,
            r#"{"Command":"ListQueues","Prefix":""}"#,
            r#"{"Command":"ListQueues","Prefix":"a"}"#,
            r#"{"Command":"ListQueues","Prefix":"a1"}"#,
            r#"{"Command":"ListQueues","Prefix":"zzz"}"#,
            r#"{"Command":"ListQueues","Prefix":5}"#,
        ]
        .map(str::to_owned),
    );
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = run(&lines);
    let all = json!({"ok": true, "QueueNames": ["A1", "a", "a-1", "a.fifo", "a1", "a_1", "b"]});
    assert_responses(
        responses[names.len()..].to_vec(),
        vec![
            all.clone(),
            all.clone(),
            all,
            json!({"ok": true, "QueueNames": ["a", "a-1", "a.fifo", "a1", "a_1"]}),
            json!({"ok": true, "QueueNames": ["a1"]}),
            json!({"ok": true, "QueueNames": []}),
            err("InvalidParameterValue"),
        ],
    );
}

#[test]
fn delete_queue_unsets_dead_letter_configuration_of_its_users() {
    assert_responses(
        run(&[
            r#"{"Command":"CreateQueue","QueueName":"dead"}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":2}}"#,
            r#"{"Command":"DeleteQueue","QueueName":"dead"}"#,
            r#"{"Command":"DeleteQueue","QueueName":"dead"}"#,
            r#"{"Command":"CreateQueue","QueueName":"a"}"#,
            r#"{"Command":"CreateQueue","QueueName":"dead"}"#,
            r#"{"Command":"CreateQueue","QueueName":"a"}"#,
            r#"{"Command":"ListQueues"}"#,
            r#"{"Command":"DeleteQueue"}"#,
            r#"{"Command":"DeleteQueue","QueueName":"a b"}"#,
        ]),
        vec![
            ok(),
            ok(),
            ok(),
            err("QueueDoesNotExist"),
            ok(),
            ok(),
            ok(),
            json!({"ok": true, "QueueNames": ["a", "dead"]}),
            err("MissingParameter"),
            err("QueueDoesNotExist"),
        ],
    );
}
