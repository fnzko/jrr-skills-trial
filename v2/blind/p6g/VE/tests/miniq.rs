use std::io::Write;
use std::process::{Command, Stdio};

use miniq::Broker;
use serde_json::{Value, json};

#[derive(Default)]
pub struct Session(Broker);

impl Session {
    pub fn raw(&mut self, line: &[u8]) -> Value {
        let response = self.0.handle_line(line);
        serde_json::from_str(&response).expect("responses are JSON")
    }

    pub fn run(&mut self, command: Value) -> Value {
        self.raw(command.to_string().as_bytes())
    }

    /// Runs a command that must succeed and returns its response.
    pub fn ok(&mut self, command: Value) -> Value {
        let response = self.run(command.clone());
        assert_eq!(response["ok"], true, "{command} failed: {response}");
        response
    }

    pub fn create(&mut self, name: &str, attributes: Value) {
        let response =
            self.ok(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
        assert_eq!(response, json!({"ok": true}));
    }

    pub fn tick(&mut self, seconds: u64) {
        self.ok(json!({"Command": "Tick", "Seconds": seconds}));
    }

    pub fn send(&mut self, queue: &str, body: &str) -> Value {
        self.ok(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    pub fn send_fifo(&mut self, queue: &str, body: &str, group: &str, dedup: &str) -> Value {
        self.ok(
            json!({"Command": "SendMessage", "QueueName": queue, "Body": body,
            "GroupId": group, "DeduplicationId": dedup}),
        )
    }

    pub fn receive(&mut self, queue: &str, max: u64) -> Vec<Value> {
        let response =
            self.ok(json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max}));
        response["Messages"]
            .as_array()
            .expect("Messages is an array")
            .clone()
    }

    /// Bodies of the messages a `ReceiveMessage` delivers.
    pub fn receive_bodies(&mut self, queue: &str, max: u64) -> Vec<String> {
        self.receive(queue, max)
            .iter()
            .map(|m| m["Body"].as_str().expect("Body is a string").to_owned())
            .collect()
    }

    pub fn counts(&mut self, queue: &str) -> Value {
        self.ok(json!({"Command": "GetQueueAttributes", "QueueName": queue}))["Counts"].clone()
    }
}

pub fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], false, "expected a failure: {response}");
    response["error"]["code"]
        .as_str()
        .expect("error code is a string")
}

#[test]
fn bad_lines_get_invalid_request_and_the_next_line_still_runs() {
    let mut s = Session::default();
    let lines: [&[u8]; 9] = [
        b"",
        b"   ",
        b"not json",
        b"[1]",
        b"\"Tick\"",
        b"{}",
        br#"{"Command": 5}"#,
        br#"{"Command": null}"#,
        br#"{"Command": "tick", "Seconds": 1}"#,
    ];
    for line in lines {
        assert_eq!(error_code(&s.raw(line)), "InvalidRequest", "{line:?}");
    }
    assert_eq!(
        error_code(&s.raw(b"{\"Command\": \"Tick\", \"Seconds\": 1}\xff")),
        "InvalidRequest"
    );
    assert_eq!(error_code(&s.raw(b"\xff")), "InvalidRequest");
    assert_eq!(
        error_code(&s.raw(br#"{"Command": "Tick", "Seconds": 1} x"#)),
        "InvalidRequest"
    );
    assert_eq!(
        s.run(json!({"Command": "Tick", "Seconds": 5})),
        json!({"ok": true, "Now": 5})
    );
}

#[test]
fn unknown_members_are_ignored_and_null_means_absent() {
    let mut s = Session::default();
    let response = s.run(json!({"Command": "Tick", "Seconds": 2, "Extra": [1, {"a": null}]}));
    assert_eq!(response, json!({"ok": true, "Now": 2}));
    assert_eq!(
        error_code(&s.run(json!({"Command": "Tick", "Seconds": null}))),
        "MissingParameter"
    );
    s.create(
        "q",
        json!({"VisibilityTimeout": null, "DeadLetterQueue": null}),
    );
    let attributes = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attributes["Attributes"]["VisibilityTimeout"], 30);
}

#[test]
fn integers_must_be_written_as_integers() {
    let mut s = Session::default();
    for seconds in ["30.0", "3e1", "\"30\"", "true", "-1", "1209601"] {
        let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
        assert_eq!(
            error_code(&s.raw(line.as_bytes())),
            "InvalidParameterValue",
            "{seconds}"
        );
    }
    assert_eq!(
        s.run(json!({"Command": "Tick", "Seconds": 1209600}))["Now"],
        1209600
    );
}

#[test]
fn missing_parameters_are_reported_before_invalid_values() {
    let mut s = Session::default();
    let response = s.run(json!({"Command": "SendMessage", "QueueName": 5, "DelaySeconds": "x"}));
    assert_eq!(error_code(&response), "MissingParameter");
    let response =
        s.run(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": null}));
    assert_eq!(error_code(&response), "MissingParameter");
    let response = s.run(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"}),
    );
    assert_eq!(error_code(&response), "MissingParameter");
}

#[test]
fn invalid_values_are_reported_before_the_queue_lookup() {
    let mut s = Session::default();
    let response =
        s.run(json!({"Command": "ReceiveMessage", "QueueName": "absent", "MaxMessages": 11}));
    assert_eq!(error_code(&response), "InvalidParameterValue");
    let response = s.run(json!({"Command": "ReceiveMessage", "QueueName": "absent"}));
    assert_eq!(error_code(&response), "QueueDoesNotExist");
    let response = s.run(json!({"Command": "ReceiveMessage", "QueueName": "bad name!"}));
    assert_eq!(error_code(&response), "QueueDoesNotExist");
}

#[test]
fn create_queue_checks_the_name_before_other_parameters() {
    let mut s = Session::default();
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "bad name",
        "Attributes": {"VisibilityTimeout": -1}}));
    assert_eq!(error_code(&response), "QueueNameInvalid");
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "ok", "Attributes": [1]}));
    assert_eq!(error_code(&response), "InvalidParameterValue");
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": 7}));
    assert_eq!(error_code(&response), "InvalidParameterValue");
    let response =
        s.run(json!({"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"Nope": null}}));
    assert_eq!(error_code(&response), "InvalidParameterValue");
}

#[test]
fn attribute_values_are_range_and_type_checked() {
    let mut s = Session::default();
    let bad = [
        json!({"VisibilityTimeout": 43201}),
        json!({"DelaySeconds": 901}),
        json!({"RetentionSeconds": 59}),
        json!({"RetentionSeconds": 1209601}),
        json!({"Fifo": "true"}),
        json!({"ContentBasedDeduplication": 1}),
        json!({"DeadLetterQueue": 3, "MaxReceiveCount": 1}),
        json!({"MaxReceiveCount": 0}),
        json!({"MaxReceiveCount": 1001}),
    ];
    for attributes in bad {
        let response =
            s.run(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": attributes}));
        assert_eq!(
            error_code(&response),
            "InvalidParameterValue",
            "{attributes}"
        );
    }
}

fn run_binary(input: &[u8]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miniq"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the miniq binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input)
        .expect("input is written");
    let output = child.wait_with_output().expect("the binary exits");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("stdout is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each output line is JSON"))
        .collect()
}

#[test]
fn binary_answers_every_line_including_blank_ones_and_an_unterminated_last_line() {
    let responses = run_binary(
        b"{\"Command\":\"Tick\",\"Seconds\":3}\n\n\xff\n{\"Command\":\"Tick\",\"Seconds\":4}",
    );
    let codes: Vec<_> = responses.iter().map(|r| r["ok"].clone()).collect();
    assert_eq!(
        codes,
        [json!(true), json!(false), json!(false), json!(true)]
    );
    assert_eq!(responses[3]["Now"], 7);
}

#[test]
fn binary_with_no_input_prints_nothing() {
    assert!(run_binary(b"").is_empty());
}

#[test]
fn binary_treats_a_lone_newline_as_one_blank_command() {
    let responses = run_binary(b"\n");
    assert_eq!(responses.len(), 1);
    assert_eq!(error_code(&responses[0]), "InvalidRequest");
}

fn fifo_attributes() -> Value {
    json!({"Fifo": true})
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let mut s = Session::default();
    s.create("q", json!({"VisibilityTimeout": 10}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 10, "DelaySeconds": 0, "Fifo": false}),
    );
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(error_code(&response), "QueueAlreadyExists");
}

#[test]
fn create_queue_rules_apply_in_spec_order() {
    let mut s = Session::default();
    let cases = [
        // (name, attributes, expected code)
        ("a.fifo", json!({}), "QueueNameInvalid"),
        ("a", fifo_attributes(), "QueueNameInvalid"),
        (
            "a",
            json!({"ContentBasedDeduplication": true}),
            "InvalidParameterValue",
        ),
        (
            "a",
            json!({"DeadLetterQueue": "d"}),
            "InvalidParameterValue",
        ),
        ("a", json!({"MaxReceiveCount": 2}), "InvalidParameterValue"),
        (
            "a",
            json!({"DeadLetterQueue": "d", "MaxReceiveCount": 2}),
            "QueueDoesNotExist",
        ),
    ];
    for (name, attributes, code) in cases {
        let response =
            s.run(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
        assert_eq!(error_code(&response), code, "{name} {attributes}");
    }

    s.create("d.fifo", fifo_attributes());
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "a",
        "Attributes": {"DeadLetterQueue": "d.fifo", "MaxReceiveCount": 2}}));
    assert_eq!(error_code(&response), "InvalidParameterValue");
}

#[test]
fn existing_queue_is_reported_before_a_missing_dead_letter_queue() {
    let mut s = Session::default();
    s.create("a", json!({}));
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "a",
        "Attributes": {"DeadLetterQueue": "gone", "MaxReceiveCount": 2}}));
    assert_eq!(error_code(&response), "QueueAlreadyExists");
}

#[test]
fn at_most_1000_queues_exist() {
    let mut s = Session::default();
    for n in 0..1000 {
        s.create(&format!("q{n}"), json!({}));
    }
    s.create("q0", json!({}));
    let response = s.run(json!({"Command": "CreateQueue", "QueueName": "extra"}));
    assert_eq!(error_code(&response), "LimitExceeded");
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q5"}));
    s.create("extra", json!({}));
}

#[test]
fn get_queue_attributes_lists_defaults_and_omits_unset_dead_letter_members() {
    let mut s = Session::default();
    s.create("d", json!({}));
    s.create("q", json!({}));
    let response = s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        response,
        json!({
            "ok": true,
            "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                           "Fifo": false, "ContentBasedDeduplication": false},
            "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0},
        })
    );

    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "d", "MaxReceiveCount": 3, "DelaySeconds": 5}}));
    let attributes =
        &s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"];
    assert_eq!(attributes["DeadLetterQueue"], "d");
    assert_eq!(attributes["MaxReceiveCount"], 3);
    assert_eq!(attributes["DelaySeconds"], 5);
    assert_eq!(attributes["VisibilityTimeout"], 30);
}

#[test]
fn set_queue_attributes_rules_apply_in_spec_order() {
    let mut s = Session::default();
    s.create("q", json!({}));
    s.create("f.fifo", fifo_attributes());
    let set = |s: &mut Session, queue: &str, attributes: Value| {
        s.run(
            json!({"Command": "SetQueueAttributes", "QueueName": queue, "Attributes": attributes}),
        )
    };
    assert_eq!(
        error_code(&set(&mut s, "nope", json!({}))),
        "QueueDoesNotExist"
    );
    assert_eq!(
        error_code(&set(&mut s, "q", json!({"Fifo": false}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        error_code(&set(
            &mut s,
            "q",
            json!({"Fifo": true, "ContentBasedDeduplication": true})
        )),
        "UnsupportedOperation"
    );
    assert_eq!(
        error_code(&set(
            &mut s,
            "q",
            json!({"ContentBasedDeduplication": true})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        error_code(&set(&mut s, "q", json!({"MaxReceiveCount": 1}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        error_code(&set(
            &mut s,
            "q",
            json!({"DeadLetterQueue": "x", "MaxReceiveCount": 1})
        )),
        "QueueDoesNotExist"
    );
    assert_eq!(
        error_code(&set(
            &mut s,
            "q",
            json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        error_code(&set(
            &mut s,
            "q",
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        set(&mut s, "f.fifo", json!({"ContentBasedDeduplication": true})),
        json!({"ok": true})
    );
    assert_eq!(set(&mut s, "q", json!({})), json!({"ok": true}));
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut s = Session::default();
    for name in ["b", "a_", "a-", "a.fifo", "A", "a1", "ab", "a"] {
        let attributes = if name.ends_with(".fifo") {
            fifo_attributes()
        } else {
            json!({})
        };
        s.create(name, attributes);
    }
    let list = |s: &mut Session, command: Value| s.ok(command)["QueueNames"].clone();
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues"})),
        json!(["A", "a", "a-", "a.fifo", "a1", "a_", "ab", "b"])
    );
    assert_eq!(
        list(&mut s, json!({"Command": "ListQueues", "Prefix": "a"})),
        json!(["a", "a-", "a.fifo", "a1", "a_", "ab"])
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
        list(&mut s, json!({"Command": "ListQueues", "Prefix": ""}))
            .as_array()
            .map(Vec::len),
        Some(8)
    );
}

#[test]
fn deleting_a_queue_clears_dead_letter_configurations_that_name_it() {
    let mut s = Session::default();
    s.create("dead", json!({}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
    );
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "dead"}));
    let attributes =
        &s.ok(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Attributes"];
    assert_eq!(attributes.get("DeadLetterQueue"), None);
    assert_eq!(attributes.get("MaxReceiveCount"), None);

    s.create("dead", json!({}));
    s.send("q", "m");
    assert_eq!(s.receive_bodies("q", 1), ["m"]);
    s.tick(30);
    assert_eq!(
        s.receive_bodies("q", 1),
        ["m"],
        "no dead-letter queue to move it to"
    );
    assert_eq!(
        error_code(&s.run(json!({"Command": "DeleteQueue", "QueueName": "gone"}))),
        "QueueDoesNotExist"
    );
}

#[test]
fn standard_queue_delivers_in_arrival_order_with_counters() {
    let mut s = Session::default();
    s.create("q", json!({"VisibilityTimeout": 10}));
    assert_eq!(s.send("q", "one"), json!({"ok": true, "MessageId": "m-1"}));
    s.tick(4);
    assert_eq!(s.send("q", "two"), json!({"ok": true, "MessageId": "m-2"}));

    let first = s.receive("q", 10);
    assert_eq!(
        first,
        [
            json!({"MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "one", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 4}),
            json!({"MessageId": "m-2", "ReceiptHandle": "r-2", "Body": "two", "ReceiveCount": 1, "SentAt": 4, "FirstReceivedAt": 4}),
        ]
    );
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
    assert!(s.receive("q", 1).is_empty());

    s.tick(10);
    let again = s.receive("q", 1);
    assert_eq!(
        again,
        [
            json!({"MessageId": "m-1", "ReceiptHandle": "r-3", "Body": "one", "ReceiveCount": 2, "SentAt": 0, "FirstReceivedAt": 4}),
        ]
    );
}

#[test]
fn delayed_messages_become_visible_when_their_delay_ends() {
    let mut s = Session::default();
    s.create("q", json!({"DelaySeconds": 10}));
    s.send("q", "queue delay");
    s.ok(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own delay", "DelaySeconds": 2}),
    );
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "none", "DelaySeconds": 0}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 2})
    );
    assert_eq!(s.receive_bodies("q", 10), ["none"]);
    s.tick(2);
    assert_eq!(s.receive_bodies("q", 10), ["own delay"]);
    s.tick(8);
    assert_eq!(s.receive_bodies("q", 10), ["queue delay"]);
}

#[test]
fn visible_again_message_keeps_its_place_in_arrival_order() {
    let mut s = Session::default();
    s.create("q", json!({}));
    s.send("q", "a");
    s.send("q", "b");
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 5}));
    assert_eq!(s.receive_bodies("q", 1), ["b"]);
    s.tick(5);
    assert_eq!(s.receive_bodies("q", 2), ["a"], "b is still in flight");
    s.tick(30);
    assert_eq!(s.receive_bodies("q", 2), ["a", "b"]);
}

#[test]
fn zero_visibility_timeout_redelivers_at_once_and_the_handle_never_works() {
    let mut s = Session::default();
    s.create("q", json!({}));
    s.send("q", "a");
    let response =
        s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    assert_eq!(response["Messages"][0]["ReceiptHandle"], "r-1");
    let delete = json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"});
    assert_eq!(error_code(&s.run(delete)), "ReceiptHandleExpired");
    assert_eq!(
        s.receive("q", 5).len(),
        1,
        "once per call even with timeout 0"
    );
}

#[test]
fn receipt_handles_are_invalid_or_expired_by_the_spec_rules() {
    let mut s = Session::default();
    s.create("q", json!({"VisibilityTimeout": 10}));
    s.create("other", json!({}));
    s.send("q", "a");
    s.receive("q", 1);
    let delete = |s: &mut Session, queue: &str, handle: &str| {
        s.run(json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle}))
    };
    for handle in ["r-2", "r-01", "R-1", "r-1 ", "", "x", "r-0"] {
        assert_eq!(
            error_code(&delete(&mut s, "q", handle)),
            "ReceiptHandleInvalid",
            "{handle:?}"
        );
    }
    assert_eq!(
        error_code(&delete(&mut s, "other", "r-1")),
        "ReceiptHandleExpired"
    );
    assert_eq!(
        error_code(&delete(&mut s, "absent", "r-1")),
        "QueueDoesNotExist"
    );
    assert_eq!(delete(&mut s, "q", "r-1"), json!({"ok": true}));
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-1")),
        "ReceiptHandleExpired"
    );
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn only_the_latest_handle_of_an_in_flight_message_is_usable() {
    let mut s = Session::default();
    s.create("q", json!({"VisibilityTimeout": 10}));
    s.send("q", "a");
    s.receive("q", 1);
    s.tick(10);
    s.receive("q", 1);
    let delete = |s: &mut Session, handle: &str| {
        s.run(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle}))
    };
    assert_eq!(error_code(&delete(&mut s, "r-1")), "ReceiptHandleExpired");
    s.tick(10);
    assert_eq!(
        error_code(&delete(&mut s, "r-2")),
        "ReceiptHandleExpired",
        "timeout ended"
    );
}

#[test]
fn change_message_visibility_replaces_the_end_of_the_timeout() {
    let mut s = Session::default();
    s.create("q", json!({"VisibilityTimeout": 10}));
    s.send("q", "a");
    s.receive("q", 1);
    s.tick(6);
    let change = |s: &mut Session, timeout: u64| {
        s.run(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
            "ReceiptHandle": "r-1", "VisibilityTimeout": timeout}),
        )
    };
    assert_eq!(change(&mut s, 20), json!({"ok": true}));
    s.tick(19);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    s.tick(1);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&change(&mut s, 5)), "ReceiptHandleExpired");

    let again = s.receive("q", 1);
    assert_eq!(again[0]["ReceiveCount"], 2);
    let response = s.run(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
        "ReceiptHandle": "r-2", "VisibilityTimeout": 0}),
    );
    assert_eq!(response, json!({"ok": true}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let response = s.run(
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
        "ReceiptHandle": "r-2", "VisibilityTimeout": 43201}),
    );
    assert_eq!(error_code(&response), "InvalidParameterValue");
}

#[test]
fn send_message_rules_apply_in_spec_order() {
    let mut s = Session::default();
    s.create("std", json!({}));
    s.create("f.fifo", fifo_attributes());
    let send = |s: &mut Session, queue: &str, extra: Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": queue, "Body": "b"});
        command
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        s.run(command)
    };
    assert_eq!(
        error_code(&send(&mut s, "std", json!({"GroupId": "g"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        error_code(&send(&mut s, "std", json!({"DeduplicationId": "d"}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        error_code(&send(&mut s, "f.fifo", json!({"DelaySeconds": 0}))),
        "UnsupportedOperation"
    );
    assert_eq!(
        error_code(&send(&mut s, "f.fifo", json!({"DeduplicationId": "d"}))),
        "MissingParameter"
    );
    assert_eq!(
        error_code(&send(&mut s, "f.fifo", json!({"GroupId": "g"}))),
        "MissingParameter"
    );
    assert_eq!(
        error_code(&send(
            &mut s,
            "f.fifo",
            json!({"GroupId": "g!", "DeduplicationId": "d"})
        )),
        "InvalidParameterValue"
    );
    assert_eq!(
        error_code(&send(&mut s, "std", json!({"DelaySeconds": 901}))),
        "InvalidParameterValue"
    );
    assert_eq!(
        error_code(&s.run(json!({"Command": "SendMessage", "QueueName": "std", "Body": ""}))),
        "InvalidParameterValue"
    );
    let long = "\u{e9}".repeat(513);
    assert_eq!(
        error_code(&s.run(json!({"Command": "SendMessage", "QueueName": "std", "Body": long}))),
        "InvalidParameterValue"
    );
    let fits = "\u{e9}".repeat(512);
    s.send("std", &fits);
    assert_eq!(
        s.send("std", "x"),
        json!({"ok": true, "MessageId": "m-2"}),
        "failed sends use no id"
    );
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_groups_with_hidden_messages() {
    let mut s = Session::default();
    s.create("q.fifo", fifo_attributes());
    assert_eq!(
        s.send_fifo("q.fifo", "a1", "A", "1"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    s.send_fifo("q.fifo", "a2", "A", "2");
    s.send_fifo("q.fifo", "b1", "B", "3");
    s.send_fifo("q.fifo", "b2", "B", "4");

    let first = s.receive("q.fifo", 10);
    let summary: Vec<_> = first
        .iter()
        .map(|m| {
            (
                m["Body"].clone(),
                m["GroupId"].clone(),
                m["SequenceNumber"].clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (json!("a1"), json!("A"), json!(1)),
            (json!("b1"), json!("B"), json!(3))
        ]
    );
    assert!(
        s.receive("q.fifo", 10).is_empty(),
        "both groups have an in-flight message"
    );

    let delete = |s: &mut Session, handle: &str| {
        s.ok(json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": handle}))
    };
    delete(&mut s, "r-1");
    assert_eq!(s.receive_bodies("q.fifo", 10), ["a2"]);
}

#[test]
fn fifo_group_stays_blocked_while_its_earlier_message_is_in_flight_after_a_later_one_is_visible() {
    let mut s = Session::default();
    s.create("q.fifo", fifo_attributes());
    s.send_fifo("q.fifo", "a1", "A", "1");
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q.fifo", "VisibilityTimeout": 100}));
    s.send_fifo("q.fifo", "a2", "A", "2");
    assert!(s.receive("q.fifo", 10).is_empty());
    assert_eq!(
        s.counts("q.fifo"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn fifo_deduplication_window_is_300_seconds_and_not_extended_by_duplicates() {
    let mut s = Session::default();
    s.create("q.fifo", fifo_attributes());
    s.create("p.fifo", fifo_attributes());
    let original = s.send_fifo("q.fifo", "a", "A", "id");
    assert_eq!(
        original,
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    s.tick(299);
    assert_eq!(s.send_fifo("q.fifo", "other body", "B", "id"), original);
    assert_eq!(
        s.send_fifo("p.fifo", "a", "A", "id")["MessageId"],
        "m-2",
        "ids are per queue"
    );
    s.tick(1);
    assert_eq!(
        s.send_fifo("q.fifo", "a", "A", "id"),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 2})
    );
    s.tick(299);
    assert_eq!(s.send_fifo("q.fifo", "a", "A", "id")["MessageId"], "m-3");
}

#[test]
fn fifo_deduplication_id_outlives_the_message() {
    let mut s = Session::default();
    s.create("q.fifo", fifo_attributes());
    let original = s.send_fifo("q.fifo", "a", "A", "id");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q.fifo"}));
    assert_eq!(s.send_fifo("q.fifo", "a", "A", "id"), original);
    assert_eq!(
        s.counts("q.fifo"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        s.send_fifo("q.fifo", "b", "A", "id2")["SequenceNumber"],
        2,
        "sequence numbers carry on"
    );
}

#[test]
fn content_based_deduplication_uses_the_body_unless_an_id_is_given() {
    let mut s = Session::default();
    s.create(
        "q.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |s: &mut Session, body: &str| {
        s.ok(json!({"Command": "SendMessage", "QueueName": "q.fifo", "Body": body, "GroupId": "g"}))
    };
    let first = send(&mut s, "same");
    assert_eq!(send(&mut s, "same"), first);
    assert_eq!(send(&mut s, "different")["SequenceNumber"], 2);
    let explicit = s.send_fifo("q.fifo", "same", "g", "same");
    assert_eq!(
        explicit, first,
        "an explicit id equal to the body shares the window"
    );
    let explicit = s.send_fifo("q.fifo", "same", "g", "other");
    assert_eq!(explicit["SequenceNumber"], 3);
}

#[test]
fn a_queue_holds_at_most_100000_messages_in_any_state() {
    let mut s = Session::default();
    s.create("q", json!({}));
    let started = std::time::Instant::now();
    for _ in 0..100_000 {
        let response = s.raw(br#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#);
        assert_eq!(response["ok"], true);
    }
    let response = s.raw(br#"{"Command":"SendMessage","QueueName":"q","Body":"x"}"#);
    assert_eq!(error_code(&response), "LimitExceeded");
    assert!(
        started.elapsed().as_secs() < 20,
        "filling a queue must take seconds"
    );
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(s.send("q", "x")["MessageId"], "m-100001");
}

#[test]
fn messages_are_removed_when_retention_runs_out_whatever_their_state() {
    let mut s = Session::default();
    s.create(
        "q",
        json!({"RetentionSeconds": 100, "VisibilityTimeout": 1000, "DelaySeconds": 500}),
    );
    s.send("q", "delayed");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}));
    s.send("q", "in flight");
    s.send("q", "visible");
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q"}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    s.tick(99);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    s.tick(1);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let delete = json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"});
    assert_eq!(error_code(&s.run(delete)), "ReceiptHandleExpired");
}

#[test]
fn lowering_retention_removes_messages_at_once_and_raising_it_brings_none_back() {
    let mut s = Session::default();
    s.create("q", json!({}));
    s.send("q", "old");
    s.tick(100);
    s.send("q", "new");
    let set = |s: &mut Session, retention: u64| {
        s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": retention}}))
    };
    set(&mut s, 100);
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    set(&mut s, 1_000_000);
    assert_eq!(s.receive_bodies("q", 10), ["new"]);
}

#[test]
fn retention_is_not_revived_by_raising_it_after_the_deadline_passed_in_a_tick() {
    let mut s = Session::default();
    s.create("q", json!({"RetentionSeconds": 60}));
    s.send("q", "a");
    s.tick(60);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1000}}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

fn dead_letter_setup(s: &mut Session, dead_letter_attributes: Value) {
    s.create("dead", dead_letter_attributes);
    s.create(
        "q",
        json!({"VisibilityTimeout": 1, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
    );
}

#[test]
fn message_moves_to_the_dead_letter_queue_after_max_receive_count_deliveries() {
    let mut s = Session::default();
    dead_letter_setup(&mut s, json!({}));
    s.send("q", "poison");
    s.send("q", "fine");
    s.receive("q", 1);
    s.tick(1);
    s.receive("q", 1);
    s.tick(1);
    let delivered = s.receive("q", 10);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0]["Body"], "fine");
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );

    let moved = s.receive("dead", 1);
    assert_eq!(
        moved,
        [
            json!({"MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "poison", "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 2}),
        ]
    );
}

#[test]
fn dead_lettered_messages_do_not_count_toward_max_messages() {
    let mut s = Session::default();
    dead_letter_setup(&mut s, json!({}));
    s.send("q", "a");
    s.receive("q", 1);
    s.tick(1);
    s.receive("q", 1);
    s.tick(1);
    s.send("q", "b");
    assert_eq!(s.receive_bodies("q", 1), ["b"]);
    assert_eq!(
        s.counts("dead"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn dead_letter_move_resets_counters_and_gives_fifo_messages_new_sequence_numbers() {
    let mut s = Session::default();
    s.create("dead.fifo", fifo_attributes());
    s.create(
        "q.fifo",
        json!({"Fifo": true, "VisibilityTimeout": 1,
        "DeadLetterQueue": "dead.fifo", "MaxReceiveCount": 1}),
    );
    s.send_fifo("dead.fifo", "native", "G", "n");
    s.send_fifo("q.fifo", "a", "A", "1");
    s.send_fifo("q.fifo", "a-next", "A", "2");
    s.receive("q.fifo", 1);
    s.tick(1);
    // "a" is dead-lettered; its group is then free, so "a-next" is delivered in the same call.
    let delivered = s.receive("q.fifo", 10);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0]["Body"], "a-next");

    let moved = s.receive("dead.fifo", 10);
    let summary: Vec<_> = moved
        .iter()
        .map(|m| {
            (
                m["Body"].clone(),
                m["MessageId"].clone(),
                m["ReceiveCount"].clone(),
                m["SequenceNumber"].clone(),
                m["SentAt"].clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (json!("native"), json!("m-1"), json!(1), json!(1), json!(0)),
            (json!("a"), json!("m-2"), json!(1), json!(2), json!(0)),
        ]
    );
    assert_eq!(moved[1]["GroupId"], "A");
    assert_eq!(moved[1]["FirstReceivedAt"], 1);
}

#[test]
fn message_already_past_the_dead_letter_queues_retention_is_dropped_on_arrival() {
    let mut s = Session::default();
    s.create("dead", json!({"RetentionSeconds": 60}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 1, "DeadLetterQueue": "dead", "MaxReceiveCount": 1}),
    );
    s.send("q", "old");
    s.receive("q", 1);
    s.tick(60);
    assert!(s.receive("q", 1).is_empty());
    assert_eq!(
        s.counts("dead"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_new_dead_letter_configuration_applies_from_the_next_receive() {
    let mut s = Session::default();
    s.create("dead", json!({}));
    s.create("q", json!({"VisibilityTimeout": 1}));
    s.send("q", "a");
    s.receive("q", 1);
    s.tick(1);
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"DeadLetterQueue": "dead", "MaxReceiveCount": 1}}));
    assert_eq!(
        s.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert!(s.receive("q", 1).is_empty());
    assert_eq!(
        s.counts("dead"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    let mut s = Session::default();
    s.create("q.fifo", fifo_attributes());
    s.send_fifo("q.fifo", "a", "A", "id");
    s.receive("q.fifo", 1);
    s.ok(json!({"Command": "DeleteQueue", "QueueName": "q.fifo"}));
    s.create("q.fifo", fifo_attributes());
    assert_eq!(
        s.send_fifo("q.fifo", "a", "A", "id"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 1})
    );
    let delete = json!({"Command": "DeleteMessage", "QueueName": "q.fifo", "ReceiptHandle": "r-1"});
    assert_eq!(error_code(&s.run(delete)), "ReceiptHandleExpired");
}

#[test]
fn receive_message_validates_its_parameters() {
    let mut s = Session::default();
    s.create("q", json!({}));
    for extra in [
        json!({"MaxMessages": 0}),
        json!({"MaxMessages": 11}),
        json!({"VisibilityTimeout": 43201}),
        json!({"MaxMessages": 1.5}),
    ] {
        let mut command = json!({"Command": "ReceiveMessage", "QueueName": "q"});
        command
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        assert_eq!(
            error_code(&s.run(command)),
            "InvalidParameterValue",
            "{extra}"
        );
    }
    assert_eq!(
        s.run(json!({"Command": "ReceiveMessage", "QueueName": "q"})),
        json!({"ok": true, "Messages": []})
    );
}
