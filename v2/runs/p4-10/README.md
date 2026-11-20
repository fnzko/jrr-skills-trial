# miniq

An in-memory message queue service. It reads commands as JSON lines on
stdin and writes one JSON response line per command on stdout. There is no
network, no files and no wall clock: state lives in the process and time
only moves when a `Tick` command says so.

The behaviour is specified in [SPEC.md](SPEC.md).

## Usage

```sh
cargo build --release
printf '%s\n' \
  '{"Command": "CreateQueue", "QueueName": "jobs"}' \
  '{"Command": "SendMessage", "QueueName": "jobs", "Body": "hello"}' \
  '{"Command": "ReceiveMessage", "QueueName": "jobs"}' \
  | target/release/miniq
```

## Testing

```sh
cargo test
cargo clippy --all-targets
cargo fmt --check
```

Unit tests sit next to the code they check. The files in `tests/` drive the
broker through the same entry points the binary uses: `queues.rs`, `send.rs`,
`receive.rs` and `receipts.rs` cover the commands, `protocol.rs` covers the
line protocol, and `binary.rs` runs the executable.

## Layout

| Module | Responsibility |
|---|---|
| `protocol` | Reading command lines and writing responses (section 1) |
| `broker` | All state and the execution of each command |
| `queue` | One queue: its messages, retention, deduplication and receive planning |
| `attributes` | Queue attributes, defaults and their validation (section 4) |
| `params` | Typed access to command parameters, in the order section 2 checks them |
| `syntax` | Queue name and `GroupId`/`DeduplicationId` syntax |
| `error` | The closed set of error codes (section 2) |

## Notes

- A message is removed by retention lazily, whenever its queue is touched,
  and eagerly on `Tick`. Both give the same observable result.
- `ReceiveMessage` and `GetQueueAttributes` scan the queue, so they cost
  time proportional to its size. Sending is logarithmic.
