mod common;

use common::{code, exchange, ok, run_raw};
use serde_json::json;

#[test]
fn a_message_goes_through_its_whole_life() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": "jobs"}),
        json!({"Command": "SendMessage", "QueueName": "jobs", "Body": "hello"}),
        json!({"Command": "Tick", "Seconds": 10}),
        json!({"Command": "ReceiveMessage", "QueueName": "jobs"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}),
        json!({"Command": "DeleteMessage", "QueueName": "jobs", "ReceiptHandle": "r-1"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "jobs"}),
    ]);
    assert_eq!(
        responses,
        [
            ok(),
            json!({"ok": true, "MessageId": "m-1"}),
            json!({"ok": true, "Now": 10}),
            json!({"ok": true, "Messages": [{
                "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "hello",
                "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 10,
            }]}),
            json!({"ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                    "Fifo": false, "ContentBasedDeduplication": false,
                },
                "Counts": {"Visible": 0, "InFlight": 1, "Delayed": 0}}),
            ok(),
            json!({"ok": true,
                "Attributes": {
                    "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                    "Fifo": false, "ContentBasedDeduplication": false,
                },
                "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}}),
        ]
    );
}

#[test]
fn every_line_gets_exactly_one_response_and_bad_lines_do_not_stop_the_stream() {
    let input = b"\n[]\n{}\n{\"Command\": 5}\n{\"Command\": null}\n{\"Command\": \"tick\", \"Seconds\": 1}\n\xff\xfe\n{\"Command\": \"Tick\", \"Seconds\": 1, \"Extra\": [1]}\n{\"Command\": \"Tick\", \"Seconds\": 2}";
    let responses = run_raw(input);
    let codes: Vec<_> = responses.iter().map(|r| code(r).clone()).collect();
    assert_eq!(
        codes,
        [
            json!("InvalidRequest"),
            json!("InvalidRequest"),
            json!("InvalidRequest"),
            json!("InvalidRequest"),
            json!("InvalidRequest"),
            json!("InvalidRequest"),
            json!("InvalidRequest"),
            json!(null),
            json!(null),
        ]
    );
    assert_eq!(responses[7], json!({"ok": true, "Now": 1}));
    assert_eq!(responses[8], json!({"ok": true, "Now": 3}));
}

#[test]
fn an_empty_input_and_a_lone_final_newline_are_handled() {
    assert_eq!(run_raw(b"").len(), 0);
    assert_eq!(code(&run_raw(b"\n")[0]), "InvalidRequest");
}

#[test]
fn integers_must_be_written_as_plain_integers() {
    let bad_numbers = [
        "30.0",
        "3e1",
        "3E1",
        "\"30\"",
        "true",
        "[30]",
        "1e999",
        "-1",
        "1209601",
        "99999999999999999999",
    ];
    let input: String = bad_numbers
        .iter()
        .map(|n| format!("{{\"Command\": \"Tick\", \"Seconds\": {n}}}\n"))
        .collect();
    let codes: Vec<_> = run_raw(input.as_bytes())
        .iter()
        .map(|r| code(r).clone())
        .collect();
    assert_eq!(
        codes,
        vec![json!("InvalidParameterValue"); bad_numbers.len()]
    );
}

#[test]
fn tick_accumulates_and_validates() {
    let responses = exchange(&[
        json!({"Command": "Tick", "Seconds": 5}),
        json!({"Command": "Tick", "Seconds": 0}),
        json!({"Command": "Tick", "Seconds": 1209600}),
        json!({"Command": "Tick"}),
        json!({"Command": "Tick", "Seconds": null}),
        json!({"Command": "Tick", "Seconds": 1209601}),
    ]);
    assert_eq!(responses[0], json!({"ok": true, "Now": 5}));
    assert_eq!(responses[1], json!({"ok": true, "Now": 5}));
    assert_eq!(responses[2], json!({"ok": true, "Now": 1209605}));
    assert_eq!(code(&responses[3]), "MissingParameter");
    assert_eq!(code(&responses[4]), "MissingParameter");
    assert_eq!(code(&responses[5]), "InvalidParameterValue");
}

#[test]
fn the_first_failing_check_decides_the_error_code() {
    let cases = [
        (json!({"Command": "CreateQueue"}), "MissingParameter"),
        (
            json!({"Command": "CreateQueue", "QueueName": 5}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 3}),
            "QueueNameInvalid",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Bogus": 1}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 43201}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"RetentionSeconds": 59}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": "yes"}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q.fifo"}),
            "QueueNameInvalid",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"Fifo": true}}),
            "QueueNameInvalid",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q",
                   "Attributes": {"ContentBasedDeduplication": true}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"MaxReceiveCount": 3}}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "CreateQueue", "QueueName": "q",
                   "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 3}}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": 5}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "bad name!", "Body": ""}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SendMessage", "QueueName": "bad name!", "Body": "x"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "DeleteMessage", "QueueName": "nope", "ReceiptHandle": "junk"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "nope"}),
            "MissingParameter",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": []}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "SetQueueAttributes", "QueueName": "nope", "Attributes": {}}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "ListQueues", "Prefix": 3}),
            "InvalidParameterValue",
        ),
        (
            json!({"Command": "PurgeQueue", "QueueName": "nope"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "DeleteQueue", "QueueName": "nope"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "GetQueueAttributes", "QueueName": "nope"}),
            "QueueDoesNotExist",
        ),
        (
            json!({"Command": "ReceiveMessage", "QueueName": "nope", "MaxMessages": 11}),
            "InvalidParameterValue",
        ),
    ];
    for (command, expected) in cases {
        let responses = exchange(std::slice::from_ref(&command));
        assert_eq!(code(&responses[0]), expected, "{command}");
    }
}

#[test]
fn null_parameters_and_attributes_count_as_absent() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": "q",
               "Attributes": {"VisibilityTimeout": null, "Bogus": null}}),
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": null}),
        json!({"Command": "ListQueues", "Prefix": null}),
    ]);
    assert_eq!(
        responses,
        [ok(), ok(), json!({"ok": true, "QueueNames": ["q"]})]
    );
}

#[test]
fn queue_names_are_validated_only_when_creating() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": ""}),
        json!({"Command": "CreateQueue", "QueueName": "a b"}),
        json!({"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true}}),
        json!({"Command": "CreateQueue", "QueueName": "a.fifo.fifo", "Attributes": {"Fifo": true}}),
        json!({"Command": "GetQueueAttributes", "QueueName": ""}),
    ]);
    let codes: Vec<_> = responses.iter().map(|r| code(r).clone()).collect();
    assert_eq!(
        codes,
        [
            json!("QueueNameInvalid"),
            json!("QueueNameInvalid"),
            json!(null),
            json!("QueueNameInvalid"),
            json!("QueueDoesNotExist"),
        ]
    );
}

#[test]
fn creating_an_existing_queue_succeeds_only_with_equal_attributes() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 40}}),
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 40}}),
        json!({"Command": "CreateQueue", "QueueName": "q"}),
        json!({"Command": "CreateQueue", "QueueName": "d"}),
        json!({"Command": "CreateQueue", "QueueName": "p",
               "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 2}}),
        json!({"Command": "CreateQueue", "QueueName": "p",
               "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 2, "DelaySeconds": 0}}),
        json!({"Command": "CreateQueue", "QueueName": "p",
               "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 3}}),
        json!({"Command": "CreateQueue", "QueueName": "p"}),
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"VisibilityTimeout": 40, "DeadLetterQueue": "zz", "MaxReceiveCount": 1}}),
    ]);
    let codes: Vec<_> = responses.iter().map(|r| code(r).clone()).collect();
    assert_eq!(
        codes,
        [
            json!(null),
            json!(null),
            json!("QueueAlreadyExists"),
            json!(null),
            json!(null),
            json!(null),
            json!("QueueAlreadyExists"),
            json!("QueueAlreadyExists"),
            json!("QueueAlreadyExists"),
        ]
    );
}

#[test]
fn a_dead_letter_queue_must_exist_and_have_the_same_type() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": "d.fifo", "Attributes": {"Fifo": true}}),
        json!({"Command": "CreateQueue", "QueueName": "p",
               "Attributes": {"DeadLetterQueue": "d.fifo", "MaxReceiveCount": 2}}),
        json!({"Command": "CreateQueue", "QueueName": "p"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "p",
               "Attributes": {"DeadLetterQueue": "d.fifo", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "p",
               "Attributes": {"DeadLetterQueue": "p", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "p",
               "Attributes": {"DeadLetterQueue": "ghost", "MaxReceiveCount": 2}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "p",
               "Attributes": {"MaxReceiveCount": 2}}),
    ]);
    let codes: Vec<_> = responses.iter().map(|r| code(r).clone()).collect();
    assert_eq!(
        codes,
        [
            json!(null),
            json!("InvalidParameterValue"),
            json!(null),
            json!("InvalidParameterValue"),
            json!("InvalidParameterValue"),
            json!("QueueDoesNotExist"),
            json!("InvalidParameterValue"),
        ]
    );
}

#[test]
fn set_queue_attributes_changes_only_the_attributes_given() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": "d"}),
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 7}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"VisibilityTimeout": 5, "DeadLetterQueue": "d", "MaxReceiveCount": 9}}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"Fifo": false}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"ContentBasedDeduplication": true}}),
        json!({"Command": "SetQueueAttributes", "QueueName": "q",
               "Attributes": {"ContentBasedDeduplication": false, "Bogus": 1}}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
    ]);
    let expected = json!({"ok": true,
        "Attributes": {
            "VisibilityTimeout": 5, "DelaySeconds": 7, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
            "DeadLetterQueue": "d", "MaxReceiveCount": 9,
        },
        "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}});
    assert_eq!(responses[3], expected);
    assert_eq!(responses[4], ok());
    assert_eq!(code(&responses[5]), "UnsupportedOperation");
    assert_eq!(code(&responses[6]), "InvalidParameterValue");
    assert_eq!(code(&responses[7]), "InvalidParameterValue");
    assert_eq!(responses[8], expected);
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_configuration_that_used_it() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": "d"}),
        json!({"Command": "CreateQueue", "QueueName": "q",
               "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 4}}),
        json!({"Command": "DeleteQueue", "QueueName": "d"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
        json!({"Command": "CreateQueue", "QueueName": "d"}),
        json!({"Command": "GetQueueAttributes", "QueueName": "q"}),
    ]);
    let expected = json!({"ok": true,
        "Attributes": {
            "VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
            "Fifo": false, "ContentBasedDeduplication": false,
        },
        "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}});
    assert_eq!(responses[3], expected);
    assert_eq!(responses[5], expected);
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut commands: Vec<_> = ["b", "a", "A", "_", "-", "9"]
        .iter()
        .map(|name| json!({"Command": "CreateQueue", "QueueName": name}))
        .collect();
    commands.push(
        json!({"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true}}),
    );
    commands.push(json!({"Command": "ListQueues"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "a"}));
    commands.push(json!({"Command": "ListQueues", "Prefix": ""}));
    commands.push(json!({"Command": "ListQueues", "Prefix": "zz"}));
    let responses = exchange(&commands);
    assert_eq!(
        responses[7],
        json!({"ok": true, "QueueNames": ["-", "9", "A", "_", "a", "a.fifo", "b"]})
    );
    assert_eq!(
        responses[8],
        json!({"ok": true, "QueueNames": ["a", "a.fifo"]})
    );
    assert_eq!(responses[9], responses[7]);
    assert_eq!(responses[10], json!({"ok": true, "QueueNames": []}));
}

#[test]
fn a_queue_is_a_new_queue_after_it_was_deleted() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": "q"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "old"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q"}),
        json!({"Command": "DeleteQueue", "QueueName": "q"}),
        json!({"Command": "CreateQueue", "QueueName": "q"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "new"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q"}),
    ]);
    assert_eq!(code(&responses[6]), "ReceiptHandleExpired");
    assert_eq!(
        responses[7],
        json!({"ok": true, "Messages": [{
            "MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "new",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0,
        }]})
    );
}

#[test]
fn queue_and_message_limits_are_enforced() {
    let mut commands: Vec<_> = (0..1001)
        .map(|n| json!({"Command": "CreateQueue", "QueueName": format!("q{n}")}))
        .collect();
    commands.push(json!({"Command": "CreateQueue", "QueueName": "q0"}));
    let responses = exchange(&commands);
    assert_eq!(code(&responses[1000]), "LimitExceeded");
    assert_eq!(responses[1001], ok());
    assert_eq!(responses[..1000], vec![ok(); 1000][..]);

    let mut input = String::from("{\"Command\": \"CreateQueue\", \"QueueName\": \"big\"}\n");
    for _ in 0..100_001 {
        input.push_str("{\"Command\": \"SendMessage\", \"QueueName\": \"big\", \"Body\": \"x\"}\n");
    }
    let responses = run_raw(input.as_bytes());
    assert_eq!(responses.len(), 100_002);
    assert_eq!(
        responses[100_000],
        json!({"ok": true, "MessageId": "m-100000"})
    );
    assert_eq!(code(&responses[100_001]), "LimitExceeded");
}

#[test]
fn purge_removes_every_message() {
    let responses = exchange(&[
        json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": {"DelaySeconds": 5}}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "a"}),
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "b", "DelaySeconds": 0}),
        json!({"Command": "ReceiveMessage", "QueueName": "q"}),
        json!({"Command": "PurgeQueue", "QueueName": "q"}),
        json!({"Command": "ReceiveMessage", "QueueName": "q"}),
        json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}),
    ]);
    assert_eq!(responses[4], ok());
    assert_eq!(responses[5], json!({"ok": true, "Messages": []}));
    assert_eq!(code(&responses[6]), "ReceiptHandleExpired");
}
