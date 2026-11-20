# miniq specification

miniq is a small in-memory message queue service. Build it from scratch as a Rust crate whose binary is
named `miniq`, and include tests. It uses no network, no files and no wall clock: all state lives in the
process and time is virtual (section 3).

## 1. Protocol

- miniq reads stdin as lines ending in `\n`; a final line without `\n` counts if it is not empty. Every
  line, blank ones included, is one command and gets exactly one response: one JSON object on a single
  line of stdout, ending in `\n`. Responses come in input order, nothing else goes to stdout, and each
  response is written and flushed before the next line is read. At end of input miniq exits with 0.
- A command is a line that is, as a whole, one JSON object with a string member `Command` and the
  command's parameters as further members, for example `{"Command": "PurgeQueue", "QueueName": "jobs"}`.
  All names are case-sensitive. Members a command does not define are ignored. A parameter or attribute
  whose value is `null` is treated as absent. An object that repeats a member name: unspecified behavior.
- Success response: `{"ok": true}` plus exactly the result members listed for the command.
- Failure response: `{"ok": false, "error": {"code": "...", "message": "..."}}`. `code` is one of the
  codes in section 2; `message` is a human-readable string whose text is up to you. The `error` object
  may carry extra members; every other object in a response has exactly the members this document lists.
  A failed command changes no state.
- Parameter types: a *string* is a JSON string; a *boolean* is `true` or `false`; an *integer* is a JSON
  number written without fraction or exponent (`30`, but not `30.0`, `3e1` or `"30"`). Ranges are
  inclusive. Every number in a response is an integer. Member order and whitespace inside a response
  line are free.

## 2. Errors

| Code | Meaning |
|---|---|
| `InvalidRequest` | The line is not valid UTF-8 or not a JSON object, has no string `Command`, or names an unknown command |
| `MissingParameter` | A required parameter is absent |
| `InvalidParameterValue` | A parameter or attribute has the wrong type or a value that is not allowed |
| `QueueNameInvalid` | `CreateQueue` was given a name that breaks the naming rules |
| `QueueAlreadyExists` | `CreateQueue` named an existing queue with different attributes |
| `QueueDoesNotExist` | The named queue, or the named dead-letter queue, does not exist |
| `ReceiptHandleInvalid` | The receipt handle was never issued |
| `ReceiptHandleExpired` | The receipt handle was issued but is not usable (any more) |
| `UnsupportedOperation` | The request asks for something this type of queue does not support |
| `LimitExceeded` | A limit is reached: 1000 queues, or 100000 messages in one queue |

No other code is ever returned. When several checks fail, the first one in this order decides the code:

1. Request shape: `InvalidRequest`. miniq then carries on with the next line.
2. Required parameters: a parameter marked *required* is absent: `MissingParameter`.
3. Parameter values, each on its own: a present parameter has the wrong JSON type or breaks its stated
   range, length or character rule: `InvalidParameterValue`. This covers `Attributes` not being an
   object, each attribute inside it, and attribute names that section 4 does not list. In `CreateQueue`
   only, `QueueName` is checked before every other parameter, and a string that breaks the name syntax
   of section 4 gives `QueueNameInvalid`.
4. Queue lookup, for every command that has a `QueueName` except `CreateQueue`: no queue has exactly
   this name: `QueueDoesNotExist`, whatever the string looks like.
5. The numbered rules in the command's own section, in the order written.

## 3. Time and identifiers

- The clock is an integer number of seconds. It starts at 0 and only `Tick` advances it; every other
  command takes no time. `now` is its current value. Behavior is unspecified past 4000000000.
- `MessageId` is `m-<n>`: `n` starts at 1 and counts the messages accepted by the process, over all
  queues. A failed send or a duplicate send (section 6) does not use up a number.
- `ReceiptHandle` is `r-<n>`: `n` starts at 1 and counts the deliveries made by the process, over all
  queues, in delivery order.
- `SequenceNumber` is an integer kept per FIFO queue: it starts at 1 and counts the messages added to
  that queue.

## 4. Queues and messages

A queue has a unique name, the attributes below, and its messages in *arrival order*: the order in which
they were added to this queue. It is a *FIFO queue* if `Fifo` is true and a *standard queue* otherwise.

| Attribute | Type | Allowed values | Default |
|---|---|---|---|
| `VisibilityTimeout` | integer | 0 to 43200 | 30 |
| `DelaySeconds` | integer | 0 to 900 | 0 |
| `RetentionSeconds` | integer | 60 to 1209600 | 345600 |
| `Fifo` | boolean | | false |
| `ContentBasedDeduplication` | boolean | | false |
| `DeadLetterQueue` | string | any string (see the command rules) | unset |
| `MaxReceiveCount` | integer | 1 to 1000 | unset |

`DeadLetterQueue` and `MaxReceiveCount` together are the queue's *dead-letter configuration*: both are
set or both are unset.

Name syntax: a queue name is 1 to 80 characters long. It is either made only of the characters
`A-Z a-z 0-9 _ -`, or made of 1 to 75 such characters followed by the suffix `.fifo`.

A message has a `MessageId`, a `Body`, `SentAt` (the clock value when it was sent), a `ReceiveCount` (0
at first), `FirstReceivedAt` (unset until its first delivery) and, in a FIFO queue, a `GroupId` and a
`SequenceNumber`. At any moment it is in exactly one state:

- *delayed*: it was sent at time `t` with delay `d`, and `now < t + d`;
- *in flight*: its latest delivery was at time `t` with visibility timeout `v`, and `now < t + v`;
- *visible*: otherwise.

Retention: a message is removed for good as soon as `now >= SentAt + RetentionSeconds` of the queue that
holds it, whatever its state. This holds at every moment, so lowering `RetentionSeconds` can remove
messages at once, and raising it later does not bring them back.

## 5. Queue commands

**CreateQueue.** Parameters: `QueueName` (string, required); `Attributes` (object, optional: any
attributes of section 4; the ones left out take their defaults).

1. Exactly one of these two holds: the name ends in `.fifo`; `Fifo` is true: `QueueNameInvalid`.
2. `ContentBasedDeduplication` is true and `Fifo` is false: `InvalidParameterValue`.
3. Exactly one of `DeadLetterQueue` and `MaxReceiveCount` is present: `InvalidParameterValue`.
4. A queue with this name exists: if every attribute of the request, after defaults, equals the queue's
   current value (an unset dead-letter configuration on both sides is equal), the command succeeds and
   changes nothing; otherwise `QueueAlreadyExists`.
5. `DeadLetterQueue` is not the name of an existing queue: `QueueDoesNotExist`.
6. The dead-letter queue's `Fifo` differs from this queue's `Fifo`: `InvalidParameterValue`.
7. 1000 queues already exist: `LimitExceeded`.

Otherwise the queue is created, empty. Result members: none.

**SetQueueAttributes.** Parameters: `QueueName` (string, required); `Attributes` (object, required, may
be empty).

1. `Fifo` is present: `UnsupportedOperation` (the type of a queue never changes).
2. `ContentBasedDeduplication` is true and the queue is a standard queue: `InvalidParameterValue`.
3. Exactly one of `DeadLetterQueue` and `MaxReceiveCount` is present: `InvalidParameterValue`.
4. `DeadLetterQueue` is not the name of an existing queue: `QueueDoesNotExist`.
5. `DeadLetterQueue` is this queue itself, or a queue with a different `Fifo`: `InvalidParameterValue`.

Otherwise the attributes present take the given values and the others keep theirs. A dead-letter
configuration can be replaced this way but not removed. A new `VisibilityTimeout` applies to later
deliveries only and a new `DelaySeconds` to later sends only; a new `RetentionSeconds` applies to all
messages at once, a new dead-letter configuration from the next `ReceiveMessage` on. Result members: none.

**GetQueueAttributes.** Parameters: `QueueName` (string, required). Result: `Attributes`, an object with
the first five attributes of section 4 always, and `DeadLetterQueue` and `MaxReceiveCount` only when set
(never `null`); and `Counts`, an object `{"Visible": n, "InFlight": n, "Delayed": n}` with the number of
messages in each state.

**ListQueues.** Parameters: `Prefix` (string, optional). Result: `QueueNames`, an array of the names of
all queues whose name starts with `Prefix` (all queues if absent), in ascending order of byte values (so
`-` < `.` < digits < uppercase letters < `_` < lowercase letters).

**DeleteQueue.** Parameters: `QueueName` (string, required). The queue and its messages are removed.
Every queue whose `DeadLetterQueue` is the deleted queue loses its dead-letter configuration (both
attributes become unset). A queue created later under the same name is a new queue that shares nothing
with the deleted one. Result members: none.

**PurgeQueue.** Parameters: `QueueName` (string, required). Every message of the queue is removed,
whatever its state; sequence numbers and deduplication (section 6) carry on. Result members: none.

**Tick.** Parameters: `Seconds` (integer, required, 0 to 1209600). The clock advances by `Seconds`.
Result: `Now`, the new clock value.

## 6. Message commands

**SendMessage.** Parameters: `QueueName` (string, required); `Body` (string, required, 1 to 1024 bytes
when encoded as UTF-8); `DelaySeconds` (integer, optional, 0 to 900); `GroupId` and `DeduplicationId`
(string, optional, 1 to 128 characters from `A-Z a-z 0-9 _ -`).

1. Standard queue and `GroupId` or `DeduplicationId` is present, or FIFO queue and `DelaySeconds` is
   present: `UnsupportedOperation`.
2. FIFO queue and `GroupId` is absent: `MissingParameter`.
3. FIFO queue, `DeduplicationId` is absent and `ContentBasedDeduplication` is false: `MissingParameter`.
4. FIFO queue and the send is a duplicate (below): the command succeeds with the `MessageId` and
   `SequenceNumber` of the original message, and nothing else happens.
5. The queue already holds 100000 messages, counting every state: `LimitExceeded`.

Otherwise the message is accepted: it gets the next `MessageId`, `SentAt = now`, and is added at the end
of the queue. Its delay is the `DelaySeconds` parameter if present, else the queue's `DelaySeconds` at
this moment. Result: `MessageId`, and on a FIFO queue also `SequenceNumber`. Filling a queue to its
limit with one `SendMessage` per line must take seconds, not minutes.

Deduplication: the deduplication id of a FIFO send is `DeduplicationId` if present, otherwise the body
itself (of any length and content). A send is a *duplicate* if the same queue accepted a message with
the same deduplication id at a time `t` with `now - t < 300`. A duplicate does not extend that window:
from `t + 300` on, the next send with this id is accepted and opens a new window. Deduplication ids are
per queue, ignore `GroupId`, and outlive the original message: deleting it, or it leaving the queue in
any other way, does not free the id.

**ReceiveMessage.** Parameters: `QueueName` (string, required); `MaxMessages` (integer, optional, 1 to
10, default 1); `VisibilityTimeout` (integer, optional, 0 to 43200, default: the queue's attribute).

The messages of the queue are examined once each, in arrival order, until `MaxMessages` messages have
been delivered or the end of the queue is reached. A message is *eligible* if it is visible and, in a
FIFO queue, no earlier message with the same `GroupId` is still in the queue (in any state). Messages
that are not eligible are skipped. For each eligible message:

- if the queue has a dead-letter configuration and the message's `ReceiveCount` is at least
  `MaxReceiveCount`, the message is moved to the dead-letter queue (below) instead of being delivered.
  It does not count toward `MaxMessages`, and the examination goes on;
- otherwise it is delivered: `ReceiveCount` grows by 1, `FirstReceivedAt` becomes `now` if it was unset,
  the message gets the next `ReceiptHandle`, and with visibility timeout `v` it is in flight while
  `now < (time of this delivery) + v`.

Result: `Messages`, an array (possibly empty) in delivery order of objects with `MessageId`,
`ReceiptHandle`, `Body`, `ReceiveCount`, `SentAt`, `FirstReceivedAt` and, on a FIFO queue, `GroupId` and
`SequenceNumber`.

Consequences: a message whose visibility timeout ends keeps its place in arrival order. With a
visibility timeout of 0 the message is visible again at once and its handle is never usable. One
`ReceiveMessage` on a FIFO queue delivers at most one message per group, and none from a group whose
earliest message is in flight or delayed.

Dead-letter move: the message leaves its queue and is added at the end of the dead-letter queue. It
keeps `MessageId`, `Body`, `SentAt` and `GroupId`. Its `ReceiveCount` is 0 again, `FirstReceivedAt` is
unset, it has no delay (it is visible at once), and in a FIFO queue it takes the next `SequenceNumber`
of the dead-letter queue. The move neither checks nor records deduplication ids and ignores the message
limit. Retention keeps counting from `SentAt`, now under the dead-letter queue's `RetentionSeconds`, so
a message that is already too old there is removed on arrival.

**DeleteMessage** and **ChangeMessageVisibility.** Parameters of both: `QueueName` and `ReceiptHandle`
(strings, required). `ChangeMessageVisibility` also takes `VisibilityTimeout` (integer, required, 0 to
43200).

1. `ReceiptHandle` is not a handle that this process has issued so far: `ReceiptHandleInvalid`. Handles
   are compared as exact strings: `r-01`, `R-1` and `r-1 ` were never issued.
2. The handle is not the latest handle of a message that is in flight in this queue right now:
   `ReceiptHandleExpired`. This covers a message delivered again since, a visibility timeout that has
   ended, a message that is gone (deleted, purged, expired, moved to a dead-letter queue), and a handle
   issued for a message of another queue.

`DeleteMessage` removes the message. `ChangeMessageVisibility` replaces the end of the message's
visibility timeout: it is now in flight while `now < (time of this command) + VisibilityTimeout`, so 0
makes it visible at once. Neither changes `ReceiveCount`. Result members: none.
