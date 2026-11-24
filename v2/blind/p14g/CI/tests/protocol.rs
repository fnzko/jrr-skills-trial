use miniq::broker::Broker;
use serde_json::{Value, json};

struct Session(Broker);

impl Session {
    fn new() -> Self {
        Self(Broker::default())
    }

    fn raw(&mut self, line: &[u8]) -> Value {
        serde_json::to_value(self.0.handle_line(line)).expect("replies serialize")
    }

    fn run(&mut self, command: Value) -> Value {
        self.raw(command.to_string().as_bytes())
    }

    fn create(&mut self, name: &str, attributes: Value) {
        let reply = self
            .run(json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes}));
        assert_eq!(reply, json!({"ok": true}));
    }

    fn send(&mut self, queue: &str, body: &str) -> Value {
        self.run(json!({"Command": "SendMessage", "QueueName": queue, "Body": body}))
    }

    fn receive(&mut self, queue: &str) -> Value {
        self.run(json!({"Command": "ReceiveMessage", "QueueName": queue}))
    }

    fn tick(&mut self, seconds: u32) {
        self.run(json!({"Command": "Tick", "Seconds": seconds}));
    }

    fn counts(&mut self, queue: &str) -> Value {
        let reply = self.run(json!({"Command": "GetQueueAttributes", "QueueName": queue}));
        reply["Counts"].clone()
    }
}

fn error_code(reply: &Value) -> &str {
    assert_eq!(reply["ok"], json!(false), "{reply}");
    reply["error"]["code"].as_str().expect("error has a code")
}

fn received(id: u32, handle: u32, body: &str, count: u32, sent_at: u32, first: u32) -> Value {
    json!({
        "MessageId": format!("m-{id}"), "ReceiptHandle": format!("r-{handle}"), "Body": body,
        "ReceiveCount": count, "SentAt": sent_at, "FirstReceivedAt": first,
    })
}

#[test]
fn malformed_lines_get_invalid_request_and_the_session_continues() {
    let mut session = Session::new();
    let lines: [&[u8]; 8] = [
        b"",
        b"   ",
        b"not json",
        b"[1]",
        b"{}",
        br#"{"Command": 5}"#,
        br#"{"Command": "Nope"}"#,
        b"{\"Command\": \"Tick\", \"Seconds\": 1} trailing",
    ];
    for line in lines {
        assert_eq!(error_code(&session.raw(line)), "InvalidRequest", "{line:?}");
    }
    assert_eq!(
        error_code(&session.raw(b"{\"Command\": \"ListQueues\xff\"}")),
        "InvalidRequest"
    );
    assert_eq!(
        session.run(json!({"Command": "Tick", "Seconds": 7})),
        json!({"ok": true, "Now": 7})
    );
}

#[test]
fn null_members_count_as_absent_and_unknown_members_are_ignored() {
    let mut session = Session::new();
    let reply = session
        .run(json!({"Command": "CreateQueue", "QueueName": "q", "Attributes": null, "Extra": 1}));
    assert_eq!(reply, json!({"ok": true}));
    let reply = session.run(json!({"Command": "CreateQueue", "QueueName": null}));
    assert_eq!(error_code(&reply), "MissingParameter");
    let reply = session.run(json!({"Command": "CreateQueue", "QueueName": "q2", "Attributes": {"DelaySeconds": null, "Bogus": null}}));
    assert_eq!(reply, json!({"ok": true}));
}

#[test]
fn integers_must_be_written_without_fraction_or_exponent() {
    let mut session = Session::new();
    for seconds in [
        "30.0",
        "3e1",
        "\"30\"",
        "-1",
        "1209601",
        "99999999999999999999",
    ] {
        let line = format!(r#"{{"Command": "Tick", "Seconds": {seconds}}}"#);
        assert_eq!(
            error_code(&session.raw(line.as_bytes())),
            "InvalidParameterValue",
            "{seconds}"
        );
    }
    let reply = session.raw(br#"{"Command": "Tick", "Seconds": 30}"#);
    assert_eq!(reply, json!({"ok": true, "Now": 30}));
}

#[test]
fn checks_run_in_the_documented_order() {
    let mut session = Session::new();
    // Missing beats invalid.
    let reply = session.run(json!({"Command": "SendMessage", "QueueName": 5}));
    assert_eq!(error_code(&reply), "MissingParameter");
    // Invalid value beats queue lookup.
    let reply = session.run(json!({"Command": "SendMessage", "QueueName": "nope", "Body": ""}));
    assert_eq!(error_code(&reply), "InvalidParameterValue");
    // Lookup accepts any string and beats the command's own rules.
    let reply = session.run(json!({"Command": "SetQueueAttributes", "QueueName": "bad name!", "Attributes": {"Fifo": true}}));
    assert_eq!(error_code(&reply), "QueueDoesNotExist");
    // The queue name syntax is checked before other CreateQueue parameters.
    let reply =
        session.run(json!({"Command": "CreateQueue", "QueueName": "bad name", "Attributes": 3}));
    assert_eq!(error_code(&reply), "QueueNameInvalid");
    let reply = session
        .run(json!({"Command": "CreateQueue", "QueueName": "ok", "Attributes": {"Unknown": 1}}));
    assert_eq!(error_code(&reply), "InvalidParameterValue");
}

#[test]
fn list_queues_sorts_by_byte_value_and_filters_by_prefix() {
    let mut session = Session::new();
    for name in ["b", "_a", "a-1", "a.fifo", "a1", "A", "a"] {
        let attributes = if name.ends_with(".fifo") {
            json!({"Fifo": true})
        } else {
            json!({})
        };
        session.create(name, attributes);
    }
    let reply = session.run(json!({"Command": "ListQueues"}));
    assert_eq!(
        reply,
        json!({"ok": true, "QueueNames": ["A", "_a", "a", "a-1", "a.fifo", "a1", "b"]})
    );
    let reply = session.run(json!({"Command": "ListQueues", "Prefix": "a"}));
    assert_eq!(
        reply,
        json!({"ok": true, "QueueNames": ["a", "a-1", "a.fifo", "a1"]})
    );
    let reply = session.run(json!({"Command": "ListQueues", "Prefix": "zz"}));
    assert_eq!(reply, json!({"ok": true, "QueueNames": []}));
}

#[test]
fn create_queue_is_idempotent_only_for_equal_attributes() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    session.create("q", json!({"VisibilityTimeout": 10, "DelaySeconds": 0}));
    let reply = session.run(json!({"Command": "CreateQueue", "QueueName": "q"}));
    assert_eq!(error_code(&reply), "QueueAlreadyExists");
    let table = [
        (json!({"QueueName": "x.fifo"}), "QueueNameInvalid"),
        (
            json!({"QueueName": "x", "Attributes": {"Fifo": true}}),
            "QueueNameInvalid",
        ),
        (
            json!({"QueueName": "x", "Attributes": {"ContentBasedDeduplication": true}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "x", "Attributes": {"MaxReceiveCount": 2}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "x", "Attributes": {"DeadLetterQueue": "q", "MaxReceiveCount": 2, "RetentionSeconds": 59}}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "x", "Attributes": {"DeadLetterQueue": "gone", "MaxReceiveCount": 2}}),
            "QueueDoesNotExist",
        ),
        (
            json!({"QueueName": "x.fifo", "Attributes": {"Fifo": true, "DeadLetterQueue": "q", "MaxReceiveCount": 2}}),
            "InvalidParameterValue",
        ),
    ];
    for (mut command, code) in table {
        command["Command"] = json!("CreateQueue");
        assert_eq!(error_code(&session.run(command.clone())), code, "{command}");
    }
}

#[test]
fn queue_attributes_report_defaults_and_dead_letter_only_when_set() {
    let mut session = Session::new();
    session.create("dlq", json!({}));
    session.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 3}));
    let reply = session.run(json!({"Command": "GetQueueAttributes", "QueueName": "dlq"}));
    assert_eq!(
        reply,
        json!({"ok": true,
        "Attributes": {"VisibilityTimeout": 30, "DelaySeconds": 0, "RetentionSeconds": 345600,
                       "Fifo": false, "ContentBasedDeduplication": false},
        "Counts": {"Visible": 0, "InFlight": 0, "Delayed": 0}})
    );
    let reply = session.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(reply["Attributes"]["DeadLetterQueue"], json!("dlq"));
    assert_eq!(reply["Attributes"]["MaxReceiveCount"], json!(3));
}

#[test]
fn set_queue_attributes_follows_its_rules() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.create("other", json!({}));
    session.create("f.fifo", json!({"Fifo": true}));
    let table = [
        (json!({"Fifo": false}), "UnsupportedOperation"),
        (
            json!({"ContentBasedDeduplication": true}),
            "InvalidParameterValue",
        ),
        (json!({"DeadLetterQueue": "other"}), "InvalidParameterValue"),
        (
            json!({"DeadLetterQueue": "gone", "MaxReceiveCount": 1}),
            "QueueDoesNotExist",
        ),
        (
            json!({"DeadLetterQueue": "q", "MaxReceiveCount": 1}),
            "InvalidParameterValue",
        ),
        (
            json!({"DeadLetterQueue": "f.fifo", "MaxReceiveCount": 1}),
            "InvalidParameterValue",
        ),
    ];
    for (attributes, code) in table {
        let reply = session.run(
            json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": attributes}),
        );
        assert_eq!(error_code(&reply), code, "{attributes}");
    }
    let reply = session.run(json!({"Command": "SetQueueAttributes", "QueueName": "q",
        "Attributes": {"VisibilityTimeout": 5, "DeadLetterQueue": "other", "MaxReceiveCount": 2}}));
    assert_eq!(reply, json!({"ok": true}));
    let reply = session.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(reply["Attributes"]["VisibilityTimeout"], json!(5));
    assert_eq!(reply["Attributes"]["DeadLetterQueue"], json!("other"));
    let reply =
        session.run(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {}}));
    assert_eq!(reply, json!({"ok": true}));
    let reply = session.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert_eq!(reply["Attributes"]["MaxReceiveCount"], json!(2));
}

#[test]
fn standard_queue_delivery_visibility_and_delete() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    assert_eq!(
        session.send("q", "a"),
        json!({"ok": true, "MessageId": "m-1"})
    );
    assert_eq!(
        session.send("q", "b"),
        json!({"ok": true, "MessageId": "m-2"})
    );
    assert_eq!(
        session.receive("q"),
        json!({"ok": true, "Messages": [received(1, 1, "a", 1, 0, 0)]})
    );
    assert_eq!(
        session.receive("q"),
        json!({"ok": true, "Messages": [received(2, 2, "b", 1, 0, 0)]})
    );
    assert_eq!(session.receive("q"), json!({"ok": true, "Messages": []}));
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 2, "Delayed": 0})
    );
    session.tick(10);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 2, "InFlight": 0, "Delayed": 0})
    );
    let reply =
        session.run(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}));
    assert_eq!(error_code(&reply), "ReceiptHandleExpired");
    let reply =
        session.run(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 10}));
    assert_eq!(
        reply,
        json!({"ok": true, "Messages": [received(1, 3, "a", 2, 0, 0), received(2, 4, "b", 2, 0, 0)]})
    );
    let reply =
        session.run(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-3"}));
    assert_eq!(reply, json!({"ok": true}));
    let reply =
        session.run(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-3"}));
    assert_eq!(error_code(&reply), "ReceiptHandleExpired");
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
}

#[test]
fn first_received_at_keeps_the_time_of_the_first_delivery() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 5}));
    session.send("q", "a");
    session.tick(3);
    session.receive("q");
    session.tick(5);
    let reply = session.receive("q");
    assert_eq!(reply["Messages"][0], received(1, 2, "a", 2, 0, 3));
}

#[test]
fn receipt_handles_are_checked_as_exact_strings() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.create("other", json!({}));
    session.send("q", "a");
    session.receive("q");
    let table = [
        ("r-01", "ReceiptHandleInvalid"),
        ("R-1", "ReceiptHandleInvalid"),
        ("r-1 ", "ReceiptHandleInvalid"),
        ("r-2", "ReceiptHandleInvalid"),
        ("junk", "ReceiptHandleInvalid"),
    ];
    for (handle, code) in table {
        let reply = session
            .run(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": handle}));
        assert_eq!(error_code(&reply), code, "{handle}");
    }
    let reply = session
        .run(json!({"Command": "DeleteMessage", "QueueName": "other", "ReceiptHandle": "r-1"}));
    assert_eq!(error_code(&reply), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_replaces_the_timeout_and_zero_releases_the_message() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 10}));
    session.send("q", "a");
    session.receive("q");
    session.tick(4);
    let extend = json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 20});
    assert_eq!(session.run(extend), json!({"ok": true}));
    session.tick(19);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 1, "Delayed": 0})
    );
    let release = json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0});
    assert_eq!(session.run(release.clone()), json!({"ok": true}));
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    assert_eq!(error_code(&session.run(release)), "ReceiptHandleExpired");
}

#[test]
fn zero_visibility_timeout_makes_the_handle_unusable() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.send("q", "a");
    let reply =
        session.run(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0}));
    assert_eq!(reply["Messages"][0]["ReceiptHandle"], json!("r-1"));
    let reply =
        session.run(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}));
    assert_eq!(error_code(&reply), "ReceiptHandleExpired");
}

#[test]
fn delayed_messages_become_visible_after_their_delay() {
    let mut session = Session::new();
    session.create("q", json!({"DelaySeconds": 10}));
    session.send("q", "queue-delay");
    session.run(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "own-delay", "DelaySeconds": 2}),
    );
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 2})
    );
    session.tick(2);
    assert_eq!(
        session.receive("q")["Messages"][0]["Body"],
        json!("own-delay")
    );
    session.tick(8);
    assert_eq!(
        session.receive("q")["Messages"][0]["Body"],
        json!("queue-delay")
    );
}

#[test]
fn send_message_validates_parameters_and_queue_type() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.create("f.fifo", json!({"Fifo": true}));
    let big = "x".repeat(1025);
    let table = [
        (
            json!({"QueueName": "q", "Body": big}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Body": "a", "DelaySeconds": 901}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Body": "a", "GroupId": "bad id"}),
            "InvalidParameterValue",
        ),
        (
            json!({"QueueName": "q", "Body": "a", "GroupId": "g"}),
            "UnsupportedOperation",
        ),
        (
            json!({"QueueName": "q", "Body": "a", "DeduplicationId": "d"}),
            "UnsupportedOperation",
        ),
        (
            json!({"QueueName": "f.fifo", "Body": "a", "GroupId": "g", "DeduplicationId": "d", "DelaySeconds": 0}),
            "UnsupportedOperation",
        ),
        (
            json!({"QueueName": "f.fifo", "Body": "a", "DeduplicationId": "d"}),
            "MissingParameter",
        ),
        (
            json!({"QueueName": "f.fifo", "Body": "a", "GroupId": "g"}),
            "MissingParameter",
        ),
    ];
    for (mut command, code) in table {
        command["Command"] = json!("SendMessage");
        assert_eq!(error_code(&session.run(command.clone())), code, "{command}");
    }
    let body = "é".repeat(512);
    assert_eq!(
        session.send("q", &body),
        json!({"ok": true, "MessageId": "m-1"})
    );
}

#[test]
fn fifo_delivers_one_message_per_group_in_arrival_order() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true, "VisibilityTimeout": 10}));
    for (body, group) in [("a1", "A"), ("b1", "B"), ("a2", "A")] {
        session.run(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": group, "DeduplicationId": body}));
    }
    let reply =
        session.run(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}));
    let mut first = received(1, 1, "a1", 1, 0, 0);
    first["GroupId"] = json!("A");
    first["SequenceNumber"] = json!(1);
    let mut second = received(2, 2, "b1", 1, 0, 0);
    second["GroupId"] = json!("B");
    second["SequenceNumber"] = json!(2);
    assert_eq!(reply, json!({"ok": true, "Messages": [first, second]}));
    // a2 stays blocked while a1 is in flight.
    let reply =
        session.run(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}));
    assert_eq!(reply, json!({"ok": true, "Messages": []}));
    session.run(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}));
    let reply =
        session.run(json!({"Command": "ReceiveMessage", "QueueName": "f.fifo", "MaxMessages": 10}));
    assert_eq!(reply["Messages"][0]["Body"], json!("a2"));
    assert_eq!(reply["Messages"][0]["SequenceNumber"], json!(3));
}

#[test]
fn fifo_duplicates_return_the_original_and_use_no_message_id() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true}));
    let send = |id: &str, body: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": body, "GroupId": "g", "DeduplicationId": id});
    assert_eq!(
        session.run(send("d", "x")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    session.tick(299);
    assert_eq!(
        session.run(send("d", "y")),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    session.tick(1);
    // The duplicate at 299 did not extend the window.
    assert_eq!(
        session.run(send("d", "z")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
    assert_eq!(
        session.run(send("e", "w")),
        json!({"ok": true, "MessageId": "m-3", "SequenceNumber": 3})
    );
}

#[test]
fn deduplication_outlives_the_message_and_survives_purge() {
    let mut session = Session::new();
    session.create(
        "f.fifo",
        json!({"Fifo": true, "ContentBasedDeduplication": true, "VisibilityTimeout": 10}),
    );
    let send =
        json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "same", "GroupId": "g"});
    assert_eq!(
        session.run(send.clone()),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    session.receive("f.fifo");
    session.run(json!({"Command": "DeleteMessage", "QueueName": "f.fifo", "ReceiptHandle": "r-1"}));
    assert_eq!(
        session.run(send.clone()),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    session.run(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(
        session.run(send.clone()),
        json!({"ok": true, "MessageId": "m-1", "SequenceNumber": 1})
    );
    session.tick(300);
    assert_eq!(
        session.run(send),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}

#[test]
fn purge_removes_messages_in_every_state() {
    let mut session = Session::new();
    session.create("q", json!({"VisibilityTimeout": 100}));
    session.send("q", "a");
    session.send("q", "b");
    session.receive("q");
    session
        .run(json!({"Command": "SendMessage", "QueueName": "q", "Body": "c", "DelaySeconds": 50}));
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 1, "Delayed": 1})
    );
    session.run(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
    let reply =
        session.run(json!({"Command": "DeleteMessage", "QueueName": "q", "ReceiptHandle": "r-1"}));
    assert_eq!(error_code(&reply), "ReceiptHandleExpired");
}

#[test]
fn lowering_retention_removes_messages_at_once_and_raising_it_does_not_restore_them() {
    let mut session = Session::new();
    session.create("q", json!({}));
    session.send("q", "old");
    session.tick(100);
    session.send("q", "new");
    let set = |seconds: u32| json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": seconds}});
    session.run(set(60));
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    session.run(set(1_000));
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    session.tick(999);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    session.tick(1);
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn messages_received_too_often_move_to_the_dead_letter_queue() {
    let mut session = Session::new();
    session.create("dlq", json!({"RetentionSeconds": 1000}));
    session.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 2}),
    );
    session.send("q", "a");
    session.send("q", "b");
    session.receive("q");
    session.receive("q");
    // "a" has been received twice: it moves, and "b" is delivered in the same call.
    let reply =
        session.run(json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 1}));
    assert_eq!(
        reply,
        json!({"ok": true, "Messages": [received(2, 3, "b", 1, 0, 0)]})
    );
    assert_eq!(
        session.counts("dlq"),
        json!({"Visible": 1, "InFlight": 0, "Delayed": 0})
    );
    let reply = session.receive("dlq");
    assert_eq!(
        reply,
        json!({"ok": true, "Messages": [received(1, 4, "a", 1, 0, 0)]})
    );
}

#[test]
fn a_dead_lettered_message_older_than_the_target_retention_is_removed_on_arrival() {
    let mut session = Session::new();
    session.create("dlq", json!({"RetentionSeconds": 60}));
    session.create(
        "q",
        json!({"VisibilityTimeout": 0, "DeadLetterQueue": "dlq", "MaxReceiveCount": 1}),
    );
    session.send("q", "a");
    session.receive("q");
    session.tick(60);
    let reply = session.receive("q");
    assert_eq!(reply, json!({"ok": true, "Messages": []}));
    assert_eq!(
        session.counts("dlq"),
        json!({"Visible": 0, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn deleting_a_dead_letter_queue_clears_the_configuration_of_its_sources() {
    let mut session = Session::new();
    session.create("dlq", json!({}));
    session.create("q", json!({"DeadLetterQueue": "dlq", "MaxReceiveCount": 2}));
    let reply = session.run(json!({"Command": "DeleteQueue", "QueueName": "dlq"}));
    assert_eq!(reply, json!({"ok": true}));
    let reply = session.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(
        reply["Attributes"].get("DeadLetterQueue").is_none(),
        "{reply}"
    );
    assert!(
        reply["Attributes"].get("MaxReceiveCount").is_none(),
        "{reply}"
    );
    // A queue created later under the same name is unrelated to q.
    session.create("dlq", json!({}));
    let reply = session.run(json!({"Command": "GetQueueAttributes", "QueueName": "q"}));
    assert!(
        reply["Attributes"].get("DeadLetterQueue").is_none(),
        "{reply}"
    );
}

#[test]
fn a_queue_holds_at_most_100000_messages_and_fills_quickly() {
    let mut session = Session::new();
    session.create("q", json!({}));
    let line = br#"{"Command": "SendMessage", "QueueName": "q", "Body": "x"}"#;
    for _ in 0..100_000 {
        assert_eq!(session.raw(line)["ok"], json!(true));
    }
    assert_eq!(error_code(&session.raw(line)), "LimitExceeded");
    assert_eq!(
        session.counts("q"),
        json!({"Visible": 100_000, "InFlight": 0, "Delayed": 0})
    );
}

#[test]
fn at_most_1000_queues_exist() {
    let mut session = Session::new();
    for number in 0..1000 {
        session.create(&format!("q{number}"), json!({}));
    }
    let reply = session.run(json!({"Command": "CreateQueue", "QueueName": "extra"}));
    assert_eq!(error_code(&reply), "LimitExceeded");
    // Re-creating an existing queue still succeeds at the limit.
    session.create("q0", json!({}));
}

#[test]
fn purge_keeps_counting_fifo_sequence_numbers() {
    let mut session = Session::new();
    session.create("f.fifo", json!({"Fifo": true}));
    let send = |id: &str| json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": id, "GroupId": "g", "DeduplicationId": id});
    session.run(send("a"));
    session.run(json!({"Command": "PurgeQueue", "QueueName": "f.fifo"}));
    assert_eq!(
        session.run(send("b")),
        json!({"ok": true, "MessageId": "m-2", "SequenceNumber": 2})
    );
}
