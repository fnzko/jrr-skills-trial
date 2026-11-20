use serde_json::{Value, json};

/// Feeds `input` to the service and returns each response line parsed, with
/// the free-text error `message` checked to be a string and then removed so
/// that responses can be compared as whole values.
pub fn responses(input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    miniq::run(input, &mut output).expect("in-memory I/O does not fail");
    let text = String::from_utf8(output).expect("responses are UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "every response ends in a newline: {text:?}"
    );
    text.lines()
        .map(|line| {
            let mut response: Value =
                serde_json::from_str(line).expect("each response line is JSON");
            if let Some(error) = response.get_mut("error").and_then(Value::as_object_mut) {
                let message = error.remove("message");
                assert!(
                    matches!(message, Some(Value::String(_))),
                    "error message in {line}"
                );
            }
            response
        })
        .collect()
}

/// Runs the requests in order and checks that each got the paired response.
pub fn check(session: &[(Value, Value)]) {
    let input: String = session
        .iter()
        .map(|(request, _)| format!("{request}\n"))
        .collect();
    let expected: Vec<&Value> = session.iter().map(|(_, response)| response).collect();
    let actual = responses(input.as_bytes());
    assert_eq!(actual.iter().collect::<Vec<_>>(), expected);
}

pub fn ok() -> Value {
    json!({"ok": true})
}

pub fn fail(code: &str) -> Value {
    json!({"ok": false, "error": {"code": code}})
}

pub fn create(name: &str) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name})
}

pub fn create_with(name: &str, attributes: Value) -> Value {
    json!({"Command": "CreateQueue", "QueueName": name, "Attributes": attributes})
}

pub fn send(queue: &str, body: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body})
}

pub fn send_fifo(queue: &str, body: &str, group: &str, dedup: &str) -> Value {
    json!({"Command": "SendMessage", "QueueName": queue, "Body": body, "GroupId": group, "DeduplicationId": dedup})
}

pub fn receive(queue: &str, max: u32) -> Value {
    json!({"Command": "ReceiveMessage", "QueueName": queue, "MaxMessages": max})
}

pub fn tick(seconds: u32) -> Value {
    json!({"Command": "Tick", "Seconds": seconds})
}

pub fn delete_message(queue: &str, handle: &str) -> Value {
    json!({"Command": "DeleteMessage", "QueueName": queue, "ReceiptHandle": handle})
}

pub fn get_attributes(queue: &str) -> Value {
    json!({"Command": "GetQueueAttributes", "QueueName": queue})
}

pub fn sent(id: u32) -> Value {
    json!({"ok": true, "MessageId": format!("m-{id}")})
}

pub fn sent_fifo(id: u32, sequence: u32) -> Value {
    json!({"ok": true, "MessageId": format!("m-{id}"), "SequenceNumber": sequence})
}

pub fn messages(items: Value) -> Value {
    json!({"ok": true, "Messages": items})
}

pub fn counts(visible: u32, in_flight: u32, delayed: u32) -> Value {
    json!({"Visible": visible, "InFlight": in_flight, "Delayed": delayed})
}
