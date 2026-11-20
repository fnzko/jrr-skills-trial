# p1 / overall

## Method

No `BASE` directory exists, so each crate was judged as a whole. I read only `v2/task/SPEC.md` and `v2/blind/p1/*`, and I did not run cargo.

I used the same steps for every crate:

1. I read in full `lib.rs` (or `main.rs` when the I/O loop lives there), the error module and the queue module. The queue module holds the message state, receive, retention, dedup and handles.
2. I read the top of the request/command parser and of the broker/service dispatcher, up to the first handlers (about 80 to 200 lines). In each I looked at how parameters become typed values and how responses are built.
3. I read the id/name modules where they exist.
4. I ran the same greps over every crate: `unwrap`/`expect`/`clone`/`#[allow]`/`as` casts, `serde_json::Value` use, test counts, `assert_eq!` counts and weak assertions (`is_ok`, `contains`).
5. I opened the first two tests of the largest test file in a sample of crates (HE, VU, QO, FU).

The question asked of each crate was: how much do I have to learn before I trust a change, and how likely is a change to break an unstated invariant?

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| BI | 4 | 1 | BI/src/request.rs:13 | BI/src/queue.rs:336 | Small, typed `Request` enum and serde responses. Receive is one plain pass. The worst is a whole-message clone per delivery. |
| CE | 4 | 1 | CE/src/queue.rs:255 | CE/src/broker.rs:12 | Typed ids, a read-only `examine` then apply, and a `by_age` index. The only weakness is untyped `Map<String, Value>` responses. |
| FU | 4 | 1 | FU/src/queue.rs:21 | FU/src/service.rs:63 | `QueueKind::Fifo { content_based_deduplication }` and range types make bad states unrepresentable. Delete and change-visibility repeat the same preamble. |
| FI | 3 | 4 | FI/src/domain.rs:106 | FI/src/queue.rs:184 | Strong domain types, but an optimized scheduler (ready set, heap, group index). Counts infer delayed vs in-flight from `receive_count == 0`. |
| LI | 3 | 4 | LI/src/queue.rs:288 | LI/src/main.rs:7 | Clean queue with a by-handle index and a `DeadLetterMove` newtype. The I/O loop sits only in `main`, so it cannot be tested in-process. |
| QO | 3 | 4 | QO/src/deliveries.rs:30 | QO/src/types.rs:49 | Very thorough types and typed handle resolution. It is large (2450 lines) and `Seconds`/`Count` are near-duplicate generics. |
| TO | 3 | 4 | TO/src/receipt.rs:28 | TO/src/queue.rs:194 | Readable, with plan/apply, a delivery log and typed replies. The sequence number doubles as the arrival key on standard queues. |
| WU | 3 | 4 | WU/src/name.rs:12 | WU/src/params.rs:64 | Validated `QueueName`, typed ids and a by-sent index. Integer checking silently depends on the `arbitrary_precision` Cargo feature. |
| ZE | 3 | 4 | ZE/src/receipts.rs:24 | ZE/src/queue.rs:86 | Clear plan/apply receive and a receipt log. `Sent.sequence` is a bare `u64` even on standard queues, plus an `accepted` flag. |
| HE | 2 | 10 | HE/src/queue.rs:75 | HE/src/queue.rs:194 | Good `Hold` enum, but messages live in a `Vec`. `Received` returns indices that are valid only until the next change, and delete scans linearly. |
| MI | 2 | 10 | MI/src/queue.rs:130 | MI/src/queue.rs:67 | `SendOutcome` enum is nice. Ids are bare `u64`, and `dead_letter_of` silently drops a half pair, so it relies on the caller. |
| TA | 2 | 10 | TA/src/queue.rs:1 | TA/src/queue.rs:183 | Well documented, but stored message state plus five indexes rest on an unchecked "call `sync` first" contract. |
| VU | 2 | 10 | VU/src/queue.rs:259 | VU/src/queue.rs:58 | Compact decide-then-apply receive. Message id, handle and queue uid are all bare `u64`, and responses are ad-hoc `json!`. |
| BO | 1 | 14 | BO/src/queue.rs:288 | BO/src/message_commands.rs:19 | Dispatch is by string, and every handler parses its own parameters (`require` then `.expect("required")`) and mixes that with state. |
| PO | 1 | 14 | PO/src/queue.rs:194 | PO/src/broker/message_commands.rs:57 | Same shape as BO: string dispatch, require/expect per handler, bare `u64` ids, and amortized dedup pruning for no stated need. |

## What separates the top from the bottom

- **Parse once into a typed command.** Every top crate parses the line into a typed command enum before any state is touched: BI/src/request.rs:13, CE/src/request.rs:58, FU/src/service.rs:29.
  - BO and PO keep the raw JSON map until the handler runs (BO/src/broker.rs:56, PO/src/broker.rs:65). Each handler then re-asserts presence with `expect("required")`, so a renamed parameter becomes a panic, not a compile error.
- **Plain data model vs. hand-built indexes.** CE, BI and FU keep one ordered map plus at most a `by_age` index (CE/src/queue.rs:109). Their state is derived from timestamps on demand.
  - FI (FI/src/queue.rs:115-136) and TA (TA/src/queue.rs:132-146) keep four or five indexes that every mutation must update together.
  - TA also stores the message state itself, which is correct only if `sync` was called (TA/src/queue.rs:16-18). That is far more to learn before a change is safe.
- **Storage that can't be invalidated under you.** HE keeps messages in a `Vec`. It hands back indices that are only "valid until the queue is next changed" (HE/src/queue.rs:194-200) and finds handles by linear scan (HE/src/queue.rs:484).
  - Every other crate keys messages by a stable arrival number.
- **Pairs that belong together are typed together.** FU, HE and QO make content-based dedup part of the FIFO kind (FU/src/queue.rs:21, QO/src/queue.rs:27). MI rebuilds the dead-letter pair with a match that silently ignores half a pair (MI/src/queue.rs:67-74).
- **Typed ids.** The upper half gives `MessageId`/`ReceiptHandle` their own types with a single canonical parser (CE/src/ids.rs:26, FU/src/ids.rs:38). MI, PO and VU pass bare `u64` handles and ids through queue, broker and response code (VU/src/queue.rs:58-59).

## Patterns seen in most crates

- **Done well almost everywhere:**
  - A closed `ErrorCode` enum.
  - A two-phase receive (decide on shared borrows, then apply).
  - Lazy retention on lookup, which matches the spec's "at every moment" rule.
  - An identical I/O loop (`read_until`, strip `\n`, write, flush).
  - Handle parsing that rejects `r-01`, `R-1` and `r-1 ` by canonical-form checks.
- **Common weaker spots:**
  - About half build responses as `serde_json::Value`/`json!` rather than serializable types (BO, CE, LI, MI, PO, VU, WU).
  - Most keep the dead-letter target as a bare `String`.
  - Several share a near-identical "find in-flight message by handle" preamble between DeleteMessage and ChangeMessageVisibility.
- **Size vs. problem:**
  - Most crates are 1150 to 1500 source lines.
  - The 1700 to 2800 line crates (FI, HE, QO, TA) add optimization or type machinery the spec's scale does not require, and that cost shows up as learning time rather than safety.
