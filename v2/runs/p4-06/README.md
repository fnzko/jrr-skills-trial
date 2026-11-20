# miniq

An in-memory message queue service, driven over stdin and stdout. The
behaviour is specified in [SPEC.md](SPEC.md); this file covers how to build,
run and test the crate.

## Run

```sh
cargo build --release
printf '%s\n' \
  '{"Command": "CreateQueue", "QueueName": "jobs"}' \
  '{"Command": "SendMessage", "QueueName": "jobs", "Body": "build"}' \
  '{"Command": "ReceiveMessage", "QueueName": "jobs"}' \
  | ./target/release/miniq
```

Each input line is one command and gets one response line:

```
{"ok":true}
{"MessageId":"m-1","ok":true}
{"Messages":[{"Body":"build",...,"ReceiptHandle":"r-1",...}],"ok":true}
```

State lives only in the process. The clock is virtual: it starts at 0 and
moves only with `Tick`.

## Layout

| Module | Responsibility |
|---|---|
| `command` | Parses one line into a typed `Command`. Checks request shape, required members and value types (SPEC section 2, checks 1 to 3). |
| `broker` | Owns all state and runs commands. Queue lookup and each command's own rules (checks 4 and 5). |
| `queue` | One queue's messages and the indexes behind expiry, receipt handles and deduplication. |
| `attributes`, `names`, `params` | Attribute and name syntax, and typed access to members. |
| `session` | The read-answer-flush loop over any `BufRead` and `Write`. |
| `response`, `error` | The response envelope and the closed set of error codes. |

## Test

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

- Unit tests sit beside each module.
- `tests/broker_messages.rs` drives the message commands through the same
  parse and execute path as the binary, and names the SPEC rule each test pins.
- `tests/cli.rs` runs the real binary over pipes.

## Known limits

- A literal `-0` is rejected, because `serde_json` reads it as `-0.0`. See the
  note in `src/params.rs`.
- `ReceiveMessage` scans the queue in arrival order, so a call costs time in
  the queue's size. Expiry, handle lookup, counts and the dead-letter move
  are indexed or bounded per message.
- Deduplication windows are kept for the life of the queue, so memory grows
  with the number of distinct deduplication ids sent. A window that has
  closed behaves like no window, so pruning them would be safe; it is not done yet.
