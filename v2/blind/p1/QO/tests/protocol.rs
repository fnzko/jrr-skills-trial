//! End-to-end behavior through `miniq::run`, one JSON response per line.

use serde_json::{Value, json};

/// Runs `lines` as one session and returns the parsed responses.
fn session(lines: &[&str]) -> Vec<Value> {
    let mut input = lines.join("\n");
    input.push('\n');
    run_bytes(input.as_bytes())
}

fn run_bytes(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).expect("in-memory I/O does not fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    assert!(text.is_empty() || text.ends_with('\n'));
    text.lines()
        .map(|line| serde_json::from_str(line).expect("each response is one JSON object"))
        .collect()
}

fn ok() -> Value {
    json!({"ok": true})
}

fn failure(code: &str) -> Value {
    json!({"ok": false, "error": {"code": code, "message": "-"}})
}

/// Compares responses, treating every error message as equal.
fn assert_responses(actual: Vec<Value>, expected: Vec<Value>) {
    let normalize = |mut response: Value| {
        if let Some(message) = response.pointer_mut("/error/message") {
            *message = json!("-");
        }
        response
    };
    let actual: Vec<Value> = actual.into_iter().map(normalize).collect();
    assert_eq!(actual, expected);
}

#[test]
fn every_line_gets_exactly_one_response_in_order() {
    let responses = run_bytes(b"\n{\"Command\":\"Tick\",\"Seconds\":1}\nnot json\n\xff\n{\"Command\":\"Tick\",\"Seconds\":2}");
    assert_responses(
        responses,
        vec![
            failure("InvalidRequest"),
            json!({"ok": true, "Now": 1}),
            failure("InvalidRequest"),
            failure("InvalidRequest"),
            json!({"ok": true, "Now": 3}),
        ],
    );
}

#[test]
fn empty_input_produces_no_output() {
    assert_eq!(run_bytes(b""), Vec::<Value>::new());
}

#[test]
fn crlf_line_endings_are_accepted() {
    let responses = run_bytes(b"{\"Command\":\"Tick\",\"Seconds\":1}\r\n");
    assert_eq!(responses, vec![json!({"ok": true, "Now": 1})]);
}

#[test]
fn tick_accumulates_and_validates() {
    assert_responses(
        session(&[
            r#"{"Command":"Tick","Seconds":0}"#,
            r#"{"Command":"Tick","Seconds":1209600}"#,
            r#"{"Command":"Tick","Seconds":1209601}"#,
            r#"{"Command":"Tick","Seconds":5.0}"#,
            r#"{"Command":"Tick","Seconds":null}"#,
            r#"{"Command":"Tick","Seconds":1}"#,
        ]),
        vec![
            json!({"ok": true, "Now": 0}),
            json!({"ok": true, "Now": 1_209_600}),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("MissingParameter"),
            json!({"ok": true, "Now": 1_209_601}),
        ],
    );
}

#[test]
fn create_queue_applies_defaults_and_reports_them() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"jobs"}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"jobs"}"#,
            r#"{"Command":"CreateQueue","QueueName":"orders.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true,"DelaySeconds":null}}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"orders.fifo"}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true,
                "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                               "Fifo": false, "ContentBasedDeduplication": false},
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            ok(),
            json!({"ok": true,
                "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                               "Fifo": true, "ContentBasedDeduplication": true},
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
        ],
    );
}

#[test]
fn create_queue_error_order_follows_the_numbered_rules() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"bad name","Attributes":{"Bogus":1}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a.fifo"}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"x"}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"MaxReceiveCount":2}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"x","MaxReceiveCount":2}}"#,
            r#"{"Command":"CreateQueue","QueueName":"dead.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"DeadLetterQueue":"dead.fifo","MaxReceiveCount":2}}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"a"}"#,
        ]),
        vec![
            failure("QueueNameInvalid"),
            failure("QueueNameInvalid"),
            failure("QueueNameInvalid"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("QueueDoesNotExist"),
            ok(),
            failure("InvalidParameterValue"),
            failure("QueueDoesNotExist"),
        ],
    );
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"dead"}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":10,"DeadLetterQueue":"dead","MaxReceiveCount":3}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":10,"DeadLetterQueue":"dead","MaxReceiveCount":3}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":10,"DeadLetterQueue":"dead","MaxReceiveCount":4}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":10}}"#,
            r#"{"Command":"CreateQueue","QueueName":"dead","Attributes":{"VisibilityTimeout":30,"Fifo":false}}"#,
            r#"{"Command":"CreateQueue","QueueName":"dead","Attributes":{"DelaySeconds":1}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":10,"DeadLetterQueue":"missing","MaxReceiveCount":3}}"#,
        ]),
        vec![
            ok(),
            ok(),
            ok(),
            failure("QueueAlreadyExists"),
            failure("QueueAlreadyExists"),
            ok(),
            failure("QueueAlreadyExists"),
            failure("QueueAlreadyExists"),
        ],
    );
}

#[test]
fn queue_limit_is_one_thousand() {
    let mut lines: Vec<String> = (0..1000)
        .map(|n| format!(r#"{{"Command":"CreateQueue","QueueName":"q{n}"}}"#))
        .collect();
    lines.push(r#"{"Command":"CreateQueue","QueueName":"extra"}"#.to_owned());
    lines.push(r#"{"Command":"CreateQueue","QueueName":"q7"}"#.to_owned());
    lines.push(r#"{"Command":"DeleteQueue","QueueName":"q7"}"#.to_owned());
    lines.push(r#"{"Command":"CreateQueue","QueueName":"extra"}"#.to_owned());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&lines);
    assert_eq!(responses.len(), 1004);
    assert!(responses[..1000].iter().all(|r| *r == ok()));
    assert_responses(
        responses[1000..].to_vec(),
        vec![failure("LimitExceeded"), ok(), ok(), ok()],
    );
}

#[test]
fn set_queue_attributes_follows_its_rules() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"missing","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":false}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":"no"}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"MaxReceiveCount":1}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"nope","MaxReceiveCount":1}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":1}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"f.fifo","MaxReceiveCount":1}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"VisibilityTimeout":7,"DelaySeconds":2,"RetentionSeconds":60}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{}}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        ]),
        vec![
            ok(),
            ok(),
            failure("QueueDoesNotExist"),
            failure("UnsupportedOperation"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("QueueDoesNotExist"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            ok(),
            failure("MissingParameter"),
            ok(),
            json!({"ok": true,
                "Attributes": {"VisibilityTimeout": 7, "DelaySeconds": 2, "RetentionSeconds": 60,
                               "Fifo": false, "ContentBasedDeduplication": false},
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
        ],
    );
}

#[test]
fn dead_letter_configuration_can_be_replaced_and_dies_with_its_target() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"d1"}"#,
        r#"{"Command":"CreateQueue","QueueName":"d2"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DeadLetterQueue":"d1","MaxReceiveCount":2}}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"d2","MaxReceiveCount":5}}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"d1"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"d2"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        r#"{"Command":"DeleteQueue","QueueName":"d2"}"#,
    ]);
    assert_eq!(responses[4]["Attributes"]["DeadLetterQueue"], "d2");
    assert_eq!(responses[4]["Attributes"]["MaxReceiveCount"], 5);
    assert_eq!(responses[6]["Attributes"]["DeadLetterQueue"], "d2");
    let last_attributes = responses[8]["Attributes"].as_object().unwrap();
    assert!(!last_attributes.contains_key("DeadLetterQueue"));
    assert!(!last_attributes.contains_key("MaxReceiveCount"));
    assert_eq!(responses[9]["error"]["code"], "QueueDoesNotExist");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let names = ["b", "a.fifo", "Z", "_x", "-d", "9", "a", "z"];
    let mut lines: Vec<String> = names
        .iter()
        .map(|name| {
            let fifo = if name.ends_with(".fifo") {
                r#","Attributes":{"Fifo":true}"#
            } else {
                ""
            };
            format!(r#"{{"Command":"CreateQueue","QueueName":"{name}"{fifo}}}"#)
        })
        .collect();
    lines.push(r#"{"Command":"ListQueues"}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":"a"}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":""}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":"nothing"}"#.to_owned());
    lines.push(r#"{"Command":"ListQueues","Prefix":7}"#.to_owned());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let responses = session(&lines);
    let all = json!({"ok": true, "QueueNames": ["-d", "9", "Z", "_x", "a", "a.fifo", "b", "z"]});
    assert_eq!(responses[8], all);
    assert_eq!(
        responses[9],
        json!({"ok": true, "QueueNames": ["a", "a.fifo"]})
    );
    assert_eq!(responses[10], all);
    assert_eq!(responses[11], json!({"ok": true, "QueueNames": []}));
    assert_eq!(responses[12]["error"]["code"], "InvalidParameterValue");
}

#[test]
fn unknown_queue_names_fail_lookup_whatever_they_look_like() {
    let commands = [
        r#"{"Command":"GetQueueAttributes","QueueName":"not a name"}"#,
        r#"{"Command":"DeleteQueue","QueueName":""}"#,
        r#"{"Command":"PurgeQueue","QueueName":"q"}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"garbage"}"#,
        r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":1}"#,
    ];
    assert_responses(
        session(&commands),
        vec![failure("QueueDoesNotExist"); commands.len()],
    );
}

#[test]
fn send_and_receive_round_trip_on_a_standard_queue() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"one"}"#,
            r#"{"Command":"Tick","Seconds":4}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"two","DelaySeconds":10}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
            r#"{"Command":"Tick","Seconds":10}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
            r#"{"Command":"Tick","Seconds":30}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Now": 4}),
            json!({"ok": true, "MessageId": "m-2"}),
            json!({"ok": true,
                "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                               "Fifo": false, "ContentBasedDeduplication": false},
                "Counts": {"Visible": 1, "InFlight": 0, "Delayed": 1}}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 4}]}),
            json!({"ok": true, "Now": 14}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "two", "ReceiveCount": 1,
                 "SentAt": 4, "FirstReceivedAt": 14}]}),
            json!({"ok": true, "Now": 44}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "one", "ReceiveCount": 2,
                 "SentAt": 0, "FirstReceivedAt": 4},
                {"MessageId": "m-2", "ReceiptHandle": "r-4", "Body": "two", "ReceiveCount": 2,
                 "SentAt": 4, "FirstReceivedAt": 14}]}),
        ],
    );
}

#[test]
fn queue_delay_applies_at_send_time_and_message_delay_overrides_it() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":20}}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"queue delay"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"no delay","DelaySeconds":0}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DelaySeconds":0}}"#,
            r#"{"Command":"Tick","Seconds":19}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
            r#"{"Command":"Tick","Seconds":1}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "MessageId": "m-2"}),
            ok(),
            json!({"ok": true, "Now": 19}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-2", "ReceiptHandle": "r-1", "Body": "no delay", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 19}]}),
            json!({"ok": true, "Now": 20}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "queue delay", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 20}]}),
        ],
    );
}

#[test]
fn send_message_validates_in_the_documented_order() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SendMessage","Body":""}"#,
            r#"{"Command":"SendMessage","QueueName":"nope","Body":""}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":""}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DelaySeconds":1}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","DeduplicationId":"d"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"x","GroupId":"g","DeduplicationId":"d"}"#,
        ]),
        vec![
            ok(),
            ok(),
            failure("MissingParameter"),
            failure("InvalidParameterValue"),
            failure("InvalidParameterValue"),
            failure("UnsupportedOperation"),
            failure("UnsupportedOperation"),
            failure("UnsupportedOperation"),
            failure("MissingParameter"),
            failure("MissingParameter"),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ],
    );
}

#[test]
fn failed_and_duplicate_sends_do_not_use_up_ids() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d1"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b","GroupId":"h","DeduplicationId":"d1"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"","GroupId":"g","DeduplicationId":"d2"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"c","GroupId":"g","DeduplicationId":"d2"}"#,
            r#"{"Command":"Tick","Seconds":299}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"d","GroupId":"g","DeduplicationId":"d1"}"#,
            r#"{"Command":"Tick","Seconds":1}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"e","GroupId":"g","DeduplicationId":"d1"}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            failure("InvalidParameterValue"),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "Now": 299}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Now": 300}),
            json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3}),
        ],
    );
}

#[test]
fn deduplication_ids_outlive_deleted_and_purged_messages() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"same","GroupId":"g"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"same","GroupId":"g"}"#,
            r#"{"Command":"PurgeQueue","QueueName":"f.fifo"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"same","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"other","GroupId":"g"}"#,
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"explicit","GroupId":"g","DeduplicationId":"same"}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "same", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "g", "SequenceNumber": 1}]}),
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            ok(),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
            json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2}),
            json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1}),
        ],
    );
}

#[test]
fn fifo_receive_yields_one_message_per_group_and_blocks_busy_groups() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a1","GroupId":"a","DeduplicationId":"1"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a2","GroupId":"a","DeduplicationId":"2"}"#,
        r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b1","GroupId":"b","DeduplicationId":"3"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
        r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
    ]);
    let bodies = |response: &Value| -> Vec<String> {
        response["Messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["Body"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(bodies(&responses[4]), ["a1", "b1"]);
    assert_eq!(bodies(&responses[5]), Vec::<String>::new());
    assert_eq!(responses[6], ok());
    assert_eq!(bodies(&responses[7]), ["a2"]);
}

#[test]
fn visibility_timeout_zero_makes_the_message_visible_at_once_and_the_handle_unusable() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":0}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":9}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            failure("ReceiptHandleExpired"),
            failure("ReceiptHandleExpired"),
            json!({"ok": true,
                "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                               "Fifo": false, "ContentBasedDeduplication": false},
                "Counts": {"Visible": 1, "InFlight": 0, "Delayed": 0}}),
        ],
    );
}

#[test]
fn receipt_handles_are_exact_strings_and_expire_when_superseded() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"CreateQueue","QueueName":"other"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":5}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-01"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"R-1"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1 "}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":7}"#,
            r#"{"Command":"DeleteMessage","QueueName":"other","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"Tick","Seconds":5}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
        ]),
        vec![
            ok(),
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            failure("ReceiptHandleInvalid"),
            failure("ReceiptHandleInvalid"),
            failure("ReceiptHandleInvalid"),
            failure("ReceiptHandleInvalid"),
            failure("InvalidParameterValue"),
            failure("ReceiptHandleExpired"),
            json!({"ok": true, "Now": 5}),
            failure("ReceiptHandleExpired"),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "x", "ReceiveCount": 2,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            failure("ReceiptHandleExpired"),
            ok(),
            failure("ReceiptHandleExpired"),
        ],
    );
}

#[test]
fn change_message_visibility_replaces_the_deadline() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":10}"#,
            r#"{"Command":"Tick","Seconds":8}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":10}"#,
            r#"{"Command":"Tick","Seconds":9}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"Tick","Seconds":1}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":0}"#,
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-1","VisibilityTimeout":43201}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            json!({"ok": true, "Now": 8}),
            ok(),
            json!({"ok": true, "Now": 17}),
            json!({"ok": true, "Messages": []}),
            json!({"ok": true, "Now": 18}),
            failure("ReceiptHandleExpired"),
            failure("InvalidParameterValue"),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "x", "ReceiveCount": 2,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
        ],
    );
}

#[test]
fn retention_removes_messages_at_the_deadline_and_never_restores_them() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"RetentionSeconds":100}}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"Tick","Seconds":99}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":60}}"#,
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"RetentionSeconds":1000}}"#,
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            json!({"ok": true, "Now": 99}),
            ok(),
            ok(),
            json!({"ok": true,
                "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 1000,
                               "Fifo": false, "ContentBasedDeduplication": false},
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
            failure("ReceiptHandleExpired"),
        ],
    );
}

#[test]
fn purge_removes_every_state_and_keeps_ids_counting() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"b","DelaySeconds":5}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"PurgeQueue","QueueName":"q"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"c"}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "MessageId": "m-2"}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            ok(),
            failure("ReceiptHandleExpired"),
            json!({"ok": true, "MessageId": "m-3"}),
        ],
    );
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"DeleteQueue","QueueName":"q"}"#,
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        ]),
        vec![
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            ok(),
            ok(),
            failure("ReceiptHandleExpired"),
            json!({"ok": true, "Messages": []}),
        ],
    );
}

#[test]
fn messages_move_to_the_dead_letter_queue_after_max_receive_count() {
    assert_responses(
        session(&[
            r#"{"Command":"CreateQueue","QueueName":"dead","Attributes":{"VisibilityTimeout":0}}"#,
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dead","MaxReceiveCount":2}}"#,
            r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
            r#"{"Command":"ReceiveMessage","QueueName":"dead"}"#,
        ]),
        vec![
            ok(),
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "x", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-2", "Body": "x", "ReceiveCount": 2,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
            json!({"ok": true, "Messages": []}),
            failure("ReceiptHandleExpired"),
            json!({"ok": true, "Messages": [
                {"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "x", "ReceiveCount": 1,
                 "SentAt": 0, "FirstReceivedAt": 0}]}),
        ],
    );
}

#[test]
fn fifo_dead_letter_moves_take_new_sequence_numbers_and_unblock_the_group() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dead.fifo","Attributes":{"Fifo":true}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q.fifo","Attributes":{"Fifo":true,"VisibilityTimeout":0,"DeadLetterQueue":"dead.fifo","MaxReceiveCount":1}}"#,
        r#"{"Command":"SendMessage","QueueName":"dead.fifo","Body":"first","GroupId":"x","DeduplicationId":"d0"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a1","GroupId":"a","DeduplicationId":"1"}"#,
        r#"{"Command":"SendMessage","QueueName":"q.fifo","Body":"a2","GroupId":"a","DeduplicationId":"2"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q.fifo"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q.fifo"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"dead.fifo","MaxMessages":10}"#,
    ]);
    assert_eq!(
        responses[6],
        json!({"ok": true, "Messages": [
            {"MessageId": "m-3", "ReceiptHandle": "r-2", "Body": "a2", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 2}]})
    );
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [
            {"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "first", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "x", "SequenceNumber": 1},
            {"MessageId": "m-2", "ReceiptHandle": "r-4", "Body": "a1", "ReceiveCount": 1,
             "SentAt": 0, "FirstReceivedAt": 0, "GroupId": "a", "SequenceNumber": 2}]})
    );
}

#[test]
fn dead_lettered_messages_that_are_too_old_for_the_target_vanish() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dead","Attributes":{"RetentionSeconds":60}}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dead","MaxReceiveCount":1}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"Tick","Seconds":60}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"GetQueueAttributes","QueueName":"dead"}"#,
    ]);
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(
        responses[6]["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let responses = session(&[
        r#"{"Command":"CreateQueue","QueueName":"dead"}"#,
        r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"VisibilityTimeout":0}}"#,
        r#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"dead","MaxReceiveCount":2}}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
        r#"{"Command":"ReceiveMessage","QueueName":"dead"}"#,
    ]);
    assert_eq!(responses[6], json!({"ok": true, "Messages": []}));
    assert_eq!(responses[7]["Messages"][0]["ReceiveCount"], 1);
}

#[test]
fn a_queue_holds_at_most_one_hundred_thousand_messages_and_filling_it_is_fast() {
    let mut input = String::from("{\"Command\":\"CreateQueue\",\"QueueName\":\"q\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}\n");
    }
    input.push_str("{\"Command\":\"PurgeQueue\",\"QueueName\":\"q\"}\n");
    input.push_str("{\"Command\":\"SendMessage\",\"QueueName\":\"q\",\"Body\":\"x\"}\n");
    let responses = run_bytes(input.as_bytes());
    assert_eq!(responses.len(), 100_004);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(responses[100_001]["error"]["code"], "LimitExceeded");
    assert_eq!(responses[100_002], ok());
    assert_eq!(
        responses[100_003],
        json!({"ok": true, "MessageId": "m-100001"})
    );
}

#[test]
fn the_binary_serves_stdin_to_stdout() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"{\"Command\":\"Tick\",\"Seconds\":3}\nbroken")
        .expect("the child reads its stdin");
    let output = child
        .wait_with_output()
        .expect("the child exits at end of input");
    assert!(output.status.success());
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], json!({"ok": true, "Now": 3}));
    assert_eq!(lines[1]["error"]["code"], "InvalidRequest");
}
