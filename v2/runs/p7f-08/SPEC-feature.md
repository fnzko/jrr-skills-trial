# miniq feature request: batches, queue tags and a per-queue message size limit

This extends the miniq of `SPEC.md`. Everything in `SPEC.md` still holds unless this document says
otherwise: the protocol, the response shapes, the check order of its section 2, and all existing
commands. Add tests for the new behavior.

## 1. New error codes

| Code | Meaning |
|---|---|
| `EmptyBatchRequest` | `Entries` has no entry |
| `TooManyEntriesInBatchRequest` | `Entries` has more than 10 entries |
| `InvalidBatchEntryId` | An entry is not an object or has no valid `Id` |
| `BatchEntryIdsNotDistinct` | Two entries have the same `Id` |

## 2. SendMessageBatch and DeleteMessageBatch

Parameters of both: `QueueName` (string, required); `Entries` (array, required). An entry is an object
with an `Id` (string, 1 to 80 characters from `A-Z a-z 0-9 _ -`) and the parameters of the single
command other than `QueueName`: `Body`, `DelaySeconds`, `GroupId` and `DeduplicationId` for
`SendMessageBatch`, `ReceiptHandle` for `DeleteMessageBatch`. Other members are ignored, `null` is absent.

The numbered rules, checked after the queue lookup like any numbered rules:

1. `Entries` is empty: `EmptyBatchRequest`.
2. `Entries` has more than 10 elements: `TooManyEntriesInBatchRequest`.
3. An element is not an object, or its `Id` is absent, not a string or breaks the `Id` rule:
   `InvalidBatchEntryId`.
4. Two entries have the same `Id`: `BatchEntryIdsNotDistinct`.

A request that fails one of these (or an earlier check) is a failure response as usual and does nothing.
Otherwise the entries are handled one by one, in order. Each is handled exactly as `SendMessage` or
`DeleteMessage` would handle a command with the same `QueueName` and the entry's parameters, from
step 2 of the check order on, and it sees the effects of the entries before it (message ids, sequence
numbers, deduplication, the message limit, handles already used). An entry that fails changes nothing
and does not stop the others.

The response is a success, even when every entry failed. Result members, both always present:

- `Successful`: an array, in entry order, with one object per entry that succeeded: its `Id` plus the
  result members of the single command (`MessageId`, and `SequenceNumber` on a FIFO queue, for
  `SendMessageBatch`; nothing more for `DeleteMessageBatch`).
- `Failed`: an array, in entry order, with one object `{"Id": "...", "error": {...}}` per entry that
  failed, where `error` is an error object as in a failure response (`code` and `message`).

## 3. Queue tags

A queue carries tags: at most 50 pairs of a key and a value, none at first. A key is 1 to 128 characters
from `A-Z a-z 0-9 _ -`; keys are unique per queue and case-sensitive. A value is a string of 0 to 256
bytes when encoded as UTF-8.

**TagQueue.** Parameters: `QueueName` (string, required); `Tags` (object, required, may be empty: each
member name is a key and its value the tag value). A member name that breaks the key rule, or a value
that is not a string or is too long, is an `InvalidParameterValue` of step 3.

1. The queue would carry more than 50 tags after the command: `LimitExceeded`.

Otherwise every given tag is set, replacing the value of a key the queue has. Result members: none.

**UntagQueue.** Parameters: `QueueName` (string, required); `TagKeys` (array, required, may be empty;
an element that is not a string or breaks the key rule is an `InvalidParameterValue` of step 3). The
tags with these keys are removed; keys the queue does not have are ignored. Result members: none.

**ListQueueTags.** Parameters: `QueueName` (string, required). Result: `Tags`, an object with one
member per tag (empty if there are none).

Tags are not attributes: `GetQueueAttributes` does not show them, `CreateQueue` rule 4 ignores them,
and `PurgeQueue` leaves them alone. They disappear with `DeleteQueue`.

## 4. MaxMessageBytes

A new queue attribute, `MaxMessageBytes` (integer, 1 to 1024, default 1024). `CreateQueue` and
`SetQueueAttributes` accept it like any other attribute (it takes part in `CreateQueue` rule 4), and
`GetQueueAttributes` always returns it in `Attributes`.

`SendMessage` gets a new numbered rule that comes before its rule 1: the body is longer, in bytes when
encoded as UTF-8, than the queue's `MaxMessageBytes`: `InvalidParameterValue`. The rule applies to
`SendMessageBatch` entries too. The 1024-byte rule on `Body` stays where it is (step 3). Changing
`MaxMessageBytes` does not affect messages already in the queue, and dead-letter moves ignore it.
