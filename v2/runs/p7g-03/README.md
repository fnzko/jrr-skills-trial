# miniq

A small in-memory message queue driven by JSON commands on stdin, one
response line per command on stdout. See `SPEC.md` for the full behaviour.

    cargo run --release < commands.txt
    cargo test

## Layout

- `src/main.rs`: the line loop (read a line, answer it, flush).
- `src/service.rs`: request parsing, dispatch, response shaping, shared state.
- `src/queue_commands.rs`, `src/message_commands.rs`: the commands.
- `src/queue.rs`: one queue: messages in arrival order, retention and
  deduplication indexes, and `plan_receive`, the `ReceiveMessage` decision.
- `src/attributes.rs`: queue attributes, their parsing and the name syntax.
- `src/params.rs`: typed access to command members (`null` is absent).
- `tests/`: end-to-end tests through `Service` (`queues.rs`, `messages.rs`)
  and through the binary (`protocol.rs`).

## Design notes

- Time only moves on `Tick`, so retention is applied lazily: every command
  expires a queue's messages when it looks the queue up. This is
  indistinguishable from eager expiry, and it makes a send O(log n).
- Messages sit in a `BTreeMap` keyed by an ever-growing arrival key, with a
  `(SentAt, key)` set for expiry. Dead-letter moves break the SentAt order of
  the queue, which is why expiry uses the set rather than a prefix.
- Receipt handles are stored as `(queue uid, arrival key)`. A queue's uid is
  never reused, so handles of a deleted and re-created queue stay expired.
- `ReceiveMessage` scans the queue from the front, so a receive is O(n) in the
  number of messages before the last one it examines.
