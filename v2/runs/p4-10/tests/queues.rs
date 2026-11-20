//! Queue commands of section 5: creating, listing, configuring, purging and
//! deleting queues, and the clock.

mod common;

use common::{fail, ok};
use miniq::Broker;
use serde_json::json;

#[test]
fn create_queue_succeeds_once_and_again_with_the_same_attributes() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 30}}"#,
    );
}

#[test]
fn create_queue_with_different_attributes_is_refused() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
    );
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DelaySeconds": 1}}"#,
        "QueueAlreadyExists",
    );
}

#[test]
fn create_queue_checks_the_name_before_anything_else() {
    let mut broker = Broker::new();
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a b", "Attributes": 5}"#,
        "QueueNameInvalid",
    );
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue"}"#,
        "MissingParameter",
    );
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": 7}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn fifo_suffix_and_fifo_attribute_must_agree() {
    let mut broker = Broker::new();
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a.fifo"}"#,
        "QueueNameInvalid",
    );
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"Fifo": true}}"#,
        "QueueNameInvalid",
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a.fifo", "Attributes": {"Fifo": true}}"#,
    );
}

#[test]
fn content_based_deduplication_needs_a_fifo_queue() {
    let mut broker = Broker::new();
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"ContentBasedDeduplication": true}}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn dead_letter_settings_come_together() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
    );
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dead"}}"#,
        "InvalidParameterValue",
    );
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"MaxReceiveCount": 2}}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn dead_letter_queue_must_exist_and_match_the_fifo_type() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead.fifo", "Attributes": {"Fifo": true}}"#,
    );
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "nope", "MaxReceiveCount": 2}}"#,
        "QueueDoesNotExist",
    );
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "a", "Attributes": {"DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 2}}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn queue_count_is_limited_to_1000() {
    let mut broker = Broker::new();
    for n in 0..1000 {
        ok(
            &mut broker,
            &format!(r#"{{"Command": "CreateQueue", "QueueName": "q{n}"}}"#),
        );
    }
    fail(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "one-too-many"}"#,
        "LimitExceeded",
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "q0"}"#,
    );
}

#[test]
fn list_queues_is_sorted_by_bytes_and_filters_by_prefix() {
    let mut broker = Broker::new();
    for name in ["b", "a_1", "a-2", "A", "a.fifo", "a10", "_"] {
        let fifo = name.ends_with(".fifo");
        ok(
            &mut broker,
            &format!(
                r#"{{"Command": "CreateQueue", "QueueName": "{name}", "Attributes": {{"Fifo": {fifo}}}}}"#
            ),
        );
    }
    let all = ok(&mut broker, r#"{"Command": "ListQueues"}"#);
    assert_eq!(
        all["QueueNames"],
        json!(["A", "_", "a-2", "a.fifo", "a10", "a_1", "b"])
    );

    let prefixed = ok(&mut broker, r#"{"Command": "ListQueues", "Prefix": "a"}"#);
    assert_eq!(
        prefixed["QueueNames"],
        json!(["a-2", "a.fifo", "a10", "a_1"])
    );

    let none = ok(&mut broker, r#"{"Command": "ListQueues", "Prefix": "zz"}"#);
    assert_eq!(none["QueueNames"], json!([]));
}

#[test]
fn get_queue_attributes_reports_defaults_and_counts() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
    );
    let response = ok(
        &mut broker,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    );
    assert_eq!(
        response["Attributes"],
        json!({
            "VisibilityTimeout": 30,
            "DelaySeconds": 0,
            "RetentionSeconds": 345600,
            "Fifo": false,
            "ContentBasedDeduplication": false
        })
    );
    assert_eq!(
        response["Counts"],
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    fail(
        &mut broker,
        r#"{"Command": "GetQueueAttributes", "QueueName": "nope"}"#,
        "QueueDoesNotExist",
    );
}

#[test]
fn set_queue_attributes_changes_only_what_it_is_given() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DelaySeconds": 4}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"VisibilityTimeout": 9}}"#,
    );
    let response = ok(
        &mut broker,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    );
    assert_eq!(response["Attributes"]["VisibilityTimeout"], 9);
    assert_eq!(response["Attributes"]["DelaySeconds"], 4);
}

#[test]
fn set_queue_attributes_refuses_to_change_the_queue_type() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
    );
    fail(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"Fifo": false}}"#,
        "UnsupportedOperation",
    );
}

#[test]
fn set_queue_attributes_needs_its_parameters_and_an_existing_queue() {
    let mut broker = Broker::new();
    fail(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs"}"#,
        "MissingParameter",
    );
    fail(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {}}"#,
        "QueueDoesNotExist",
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
    );
    fail(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": []}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn a_dead_letter_configuration_can_be_replaced_but_not_removed() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead2"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dead2", "MaxReceiveCount": 5}}"#,
    );
    let response = ok(
        &mut broker,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    );
    assert_eq!(response["Attributes"]["DeadLetterQueue"], "dead2");
    assert_eq!(response["Attributes"]["MaxReceiveCount"], 5);

    fail(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"MaxReceiveCount": 3}}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn a_queue_cannot_be_its_own_dead_letter_queue() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
    );
    fail(
        &mut broker,
        r#"{"Command": "SetQueueAttributes", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "jobs", "MaxReceiveCount": 2}}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn delete_queue_removes_it_and_clears_dead_letter_references() {
    let mut broker = Broker::new();
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs", "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 2}}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#,
    );

    let response = ok(
        &mut broker,
        r#"{"Command": "GetQueueAttributes", "QueueName": "jobs"}"#,
    );
    assert!(response["Attributes"].get("DeadLetterQueue").is_none());
    assert!(response["Attributes"].get("MaxReceiveCount").is_none());

    fail(
        &mut broker,
        r#"{"Command": "DeleteQueue", "QueueName": "dead"}"#,
        "QueueDoesNotExist",
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "dead"}"#,
    );
}

#[test]
fn purge_queue_needs_an_existing_queue() {
    let mut broker = Broker::new();
    fail(
        &mut broker,
        r#"{"Command": "PurgeQueue", "QueueName": "jobs"}"#,
        "QueueDoesNotExist",
    );
    ok(
        &mut broker,
        r#"{"Command": "CreateQueue", "QueueName": "jobs"}"#,
    );
    ok(
        &mut broker,
        r#"{"Command": "PurgeQueue", "QueueName": "jobs"}"#,
    );
}

#[test]
fn tick_advances_the_clock_and_reports_it() {
    let mut broker = Broker::new();
    assert_eq!(
        ok(&mut broker, r#"{"Command": "Tick", "Seconds": 5}"#)["Now"],
        5
    );
    assert_eq!(
        ok(&mut broker, r#"{"Command": "Tick", "Seconds": 0}"#)["Now"],
        5
    );
    assert_eq!(
        ok(&mut broker, r#"{"Command": "Tick", "Seconds": 1209600}"#)["Now"],
        1_209_605
    );
    fail(&mut broker, r#"{"Command": "Tick"}"#, "MissingParameter");
    fail(
        &mut broker,
        r#"{"Command": "Tick", "Seconds": 1209601}"#,
        "InvalidParameterValue",
    );
    fail(
        &mut broker,
        r#"{"Command": "Tick", "Seconds": 1.5}"#,
        "InvalidParameterValue",
    );
}

#[test]
fn unknown_commands_and_non_json_lines_are_invalid_requests() {
    let mut broker = Broker::new();
    fail(
        &mut broker,
        r#"{"Command": "tick", "Seconds": 1}"#,
        "InvalidRequest",
    );
    fail(&mut broker, "", "InvalidRequest");
    fail(&mut broker, "not json", "InvalidRequest");
}
