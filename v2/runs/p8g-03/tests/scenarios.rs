use miniq::broker::Broker;
use serde_json::{Value, json};

/// Feeds commands to a broker one line at a time, like the binary does.
struct Session(Broker);

impl Session {
    fn new() -> Self {
        Session(Broker::default())
    }

    fn run(&mut self, command: Value) -> Value {
        let line = self.0.handle_line(command.to_string().as_bytes());
        serde_json::from_str(&line).expect("responses are JSON")
    }

    fn create(&mut self, name: &str, attributes: Value) -> Value {
        self.run(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}))
    }

    fn send(&mut self, queue: &str, body: &str) -> Value {
        self.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    fn receive(&mut self, queue: &str) -> Value {
        self.run(json!({"Command": "ReceiveMessage", "QueueName": queue}))
    }

    fn tick(&mut self, seconds: u64) -> Value {
        self.run(json!({"Command": "Tick", "Seconds": seconds}))
    }
}

fn ok() -> Value {
    json!({"ok": true})
}

fn error_code(response: &Value) -> &str {
    assert_eq!(response["ok"], json!(false), "{response}");
    response["error"]["code"]
        .as_str()
        .expect("error has a code")
}

fn bodies(response: &Value) -> Vec<&str> {
    response["Messages"]
        .as_array()
        .expect("a Messages array")
        .iter()
        .map(|m| m["Body"].as_str().expect("a body"))
        .collect()
}

#[test]
fn create_queue_rules_fire_in_the_documented_order() {
    let mut s = Session::new();
    assert_eq!(s.create("base", json!({})), ok());
    assert_eq!(s.create("base.fifo", json!({"Fifo": true})), ok());
    let cases = [
        ("plain", json!({"Fifo": true}), "QueueNameInvalid"),
        ("x.fifo", json!({}), "QueueNameInvalid"),
        ("bad name", json!({"Fifo": "yes"}), "QueueNameInvalid"),
        (
            "c",
            json!({"ContentBasedDeduplication": true}),
            "InvalidParameterValue",
        ),
        ("c", json!({"MaxReceiveCount": 3}), "InvalidParameterValue"),
        (
            "c",
            json!({"DeadLetterQueue": "base"}),
            "InvalidParameterValue",
        ),
        (
            "c",
            json!({"VisibilityTimeout": 43201}),
            "InvalidParameterValue",
        ),
        (
            "c",
            json!({"VisibilityTimeout": 1.5}),
            "InvalidParameterValue",
        ),
        ("c", json!({"Colour": "red"}), "InvalidParameterValue"),
        (
            "c",
            json!({"DeadLetterQueue": "nope", "MaxReceiveCount": 3}),
            "QueueDoesNotExist",
        ),
        (
            "c",
            json!({"DeadLetterQueue": "base.fifo", "MaxReceiveCount": 3}),
            "InvalidParameterValue",
        ),
        (
            "base",
            json!({"VisibilityTimeout": 31}),
            "QueueAlreadyExists",
        ),
    ];
    for (name, attributes, code) in cases {
        let response = s.create(name, attributes.clone());
        assert_eq!(error_code(&response), code, "{name} {attributes}");
    }
}

#[test]
fn creating_an_existing_queue_with_equal_attributes_changes_nothing() {
    let mut s = Session::new();
    assert_eq!(s.create("q", json!({"VisibilityTimeout": 5})), ok());
    assert_eq!(s.send("q", "kept")["ok"], json!(true));
    assert_eq!(
        s.create("q", json!({"VisibilityTimeout": 5, "DelaySeconds": null})),
        ok()
    );
    assert_eq!(bodies(&s.receive("q")), vec!["kept"]);
}

#[test]
fn queue_attributes_report_defaults_and_counts() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 10}));
    s.send("q", "delayed");
    s.run(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 0}}));
    s.send("q", "now");
    s.receive("q");
    let response = s.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(
        response,
        json!({
            "ok": true,
            "Attributes": {
                "VisibilityTimeout": 30,
                "DelaySeconds": 0,
                "RetentionSeconds": 345600,
                "Fifo": false,
                "ContentBasedDeduplication": false
            },
            "Counts": {"Visible": 0, "InFlight": 1, "Delayed": 1}
        })
    );
}

#[test]
fn message_states_follow_the_clock() {
    let mut s = Session::new();
    s.create("q", json!({"DelaySeconds": 5, "VisibilityTimeout": 10}));
    assert_eq!(s.send("q", "a"), json!({"ok": true, "MessageId": "m-1"}));
    assert_eq!(bodies(&s.receive("q")), Vec::<&str>::new());
    s.tick(5);
    let first = s.receive("q");
    assert_eq!(
        first,
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-1", "Body": "a",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 5
        }]})
    );
    s.tick(9);
    assert_eq!(bodies(&s.receive("q")), Vec::<&str>::new());
    s.tick(1);
    let second = s.receive("q");
    assert_eq!(second["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(second["Messages"][0]["FirstReceivedAt"], json!(5));
    assert_eq!(second["Messages"][0]["ReceiptHandle"], json!("r-2"));
}

#[test]
fn retention_removes_messages_in_every_state_and_lowering_it_acts_at_once() {
    let mut s = Session::new();
    s.create("q", json!({"RetentionSeconds": 60}));
    s.send("q", "old");
    s.tick(59);
    s.send("q", "young");
    s.tick(1);
    let counts =
        s.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 1, "InFlight": 0, "Delayed": 0}));
    s.run(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}));
    s.tick(59);
    s.run(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1209600}}));
    assert_eq!(bodies(&s.receive("q")), Vec::<&str>::new());
}

#[test]
fn receipt_handles_are_invalid_until_issued_and_expired_when_unusable() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 0}));
    s.create("other", json!({}));
    s.send("q", "a");
    s.send("other", "b");
    s.receive("q");
    let delete = |s: &mut Session, queue: &str, handle: &str| {
        s.run(json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle}))
    };
    let cases = [
        ("q", "r-01", "ReceiptHandleInvalid"),
        ("q", "R-1", "ReceiptHandleInvalid"),
        ("q", "r-1 ", "ReceiptHandleInvalid"),
        ("q", "r-2", "ReceiptHandleInvalid"),
        ("q", "r-1", "ReceiptHandleExpired"),
        ("missing", "r-1", "QueueDoesNotExist"),
    ];
    for (queue, handle, code) in cases {
        assert_eq!(
            error_code(&delete(&mut s, queue, handle)),
            code,
            "{queue} {handle}"
        );
    }
    s.run(json!({"Command": "SetQueueAttributes", "QueueName": "other", "Attributes": {"VisibilityTimeout": 30}}));
    let received = s.receive("other");
    assert_eq!(received["Messages"][0]["ReceiptHandle"], json!("r-2"));
    assert_eq!(
        error_code(&delete(&mut s, "q", "r-2")),
        "ReceiptHandleExpired"
    );
    assert_eq!(delete(&mut s, "other", "r-2"), ok());
    assert_eq!(
        error_code(&delete(&mut s, "other", "r-2")),
        "ReceiptHandleExpired"
    );
}

#[test]
fn change_visibility_replaces_the_end_of_the_timeout() {
    let mut s = Session::new();
    s.create("q", json!({"VisibilityTimeout": 10}));
    s.send("q", "a");
    s.receive("q");
    s.tick(5);
    let change = |s: &mut Session, timeout: u64| {
        s.run(
            json!({"Command": "ChangeMessageVisibility", "QueueName": "q",
                     "ReceiptHandle": "r-1", "VisibilityTimeout": timeout}),
        )
    };
    assert_eq!(change(&mut s, 20), ok());
    s.tick(19);
    assert_eq!(bodies(&s.receive("q")), Vec::<&str>::new());
    assert_eq!(change(&mut s, 0), ok());
    assert_eq!(error_code(&change(&mut s, 5)), "ReceiptHandleExpired");
    assert_eq!(s.receive("q")["Messages"][0]["ReceiveCount"], json!(2));
}

#[test]
fn fifo_queue_delivers_one_message_per_group_in_order() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    let send = |s: &mut Session, body: &str, group: &str, dedup: &str| {
        s.run(
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body,
                     "GroupId": group, "DeduplicationId": dedup}),
        )
    };
    assert_eq!(
        send(&mut s, "a1", "A", "1"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    send(&mut s, "b1", "B", "2");
    send(&mut s, "a2", "A", "3");
    let batch =
        s.run(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}));
    assert_eq!(bodies(&batch), vec!["a1", "b1"]);
    assert_eq!(batch["Messages"][1]["GroupId"], json!("B"));
    assert_eq!(batch["Messages"][1]["SequenceNumber"], json!(2));
    let blocked =
        s.run(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}));
    assert_eq!(bodies(&blocked), Vec::<&str>::new());
}

#[test]
fn fifo_send_rules() {
    let mut s = Session::new();
    s.create("f.fifo", json!({"Fifo": true}));
    s.create("std", json!({}));
    let send = |s: &mut Session, queue: &str, extra: Value| {
        let mut command = json!({"Command": "SendMessage", "QueueName": queue, "Body": "x"});
        command
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        s.run(command)
    };
    let cases = [
        ("std", json!({"GroupId": "g"}), "UnsupportedOperation"),
        (
            "std",
            json!({"DeduplicationId": "d"}),
            "UnsupportedOperation",
        ),
        (
            "f.fifo",
            json!({"GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 0}),
            "UnsupportedOperation",
        ),
        (
            "f.fifo",
            json!({"DeduplicationId": "d"}),
            "MissingParameter",
        ),
        ("f.fifo", json!({"GroupId": "g"}), "MissingParameter"),
        (
            "f.fifo",
            json!({"GroupId": "g!", "DeduplicationId": "d"}),
            "InvalidParameterValue",
        ),
        ("std", json!({"DelaySeconds": 901}), "InvalidParameterValue"),
    ];
    for (queue, extra, code) in cases {
        assert_eq!(
            error_code(&send(&mut s, queue, extra.clone())),
            code,
            "{queue} {extra}"
        );
    }
}

#[test]
fn duplicate_sends_return_the_original_and_the_window_does_not_extend() {
    let mut s = Session::new();
    s.create(
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true}),
    );
    let send = |s: &mut Session, body: &str| {
        s.run(
            json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g"}),
        )
    };
    assert_eq!(
        send(&mut s, "same"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    s.tick(299);
    assert_eq!(
        send(&mut s, "same"),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    s.tick(1);
    assert_eq!(
        send(&mut s, "same"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    s.run(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(
        send(&mut s, "same"),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        send(&mut s, "other"),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn exhausted_messages_move_to_the_dead_letter_queue_and_start_over() {
    let mut s = Session::new();
    s.create("dead", json!({"RetentionSeconds": 1000}));
    s.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dead", "MaxReceiveCount": 2}),
    );
    s.send("q", "poison");
    s.send("q", "fine");
    assert_eq!(bodies(&s.receive("q")), vec!["poison"]);
    assert_eq!(bodies(&s.receive("q")), vec!["poison"]);
    let third = s.run(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 5}));
    assert_eq!(bodies(&third), vec!["fine"]);
    let moved = s.receive("dead");
    assert_eq!(
        moved,
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1", "ReceiptHandle": "r-4", "Body": "poison",
            "ReceiveCount": 1, "SentAt": 0, "FirstReceivedAt": 0
        }]})
    );
}

#[test]
fn dead_lettered_messages_that_are_too_old_vanish_on_arrival() {
    let mut s = Session::new();
    s.create("dead", json!({"RetentionSeconds": 60}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 1, "VisibilityTimeout": 0}),
    );
    s.send("q", "old");
    s.receive("q");
    s.tick(60);
    assert_eq!(bodies(&s.receive("q")), Vec::<&str>::new());
    let counts =
        s.run(json!({"Command": "GetQueueAttributes", "QueueName": "dead"}))["Counts"].clone();
    assert_eq!(counts, json!({"Visible": 0, "InFlight": 0, "Delayed": 0}));
}

#[test]
fn set_queue_attributes_validates_before_changing_anything() {
    let mut s = Session::new();
    s.create("a", json!({}));
    s.create("b", json!({}));
    s.create("f.fifo", json!({"Fifo": true}));
    let set = |s: &mut Session, attributes: Value| {
        s.run(json!({"Command": "SetQueueAttributes", "QueueName": "a", "Attributes": attributes}))
    };
    let cases = [
        (json!({"Fifo": false}), "UnsupportedOperation"),
        (
            json!({"ContentBasedDeduplication": true}),
            "InvalidParameterValue",
        ),
        (json!({"MaxReceiveCount": 2}), "InvalidParameterValue"),
        (
            json!({"DeadLetterQueue": "ghost", "MaxReceiveCount": 2}),
            "QueueDoesNotExist",
        ),
        (
            json!({"DeadLetterQueue": "a", "MaxReceiveCount": 2}),
            "InvalidParameterValue",
        ),
        (
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 2}),
            "InvalidParameterValue",
        ),
        (
            json!({"VisibilityTimeout": 9, "DelaySeconds": 901}),
            "InvalidParameterValue",
        ),
    ];
    for (attributes, code) in cases {
        assert_eq!(
            error_code(&set(&mut s, attributes.clone())),
            code,
            "{attributes}"
        );
    }
    let unchanged = s.run(json!({"Command": "GetQueueAttributes", "QueueName": "a"}));
    assert_eq!(unchanged["Attributes"]["VisibilityTimeout"], json!(30));
    assert_eq!(
        set(
            &mut s,
            json!({"DeadLetterQueue": "b", "MaxReceiveCount": 2})
        ),
        ok()
    );
}

#[test]
fn deleting_a_queue_clears_dead_letter_references_and_the_name_is_reusable() {
    let mut s = Session::new();
    s.create("dead", json!({}));
    s.create(
        "q",
        json!({"DeadLetterQueue": "dead", "MaxReceiveCount": 3}),
    );
    s.send("dead", "x");
    assert_eq!(
        s.run(json!({"Command": "DeleteQueue", "QueueName": "dead"})),
        ok()
    );
    let attributes = s.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(attributes["Attributes"].get("DeadLetterQueue"), None);
    assert_eq!(attributes["Attributes"].get("MaxReceiveCount"), None);
    s.create("dead", json!({}));
    assert_eq!(bodies(&s.receive("dead")), Vec::<&str>::new());
    let missing = s.run(json!({"Command": "DeleteQueue", "QueueName": "dead!"}));
    assert_eq!(error_code(&missing), "QueueDoesNotExist");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut s = Session::new();
    for name in ["b", "a_1", "a-1", "a.fifo", "A1", "a1", "a"] {
        let attributes = json!({"Fifo": name.ends_with(".fifo")});
        assert_eq!(s.create(name, attributes), ok(), "{name}");
    }
    let all = s.run(json!({"Command": "ListQueues"}));
    assert_eq!(
        all,
        json!({"ok": true, "QueueNames": ["A1", "a", "a-1", "a.fifo", "a1", "a_1", "b"]})
    );
    let some = s.run(json!({"Command": "ListQueues", "Prefix": "a"}));
    assert_eq!(
        some,
        json!({"ok": true, "QueueNames": ["a", "a-1", "a.fifo", "a1", "a_1"]})
    );
}

#[test]
fn queue_and_message_limits_are_enforced() {
    let mut s = Session::new();
    s.create("full", json!({}));
    let send = r#"{"Command":"SendMessage","QueueName":"full","Body":"x"}"#;
    for _ in 0..100_000 {
        let line = s.0.handle_line(send.as_bytes());
        assert!(line.starts_with(r#"{"ok":true"#), "{line}");
    }
    assert_eq!(error_code(&s.send("full", "one too many")), "LimitExceeded");
    s.run(json!({"Command": "PurgeQueue", "QueueName": "full"}));
    assert_eq!(s.send("full", "again")["MessageId"], json!("m-100001"));

    for n in 1..1000 {
        assert_eq!(s.create(&format!("q{n}"), json!({})), ok());
    }
    assert_eq!(error_code(&s.create("last", json!({}))), "LimitExceeded");
}

#[test]
fn failed_commands_do_not_use_up_identifiers() {
    let mut s = Session::new();
    s.create("q", json!({}));
    assert_eq!(error_code(&s.send("q", "")), "InvalidParameterValue");
    assert_eq!(s.send("q", "ok")["MessageId"], json!("m-1"));
}

#[test]
fn the_binary_answers_every_line_including_an_unterminated_last_one() {
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
        .write_all(b"\n{\"Command\":\"Tick\",\"Seconds\":3}\n{\"Command\":\"Tick\",\"Seconds\":4}")
        .expect("input is written");
    let output = child.wait_with_output().expect("miniq exits");
    assert_eq!(output.status.code(), Some(0));
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .expect("output is UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is JSON"))
        .collect();
    assert_eq!(error_code(&lines[0]), "InvalidRequest");
    assert_eq!(
        lines[1..],
        [json!({"ok": true, "Now": 3}), json!({"ok": true, "Now": 7})]
    );
}
