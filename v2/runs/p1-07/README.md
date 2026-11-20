# miniq

A small in-memory message queue. It reads one JSON command per line on stdin
and writes one JSON response per line on stdout; see [SPEC.md](SPEC.md).

```sh
printf '%s\n' '{"Command":"CreateQueue","QueueName":"jobs"}' \
              '{"Command":"SendMessage","QueueName":"jobs","Body":"hi"}' | cargo run -q
```

Layout: `command` parses and validates a line, `broker` owns the clock and
global counters and executes commands, `queue` holds one queue's messages.
Time is virtual and only advances with `Tick`.
