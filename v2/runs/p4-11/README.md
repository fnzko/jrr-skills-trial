# miniq

miniq is an in-memory message queue service. It reads commands as JSON lines
on stdin and writes one JSON response line per command on stdout. It uses no
network and no files; time is virtual and advances only with `Tick`.

The protocol, error codes and queue rules are in [SPEC.md](SPEC.md).

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
```

## Layout

- `src/protocol.rs`: reads lines, writes responses, flushes after each.
- `src/command.rs`: parses one line into a typed `Command`; checks request
  shape, required parameters and parameter values.
- `src/broker.rs`: the clock, all queues and the delivery log; checks that
  need queue state, in the order the spec gives.
- `src/queue.rs`: one queue's messages, send and deduplication, receive
  planning and retention.
- `src/attributes.rs`, `src/types.rs`, `src/wire.rs`: queue attributes,
  validated identifiers and JSON parameter conversion.
- `src/error.rs`: the ten error codes.
- `tests/`: whole-protocol sessions and a run of the binary.
