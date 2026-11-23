# miniq

A small in-memory message queue driven by one JSON command per line on stdin
(one JSON response per line on stdout). See `SPEC.md` for the protocol.

    cargo run --release < commands.jsonl

Layout:

- `src/service.rs`: command dispatch, clock and process-wide counters
- `src/queue.rs`: a queue, its messages, retention, deduplication, receive logic
- `src/attributes.rs`: queue attributes and validation of `Attributes` objects
- `src/params.rs`: typed access to command members (`null` means absent)
- `tests/`: black-box tests that run the `miniq` binary on scripted input
