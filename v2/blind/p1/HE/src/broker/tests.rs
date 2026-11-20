use super::*;

type Expected = Result<&'static str, ErrorCode>;

const OK: Expected = Ok(r#"{"ok":true}"#);
const NO_MESSAGES: Expected = Ok(r#"{"ok":true,"Messages":[]}"#);

fn run(broker: &mut Broker, line: &str) -> Result<String, ErrorCode> {
    let request = Request::from_line(line.as_bytes()).map_err(|e| e.code())?;
    let command = request.command().map_err(|e| e.code())?;
    let success = broker.execute(command).map_err(|e| e.code())?;
    Ok(render(&Ok(success)).expect("a success response serializes"))
}

/// Runs the commands on one fresh broker and compares every response.
fn play(steps: &[(&str, Expected)]) {
    let mut broker = Broker::default();
    for (line, expected) in steps {
        let got = run(&mut broker, line);
        assert_eq!(got.as_deref(), expected.as_ref().map(|s| *s), "{line}");
    }
}

#[test]
fn message_lifecycle_with_visibility_timeout() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, OK),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"a","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            NO_MESSAGES,
        ),
        (
            r#"{"Command":"Tick","Seconds":29}"#,
            Ok(r#"{"ok":true,"Now":29}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            NO_MESSAGES,
        ),
        (
            r#"{"Command":"Tick","Seconds":1}"#,
            Ok(r#"{"ok":true,"Now":30}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-2","Body":"a","ReceiveCount":2,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            Err(ErrorCode::ReceiptHandleExpired),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
            OK,
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-2"}"#,
            Err(ErrorCode::ReceiptHandleExpired),
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            Ok(
                r#"{"ok":true,"Attributes":{"VisibilityTimeout":30,"DelaySeconds":0,"RetentionSeconds":345600,"Fifo":false,"ContentBasedDeduplication":false},"Counts":{"Visible":0,"InFlight":0,"Delayed":0}}"#,
            ),
        ),
    ]);
}

#[test]
fn delays_and_changed_visibility_keep_arrival_order() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"d","Attributes":{"DelaySeconds":5}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"d","Body":"x"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"d","Body":"y","DelaySeconds":0}"#,
            Ok(r#"{"ok":true,"MessageId":"m-2"}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"d","MaxMessages":10}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-2","ReceiptHandle":"r-1","Body":"y","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"Tick","Seconds":5}"#,
            Ok(r#"{"ok":true,"Now":5}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"d"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-2","Body":"x","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":5}]}"#,
            ),
        ),
        (
            r#"{"Command":"ChangeMessageVisibility","QueueName":"d","ReceiptHandle":"r-2","VisibilityTimeout":0}"#,
            OK,
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"d"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-3","Body":"x","ReceiveCount":2,"SentAt":0,"FirstReceivedAt":5}]}"#,
            ),
        ),
        (
            r#"{"Command":"ChangeMessageVisibility","QueueName":"d","ReceiptHandle":"r-2","VisibilityTimeout":9}"#,
            Err(ErrorCode::ReceiptHandleExpired),
        ),
        (
            r#"{"Command":"ChangeMessageVisibility","QueueName":"d","ReceiptHandle":"r-3","VisibilityTimeout":10}"#,
            OK,
        ),
        (
            r#"{"Command":"Tick","Seconds":9}"#,
            Ok(r#"{"ok":true,"Now":14}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"d"}"#,
            NO_MESSAGES,
        ),
        (
            r#"{"Command":"Tick","Seconds":1}"#,
            Ok(r#"{"ok":true,"Now":15}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"d"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-4","Body":"x","ReceiveCount":3,"SentAt":0,"FirstReceivedAt":5}]}"#,
            ),
        ),
    ]);
}

#[test]
fn receive_limits_and_overrides_visibility() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, OK),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"1"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"2"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-2"}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"3"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-3"}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":2,"VisibilityTimeout":5}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"1","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0},{"MessageId":"m-2","ReceiptHandle":"r-2","Body":"2","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"Tick","Seconds":5}"#,
            Ok(r#"{"ok":true,"Now":5}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":10}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-3","Body":"1","ReceiveCount":2,"SentAt":0,"FirstReceivedAt":0},{"MessageId":"m-2","ReceiptHandle":"r-4","Body":"2","ReceiveCount":2,"SentAt":0,"FirstReceivedAt":0},{"MessageId":"m-3","ReceiptHandle":"r-5","Body":"3","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":5}]}"#,
            ),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":0}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","MaxMessages":11}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
    ]);
}

#[test]
fn zero_visibility_timeout_never_leaves_a_usable_handle() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, OK),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q","VisibilityTimeout":0}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"a","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            Err(ErrorCode::ReceiptHandleExpired),
        ),
    ]);
}

#[test]
fn receipt_handles_are_invalid_unless_issued_and_expired_across_queues() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, OK),
        (r#"{"Command":"CreateQueue","QueueName":"other"}"#, OK),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            Err(ErrorCode::ReceiptHandleInvalid),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"a","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-01"}"#,
            Err(ErrorCode::ReceiptHandleInvalid),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"R-1"}"#,
            Err(ErrorCode::ReceiptHandleInvalid),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1 "}"#,
            Err(ErrorCode::ReceiptHandleInvalid),
        ),
        (
            r#"{"Command":"ChangeMessageVisibility","QueueName":"q","ReceiptHandle":"r-2","VisibilityTimeout":1}"#,
            Err(ErrorCode::ReceiptHandleInvalid),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"other","ReceiptHandle":"r-1"}"#,
            Err(ErrorCode::ReceiptHandleExpired),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"missing","ReceiptHandle":"nonsense"}"#,
            Err(ErrorCode::QueueDoesNotExist),
        ),
    ]);
}

#[test]
fn fifo_sends_check_their_rules_in_order() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            OK,
        ),
        (r#"{"Command":"CreateQueue","QueueName":"s"}"#, OK),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","DelaySeconds":1}"#,
            Err(ErrorCode::UnsupportedOperation),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a"}"#,
            Err(ErrorCode::MissingParameter),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g"}"#,
            Err(ErrorCode::MissingParameter),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"s","Body":"a","GroupId":"g"}"#,
            Err(ErrorCode::UnsupportedOperation),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"s","Body":"a","DeduplicationId":"d"}"#,
            Err(ErrorCode::UnsupportedOperation),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
    ]);
}

#[test]
fn fifo_delivers_one_message_per_group_and_blocks_behind_unfinished_ones() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g1","DeduplicationId":"d1"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b","GroupId":"g1","DeduplicationId":"d2"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-2","SequenceNumber":2}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"c","GroupId":"g2","DeduplicationId":"d3"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-3","SequenceNumber":3}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"a","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0,"GroupId":"g1","SequenceNumber":1},{"MessageId":"m-3","ReceiptHandle":"r-2","Body":"c","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0,"GroupId":"g2","SequenceNumber":3}]}"#,
            ),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
            NO_MESSAGES,
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#,
            OK,
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo","MaxMessages":10}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-2","ReceiptHandle":"r-3","Body":"b","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0,"GroupId":"g1","SequenceNumber":2}]}"#,
            ),
        ),
    ]);
}

#[test]
fn fifo_deduplication_window_is_fixed_and_outlives_the_message() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (
            r#"{"Command":"Tick","Seconds":299}"#,
            Ok(r#"{"ok":true,"Now":299}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"f.fifo"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"a","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":299,"GroupId":"g","SequenceNumber":1}]}"#,
            ),
        ),
        (
            r#"{"Command":"DeleteMessage","QueueName":"f.fifo","ReceiptHandle":"r-1"}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"other","GroupId":"h","DeduplicationId":"d"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (
            r#"{"Command":"Tick","Seconds":1}"#,
            Ok(r#"{"ok":true,"Now":300}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-2","SequenceNumber":2}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-2","SequenceNumber":2}"#),
        ),
    ]);
}

#[test]
fn content_based_deduplication_uses_the_body_as_id() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"c.fifo","Attributes":{"Fifo":true,"ContentBasedDeduplication":true}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"same","GroupId":"g"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"same","GroupId":"h"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"x","GroupId":"g","DeduplicationId":"same"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"c.fifo","Body":"different","GroupId":"g"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-2","SequenceNumber":2}"#),
        ),
    ]);
}

#[test]
fn purge_keeps_sequence_numbers_and_deduplication() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d1"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (r#"{"Command":"PurgeQueue","QueueName":"f.fifo"}"#, OK),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"a","GroupId":"g","DeduplicationId":"d1"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"f.fifo","Body":"b","GroupId":"g","DeduplicationId":"d2"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-2","SequenceNumber":2}"#),
        ),
    ]);
}

#[test]
fn messages_over_the_receive_count_move_to_the_dead_letter_queue() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"dlq"}"#, OK),
        (
            r#"{"Command":"CreateQueue","QueueName":"src","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dlq","MaxReceiveCount":2}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"src","Body":"x"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"src"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"x","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"Tick","Seconds":7}"#,
            Ok(r#"{"ok":true,"Now":7}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"src"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-2","Body":"x","ReceiveCount":2,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"src"}"#,
            NO_MESSAGES,
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"src"}"#,
            Ok(
                r#"{"ok":true,"Attributes":{"VisibilityTimeout":0,"DelaySeconds":0,"RetentionSeconds":345600,"Fifo":false,"ContentBasedDeduplication":false,"DeadLetterQueue":"dlq","MaxReceiveCount":2},"Counts":{"Visible":0,"InFlight":0,"Delayed":0}}"#,
            ),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"dlq"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-3","Body":"x","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":7}]}"#,
            ),
        ),
        (r#"{"Command":"DeleteQueue","QueueName":"dlq"}"#, OK),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"src"}"#,
            Ok(
                r#"{"ok":true,"Attributes":{"VisibilityTimeout":0,"DelaySeconds":0,"RetentionSeconds":345600,"Fifo":false,"ContentBasedDeduplication":false},"Counts":{"Visible":0,"InFlight":0,"Delayed":0}}"#,
            ),
        ),
    ]);
}

#[test]
fn a_dead_lettered_fifo_message_frees_its_group_within_the_same_receive() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"dl.fifo","Attributes":{"Fifo":true}}"#,
            OK,
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"src.fifo","Attributes":{"Fifo":true,"VisibilityTimeout":0,"DeadLetterQueue":"dl.fifo","MaxReceiveCount":1}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"src.fifo","Body":"a","GroupId":"g","DeduplicationId":"1"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1","SequenceNumber":1}"#),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"src.fifo","Body":"b","GroupId":"g","DeduplicationId":"2"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-2","SequenceNumber":2}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"src.fifo"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"a","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0,"GroupId":"g","SequenceNumber":1}]}"#,
            ),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"src.fifo"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-2","ReceiptHandle":"r-2","Body":"b","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0,"GroupId":"g","SequenceNumber":2}]}"#,
            ),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"dl.fifo"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-3","Body":"a","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0,"GroupId":"g","SequenceNumber":1}]}"#,
            ),
        ),
    ]);
}

#[test]
fn a_message_too_old_for_the_dead_letter_queue_is_removed_on_arrival() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"dlq","Attributes":{"RetentionSeconds":60}}"#,
            OK,
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"src","Attributes":{"VisibilityTimeout":0,"DeadLetterQueue":"dlq","MaxReceiveCount":1}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"src","Body":"x"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"src"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"x","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (
            r#"{"Command":"Tick","Seconds":100}"#,
            Ok(r#"{"ok":true,"Now":100}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"src"}"#,
            NO_MESSAGES,
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"dlq"}"#,
            Ok(
                r#"{"ok":true,"Attributes":{"VisibilityTimeout":30,"DelaySeconds":0,"RetentionSeconds":60,"Fifo":false,"ContentBasedDeduplication":false},"Counts":{"Visible":0,"InFlight":0,"Delayed":0}}"#,
            ),
        ),
    ]);
}

#[test]
fn retention_removes_messages_with_the_clock_and_with_attribute_changes() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"r","Attributes":{"RetentionSeconds":60}}"#,
            OK,
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"r","Body":"a"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"Tick","Seconds":59}"#,
            Ok(r#"{"ok":true,"Now":59}"#),
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"r"}"#,
            Ok(
                r#"{"ok":true,"Attributes":{"VisibilityTimeout":30,"DelaySeconds":0,"RetentionSeconds":60,"Fifo":false,"ContentBasedDeduplication":false},"Counts":{"Visible":1,"InFlight":0,"Delayed":0}}"#,
            ),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"r","Attributes":{"RetentionSeconds":60}}"#,
            OK,
        ),
        (
            r#"{"Command":"Tick","Seconds":1}"#,
            Ok(r#"{"ok":true,"Now":60}"#),
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"r"}"#,
            Ok(
                r#"{"ok":true,"Attributes":{"VisibilityTimeout":30,"DelaySeconds":0,"RetentionSeconds":60,"Fifo":false,"ContentBasedDeduplication":false},"Counts":{"Visible":0,"InFlight":0,"Delayed":0}}"#,
            ),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"r","Attributes":{"RetentionSeconds":1000}}"#,
            OK,
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"r"}"#,
            Ok(
                r#"{"ok":true,"Attributes":{"VisibilityTimeout":30,"DelaySeconds":0,"RetentionSeconds":1000,"Fifo":false,"ContentBasedDeduplication":false},"Counts":{"Visible":0,"InFlight":0,"Delayed":0}}"#,
            ),
        ),
    ]);
}

#[test]
fn lowering_retention_removes_old_messages_at_once() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"r"}"#, OK),
        (
            r#"{"Command":"SendMessage","QueueName":"r","Body":"a"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"Tick","Seconds":100}"#,
            Ok(r#"{"ok":true,"Now":100}"#),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"r","Attributes":{"RetentionSeconds":100}}"#,
            OK,
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"r"}"#,
            NO_MESSAGES,
        ),
    ]);
}

#[test]
fn checks_run_in_the_documented_order() {
    play(&[
        (
            r#"{"Command":"SendMessage"}"#,
            Err(ErrorCode::MissingParameter),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"nope","Body":""}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
        (
            r#"{"Command":"SendMessage","QueueName":"nope","Body":"x"}"#,
            Err(ErrorCode::QueueDoesNotExist),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":""}"#,
            Err(ErrorCode::QueueDoesNotExist),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"a.fifo"}"#,
            Err(ErrorCode::QueueNameInvalid),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"Fifo":true}}"#,
            Err(ErrorCode::QueueNameInvalid),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"ContentBasedDeduplication":true}}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"MaxReceiveCount":2}}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":{"MaxReceiveCount":2,"DeadLetterQueue":"missing"}}"#,
            Err(ErrorCode::QueueDoesNotExist),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"a","Attributes":null}"#,
            OK,
        ),
    ]);
}

#[test]
fn creating_an_existing_queue_is_idempotent_only_for_equal_attributes() {
    play(&[
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":3}}"#,
            OK,
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":3,"VisibilityTimeout":30,"Fifo":false}}"#,
            OK,
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q"}"#,
            Err(ErrorCode::QueueAlreadyExists),
        ),
        (
            r#"{"Command":"CreateQueue","QueueName":"q","Attributes":{"DelaySeconds":3,"MaxReceiveCount":1,"DeadLetterQueue":"q"}}"#,
            Err(ErrorCode::QueueAlreadyExists),
        ),
    ]);
}

#[test]
fn dead_letter_configuration_rules_for_set_queue_attributes() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, OK),
        (r#"{"Command":"CreateQueue","QueueName":"d"}"#, OK),
        (
            r#"{"Command":"CreateQueue","QueueName":"f.fifo","Attributes":{"Fifo":true}}"#,
            OK,
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"Fifo":false}}"#,
            Err(ErrorCode::UnsupportedOperation),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"ContentBasedDeduplication":true}}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"d"}}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"zzz","MaxReceiveCount":1}}"#,
            Err(ErrorCode::QueueDoesNotExist),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"q","MaxReceiveCount":1}}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"f.fifo","MaxReceiveCount":1}}"#,
            Err(ErrorCode::InvalidParameterValue),
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{"DeadLetterQueue":"d","MaxReceiveCount":4,"DelaySeconds":2}}"#,
            OK,
        ),
        (
            r#"{"Command":"SetQueueAttributes","QueueName":"q","Attributes":{}}"#,
            OK,
        ),
        (
            r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
            Ok(
                r#"{"ok":true,"Attributes":{"VisibilityTimeout":30,"DelaySeconds":2,"RetentionSeconds":345600,"Fifo":false,"ContentBasedDeduplication":false,"DeadLetterQueue":"d","MaxReceiveCount":4},"Counts":{"Visible":0,"InFlight":0,"Delayed":0}}"#,
            ),
        ),
    ]);
}

#[test]
fn list_queues_filters_by_prefix_in_byte_order() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"ab"}"#, OK),
        (r#"{"Command":"CreateQueue","QueueName":"a_"}"#, OK),
        (r#"{"Command":"CreateQueue","QueueName":"aB"}"#, OK),
        (r#"{"Command":"CreateQueue","QueueName":"a1"}"#, OK),
        (
            r#"{"Command":"CreateQueue","QueueName":"a.fifo","Attributes":{"Fifo":true}}"#,
            OK,
        ),
        (r#"{"Command":"CreateQueue","QueueName":"a-1"}"#, OK),
        (r#"{"Command":"CreateQueue","QueueName":"b"}"#, OK),
        (
            r#"{"Command":"ListQueues","Prefix":"a"}"#,
            Ok(r#"{"ok":true,"QueueNames":["a-1","a.fifo","a1","aB","a_","ab"]}"#),
        ),
        (
            r#"{"Command":"ListQueues"}"#,
            Ok(r#"{"ok":true,"QueueNames":["a-1","a.fifo","a1","aB","a_","ab","b"]}"#),
        ),
        (
            r#"{"Command":"ListQueues","Prefix":"a1"}"#,
            Ok(r#"{"ok":true,"QueueNames":["a1"]}"#),
        ),
        (
            r#"{"Command":"ListQueues","Prefix":"zz"}"#,
            Ok(r#"{"ok":true,"QueueNames":[]}"#),
        ),
    ]);
}

#[test]
fn a_recreated_queue_shares_nothing_with_the_deleted_one() {
    play(&[
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, OK),
        (
            r#"{"Command":"SendMessage","QueueName":"q","Body":"a"}"#,
            Ok(r#"{"ok":true,"MessageId":"m-1"}"#),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            Ok(
                r#"{"ok":true,"Messages":[{"MessageId":"m-1","ReceiptHandle":"r-1","Body":"a","ReceiveCount":1,"SentAt":0,"FirstReceivedAt":0}]}"#,
            ),
        ),
        (r#"{"Command":"DeleteQueue","QueueName":"q"}"#, OK),
        (
            r#"{"Command":"DeleteQueue","QueueName":"q"}"#,
            Err(ErrorCode::QueueDoesNotExist),
        ),
        (r#"{"Command":"CreateQueue","QueueName":"q"}"#, OK),
        (
            r#"{"Command":"DeleteMessage","QueueName":"q","ReceiptHandle":"r-1"}"#,
            Err(ErrorCode::ReceiptHandleExpired),
        ),
        (
            r#"{"Command":"ReceiveMessage","QueueName":"q"}"#,
            NO_MESSAGES,
        ),
    ]);
}

#[test]
fn at_most_1000_queues_exist() {
    let mut broker = Broker::default();
    for n in 0..1000 {
        let line = format!(r#"{{"Command":"CreateQueue","QueueName":"q{n}"}}"#);
        assert_eq!(run(&mut broker, &line).as_deref(), Ok(r#"{"ok":true}"#));
    }
    let extra = r#"{"Command":"CreateQueue","QueueName":"extra"}"#;
    assert_eq!(run(&mut broker, extra), Err(ErrorCode::LimitExceeded));
    let existing = r#"{"Command":"CreateQueue","QueueName":"q0"}"#;
    assert_eq!(run(&mut broker, existing).as_deref(), Ok(r#"{"ok":true}"#));
}

#[test]
fn a_queue_holds_at_most_100000_messages() {
    let mut broker = Broker::default();
    run(&mut broker, r#"{"Command":"CreateQueue","QueueName":"q"}"#).expect("queue is created");
    let send = r#"{"Command":"SendMessage","QueueName":"q","Body":"b"}"#;
    for _ in 0..100_000 {
        run(&mut broker, send).expect("the queue has room");
    }
    assert_eq!(run(&mut broker, send), Err(ErrorCode::LimitExceeded));
    let counts = run(
        &mut broker,
        r#"{"Command":"GetQueueAttributes","QueueName":"q"}"#,
    );
    assert_eq!(
        counts.as_deref(),
        Ok(
            r#"{"ok":true,"Attributes":{"VisibilityTimeout":30,"DelaySeconds":0,"RetentionSeconds":345600,"Fifo":false,"ContentBasedDeduplication":false},"Counts":{"Visible":100000,"InFlight":0,"Delayed":0}}"#
        )
    );
}
