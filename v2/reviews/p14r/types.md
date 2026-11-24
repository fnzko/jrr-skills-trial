# p14r / types

## Method

I read every file of `BASE/src` in full. For each of the 15 attempts I read the full `diff -r -U2 BASE/src <CODE>/src`, plus the full text of each new module (`ids.rs`, `limits.rs`, `time.rs`). Every crate then got the same fixed grep pass over `src/`, which counted:

- string dispatch on the command name (`match req.command.as_str`)
- `expect("required")` after `Params::require`
- newtypes over `u64`/`String`
- the FIFO tuple `Some((group_id, deduplication_id))` / `Option<(&str, &str)>`
- `group_id: Option<String>` and `sequence_number: Option<u64>` as separate message fields
- `dead_letter_queue: Option<String>` in `AttributeUpdate`
- `BTreeMap<String, Queue>`
- raw `u64` keys (`BTreeMap<u64, Message>`, `next_message_id: u64`)

I did not read the tests, since they are not the dimension. Because every attempt starts from BASE, I scored the whole crate and let the diff decide the differences.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| TA | 4 | 1 | TA/src/naming.rs:19 (`QueueName` parsed at create, map keyed by it, TA/src/broker.rs:31) | TA/src/broker/message_commands.rs:138 (`expect` because `group_id`/`sequence_number` stay two Options) | Typed queue names, `Timestamp`/`Seconds`, all three ids, `Command`, and a `FifoSend` enum. One coupling is left open. |
| JU | 3 | 2 | JU/src/queue.rs:34 (`FifoPlace { group_id, sequence_number }` replaces two parallel Options) | JU/src/queue.rs:83 (arrival key still bare `u64`) | The only attempt that makes the FIFO fields one value. It also adds `SendTarget`, `Command` and private id newtypes. |
| DE | 3 | 3 | DE/src/ids.rs:63 (`ReceiptHandles::parse_issued` returns only handles this broker issued) | DE/src/broker/message_commands.rs:111 (`SendRules::Fifo` is flattened back into a tuple) | Private newtypes with issuers, `Command` parsed in `Request::parse`, and a `SendRules` enum. |
| DI | 3 | 3 | DI/src/broker/message_commands.rs:34 (`FifoIds` struct for group and dedup ids) | DI/src/queue.rs:22 (`group_id: Option<String>` beside `sequence_number: Option<u64>`) | `Command` at the boundary, private `MessageId`/`ReceiptHandle`/`Arrival`, and named FIFO ids. |
| PE | 3 | 3 | PE/src/ids.rs:58 (private `ArrivalKey` with `take_next`) | PE/src/queue.rs:22 (parallel FIFO Options) | Same shape as DI: `Command`, three private newtypes and a `FifoIds` struct. |
| XE | 3 | 3 | XE/src/broker/message_commands.rs:32 (`FifoSend` struct; the dead-letter config is built once and passed to `apply`) | XE/src/broker/message_commands.rs:123 (`expect` on the sequence number the types do not tie to FIFO) | Three newtypes and `Command`. Ids stay typed through the in-flight lookup. FIFO coupling is still by convention. |
| RU | 3 | 7 | RU/src/ids.rs:58 (private `ArrivalKey`) | RU/src/broker/message_commands.rs:40 (`expect("presence checked by Params::require")` repeated, no typed required getter) | Newtypes and `Command` are done. Required parameters are still unwrapped and the FIFO tuple remains. |
| VO | 3 | 7 | VO/src/ids.rs:62 (private `Arrival` with `issue`) | VO/src/broker/message_commands.rs:44 (`Option<(&str, &str)>` return), plus 11 `expect("required")` | Ids and `Command` are typed. Required values and the FIFO pair stay loose. |
| CE | 3 | 9 | CE/src/ids.rs:38 (`ArrivalIndex` key type) | CE/src/ids.rs:7 (`MessageId(pub u64)` with a pub field, the counter is still a bare `u64` at CE/src/broker.rs:24) | `Command` is parsed lazily at CE/src/request.rs:73, so `Request` still holds the `String`. Newtypes are open. |
| ME | 3 | 9 | ME/src/request.rs:16 (`Command` parsed in `Request::parse`) | ME/src/queue.rs:76 (`BTreeMap<u64, Message>`, arrival still a raw integer) | Message ids and handles are typed. Arrival keys and the FIFO pair are not. |
| KA | 2 | 11 | KA/src/ids.rs:39 (`Arrival` newtype separates keys from handles) | KA/src/broker.rs:65 (commands still dispatched by string match) | Id newtypes with pub fields. No `Command` enum, and `expect("required")` remains. |
| MO | 2 | 11 | MO/src/request.rs:16 (`Command` enum parsed at the boundary) | MO/src/queue.rs:75 (`BTreeMap<u64, Message>`; ids, handles and keys all bare `u64`) | Fixes the closed command set only. |
| KO | 2 | 13 | KO/src/broker/message_commands.rs:53 (`FifoIds` struct inside a `FifoSend` enum) | KO/src/broker.rs:65 (string dispatch), and raw `u64` ids at KO/src/broker.rs:24 | Only the send path gains types. |
| LA | 2 | 14 | LA/src/broker/message_commands.rs:53 (`FifoSend` enum separates the duplicate outcome) | LA/src/broker/message_commands.rs:57 (`New(Option<(&str, &str)>)` keeps an anonymous pair) | Small step on one handler. Ids and commands are unchanged. |
| VI | 2 | 14 | VI/src/broker/message_commands.rs:48 (`FifoSend` enum) | VI/src/broker/message_commands.rs:52 (same anonymous tuple), plus string dispatch at VI/src/broker.rs:65 | Same as LA. |
| BASE | 1 | 16 | BASE/src/attributes.rs:20 (`DeadLetter` struct, `Option` for unset) | BASE/src/broker.rs:65 (command names matched as strings); BASE/src/queue.rs:75 (`u64` used for ids, handles and keys alike) | Good `ErrorCode`/`State`/`Step` enums. Identifiers of three kinds share `u64`, and the command set is a string. |

## What separates the top from the bottom

- **Identifier newtypes.** The bottom five keep message ids, receipt handles and arrival keys all as `u64`, so `HashMap<u64, u64>` can be read either way round (BASE/src/queue.rs:75, MO/src/queue.rs:75). Every 3-or-better crate gives at least two of them their own types. The best also make the field private and hand out values through an issuer (DE/src/ids.rs:46, DI/src/ids.rs:59). CE and KA leave the field `pub`.
- **Parse the command once at the boundary.** Most attempts turn `Command` into an enum inside `Request::parse`. KA, KO, LA, VI and BASE still `match` on strings (KA/src/broker.rs:65). CE adds the enum but keeps the `String` in `Request` and parses it per dispatch (CE/src/request.rs:73).
- **FIFO fields that only make sense together.** Only JU merges `group_id` and `sequence_number` into one `Option<FifoPlace>` (JU/src/queue.rs:34). Everyone else keeps two parallel Options. TA and XE then need an `expect` to rejoin them (TA/src/broker/message_commands.rs:138, XE/src/broker/message_commands.rs:123). On the send side, DE, JU, TA and XE name the group/dedup pair with an enum or struct. LA, VI, VO and KA keep an anonymous `(&str, &str)`.
- **Typed domain values beyond ids.** Only TA types queue names (TA/src/naming.rs:19, with `Borrow<str>` so lookups still take `&str`) and splits instants from spans (`Timestamp`/`Seconds` in TA/src/time.rs). That is what puts it alone at 4.
- **Required parameters.** Most attempts replace `expect("required")` with `required_string`/`required_integer`, which return a typed value. RU only rewords the `expect` (RU/src/broker/message_commands.rs:40). VO, KA, KO, LA, VI and MO keep the BASE `expect("required")` calls.

## Patterns seen in most crates

- Nearly all of them keep `AttributeUpdate` with separate `dead_letter_queue: Option<String>` and `max_receive_count: Option<u64>` (for example BASE/src/attributes.rs:101). The "both or neither" rule is then rechecked by `dead_letter()`. TA and XE at least compute the `DeadLetter` once and pass it to `apply`. Nobody parses it into one value at the boundary.
- Queue names stay `String`/`&str` everywhere except TA, and `DeadLetter.queue` is a `String` in every crate.
- Attribute names stay a `&str` array plus `reject_unknown` in every crate. Result members stay a `serde_json::Map` built at the edge, which is acceptable since it is the wire format.
- The `ErrorCode`, `State` and `Step` enums and the `Option` for unset times were already good in BASE, and every attempt keeps them.
