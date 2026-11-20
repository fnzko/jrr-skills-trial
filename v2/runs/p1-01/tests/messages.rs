mod common;

use common::{Session, counts, delivered};
use serde_json::json;

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn a_standard_message_goes_through_send_receive_and_delete() {
    let mut s = Session::new();
    s.create("q");
    assert_eq!(
        s.send_message("q", "hello"),
        json!({"ok": true, "MessageId": "m-1"})
    );
    s.tick(4);
    assert_eq!(
        s.receive("q"),
        json!({"ok": true, "Messages": [{
            "MessageId": "m-1",
            "ReceiptHandle": "r-1",
            "Body": "hello",
            "ReceiveCount": 1,
            "SentAt": 0,
            "FirstReceivedAt": 4,
        }]})
    );
    assert_eq!(s.receive("q"), json!({"ok": true, "Messages": []}));
    assert_eq!(s.delete_message("q", "r-1"), json!({"ok": true}));
    assert_eq!(s.counts("q"), counts(0, 0, 0));
}

#[test]
fn ids_and_handles_count_over_all_queues_and_failed_sends_use_none() {
    let mut s = Session::new();
    s.create("a");
    s.create_with("f.fifo", json!({"Fifo": true}));
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "f.fifo", "Body": "x"})),
        "MissingParameter"
    );
    assert_eq!(s.send_message("a", "1")["MessageId"], json!("m-1"));
    s.create("b");
    assert_eq!(s.send_message("b", "2")["MessageId"], json!("m-2"));
    assert_eq!(delivered(&s.receive("b")), pairs(&[("m-2", "r-1")]));
    assert_eq!(delivered(&s.receive("a")), pairs(&[("m-1", "r-2")]));
}

#[test]
fn send_message_rejects_what_a_standard_queue_does_not_support() {
    let mut s = Session::new();
    s.create("q");
    for extra in [("GroupId", "g"), ("DeduplicationId", "d")] {
        let mut command = json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"});
        command[extra.0] = json!(extra.1);
        assert_eq!(s.err(command), "UnsupportedOperation", "{}", extra.0);
    }
    assert_eq!(s.counts("q"), counts(0, 0, 0));
}

#[test]
fn send_message_checks_parameters_before_looking_up_the_queue() {
    let mut s = Session::new();
    let send =
        |queue: &str, body| json!({"Command": "SendMessage", "QueueName": queue, "Body": body});
    assert_eq!(s.err(send("ghost", json!(""))), "InvalidParameterValue");
    assert_eq!(s.err(send("ghost", json!("x"))), "QueueDoesNotExist");
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "ghost", "GroupId": "g"})),
        "MissingParameter"
    );
}

#[test]
fn messages_come_back_in_arrival_order_and_keep_their_place_after_a_timeout() {
    let mut s = Session::new();
    s.create_with("q", json!({"VisibilityTimeout": 10}));
    s.send_message("q", "a");
    s.send_message("q", "b");
    s.send_message("q", "c");
    assert_eq!(delivered(&s.receive("q")), pairs(&[("m-1", "r-1")]));
    assert_eq!(delivered(&s.receive("q")), pairs(&[("m-2", "r-2")]));
    s.tick(10);
    let response = s.receive_up_to("q", 10);
    assert_eq!(
        delivered(&response),
        pairs(&[("m-1", "r-3"), ("m-2", "r-4"), ("m-3", "r-5")])
    );
    let history: Vec<_> = response["Messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["ReceiveCount"].clone(), m["FirstReceivedAt"].clone()))
        .collect();
    assert_eq!(
        history,
        [
            (json!(2), json!(0)),
            (json!(2), json!(0)),
            (json!(1), json!(10))
        ]
    );
}

#[test]
fn receive_returns_at_most_max_messages_and_validates_it() {
    let mut s = Session::new();
    s.create("q");
    for body in ["a", "b", "c"] {
        s.send_message("q", body);
    }
    assert_eq!(delivered(&s.receive_up_to("q", 2)).len(), 2);
    assert_eq!(delivered(&s.receive_up_to("q", 10)).len(), 1);
    for bad in [json!(0), json!(11), json!("1"), json!(1.0)] {
        let command = json!({"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": bad});
        assert_eq!(s.err(command), "InvalidParameterValue", "{bad}");
    }
}

#[test]
fn delay_hides_a_message_until_its_time_and_a_new_queue_delay_applies_to_later_sends() {
    let mut s = Session::new();
    s.create_with("q", json!({"DelaySeconds": 5}));
    s.send_message("q", "queue delay");
    s.ok(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "no delay", "DelaySeconds": 0}),
    );
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"DelaySeconds": 20}}));
    s.send_message("q", "later");
    assert_eq!(s.counts("q"), counts(1, 0, 2));
    assert_eq!(
        delivered(&s.receive_up_to("q", 10)),
        pairs(&[("m-2", "r-1")])
    );
    s.tick(5);
    assert_eq!(s.counts("q"), counts(1, 1, 1));
    assert_eq!(
        delivered(&s.receive_up_to("q", 10)),
        pairs(&[("m-1", "r-2")])
    );
    s.tick(15);
    assert_eq!(s.counts("q"), counts(1, 2, 0));
}

#[test]
fn a_zero_visibility_timeout_makes_the_handle_unusable() {
    let mut s = Session::new();
    s.create("q");
    s.send_message("q", "x");
    let command = json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 0});
    assert_eq!(delivered(&s.ok(command.clone())), pairs(&[("m-1", "r-1")]));
    assert_eq!(
        s.delete_message("q", "r-1")["error"]["code"],
        json!("ReceiptHandleExpired")
    );
    let again = s.ok(command);
    assert_eq!(again["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(s.counts("q"), counts(1, 0, 0));
}

#[test]
fn a_new_visibility_timeout_applies_to_later_deliveries_only() {
    let mut s = Session::new();
    s.create("q");
    s.send_message("q", "a");
    s.send_message("q", "b");
    s.receive("q");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"VisibilityTimeout": 5}}));
    s.receive("q");
    s.tick(5);
    assert_eq!(s.counts("q"), counts(1, 1, 0));
}

#[test]
fn handle_errors_distinguish_never_issued_from_no_longer_usable() {
    let mut s = Session::new();
    s.create("q");
    s.create("other");
    s.send_message("q", "x");
    s.receive("q");
    for handle in ["r-2", "r-01", "R-1", "r-1 ", "", "1", "r-0"] {
        assert_eq!(
            s.delete_message("q", handle)["error"]["code"],
            json!("ReceiptHandleInvalid"),
            "{handle:?}"
        );
    }
    assert_eq!(
        s.delete_message("other", "r-1")["error"]["code"],
        json!("ReceiptHandleExpired")
    );
    assert_eq!(
        s.delete_message("ghost", "bogus")["error"]["code"],
        json!("QueueDoesNotExist")
    );
    assert_eq!(s.delete_message("q", "r-1"), json!({"ok": true}));
    assert_eq!(
        s.delete_message("q", "r-1")["error"]["code"],
        json!("ReceiptHandleExpired")
    );
}

#[test]
fn only_the_latest_handle_of_a_message_is_usable() {
    let mut s = Session::new();
    s.create_with("q", json!({"VisibilityTimeout": 10}));
    s.send_message("q", "x");
    s.receive("q");
    s.tick(10);
    s.receive("q");
    assert_eq!(
        s.delete_message("q", "r-1")["error"]["code"],
        json!("ReceiptHandleExpired")
    );
    assert_eq!(s.delete_message("q", "r-2"), json!({"ok": true}));
}

#[test]
fn an_ended_timeout_expires_the_handle() {
    let mut s = Session::new();
    s.create_with("q", json!({"VisibilityTimeout": 10}));
    s.send_message("q", "x");
    s.receive("q");
    s.tick(9);
    let change = |timeout| json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": timeout});
    s.ok(change(10));
    s.tick(9);
    assert_eq!(s.counts("q"), counts(0, 1, 0));
    s.tick(1);
    assert_eq!(s.counts("q"), counts(1, 0, 0));
    assert_eq!(s.err(change(10)), "ReceiptHandleExpired");
}

#[test]
fn change_visibility_to_zero_makes_the_message_visible_at_once() {
    let mut s = Session::new();
    s.create("q");
    s.send_message("q", "x");
    s.receive("q");
    s.ok(json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 0}));
    assert_eq!(s.counts("q"), counts(1, 0, 0));
    let again = s.receive("q");
    assert_eq!(again["Messages"][0]["ReceiveCount"], json!(2));
    assert_eq!(
        s.delete_message("q", "r-1")["error"]["code"],
        json!("ReceiptHandleExpired")
    );
}

#[test]
fn change_visibility_can_lengthen_a_timeout_and_validates_its_value() {
    let mut s = Session::new();
    s.create("q");
    s.send_message("q", "x");
    s.receive("q");
    s.ok(json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": 100}));
    s.tick(99);
    assert_eq!(s.counts("q"), counts(0, 1, 0));
    for bad in [json!(43201), json!(-1), json!("5")] {
        let command = json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1", "VisibilityTimeout": bad});
        assert_eq!(s.err(command), "InvalidParameterValue", "{bad}");
    }
    let missing =
        json!({"Command": "ChangeMessageVisibility", "QueueName": "q", "ReceiptHandle": "r-1"});
    assert_eq!(s.err(missing), "MissingParameter");
    assert_eq!(s.delete_message("q", "r-1"), json!({"ok": true}));
}

#[test]
fn a_delayed_message_has_no_usable_handle() {
    let mut s = Session::new();
    s.create("q");
    s.send_message("q", "x");
    s.receive("q");
    s.ok(json!({"Command": "PurgeQueue", "QueueName": "q"}));
    s.ok(json!({"Command": "SendMessage", "QueueName": "q", "Body": "y", "DelaySeconds": 50}));
    assert_eq!(
        s.delete_message("q", "r-1")["error"]["code"],
        json!("ReceiptHandleExpired")
    );
}

#[test]
fn retention_removes_messages_in_every_state_at_the_moment_it_runs_out() {
    let mut s = Session::new();
    s.create_with("q", json!({"RetentionSeconds": 60}));
    s.send_message("q", "visible");
    s.send_message("q", "in flight");
    s.ok(
        json!({"Command": "SendMessage", "QueueName": "q", "Body": "delayed", "DelaySeconds": 900}),
    );
    s.ok(json!({"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 1000}));
    s.tick(59);
    assert_eq!(s.counts("q"), counts(1, 1, 1));
    s.tick(1);
    assert_eq!(s.counts("q"), counts(0, 0, 0));
    assert_eq!(
        s.delete_message("q", "r-1")["error"]["code"],
        json!("ReceiptHandleExpired")
    );
}

#[test]
fn lowering_retention_removes_messages_at_once_and_raising_it_does_not_restore_them() {
    let mut s = Session::new();
    s.create("q");
    s.send_message("q", "old");
    s.tick(100);
    s.send_message("q", "new");
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 60}}));
    assert_eq!(s.counts("q"), counts(1, 0, 0));
    s.ok(json!({"Command": "SetQueueAttributes", "QueueName": "q", "Attributes": {"RetentionSeconds": 1_209_600}}));
    assert_eq!(s.counts("q"), counts(1, 0, 0));
    assert_eq!(delivered(&s.receive("q")), pairs(&[("m-2", "r-1")]));
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut s = Session::new();
    s.create("q");
    for n in 0..100_000 {
        let response = s.send(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"}));
        assert_eq!(response["MessageId"], json!(format!("m-{}", n + 1)));
    }
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    s.receive("q");
    assert_eq!(
        s.err(json!({"Command": "SendMessage", "QueueName": "q", "Body": "x"})),
        "LimitExceeded"
    );
    s.delete_message("q", "r-1");
    assert_eq!(s.send_message("q", "x")["MessageId"], json!("m-100001"));
}
