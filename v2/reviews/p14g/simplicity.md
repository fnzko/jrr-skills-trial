# p14g / simplicity

## Method

For every crate I read the whole of `src/` up to the first `#[cfg(test)]` (test modules, blank lines, comment lines and `use` lines stripped by the same script for all 12), so each crate got the same full read of its production code. On top of that I ran the same greps on every crate: `#[allow]`, `dbg!`, `todo!`, `unreachable!`, traits, generics, builders, `macro_rules!`, and a defined-but-never-called check for every `fn` name. Tests and `Cargo.toml` were not read. No cargo was run. While reading I looked for: helpers or types with one user, the same rule checked in two places, copied blocks across handlers, caches or bookkeeping the spec does not need, loops that are not in their direct form, dead code, and whether the module split fits a ~1.5k line problem.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| CI | 3 | 1 | CI/src/broker.rs:248 | CI/src/params.rs:90 | Direct broker and single-pass receive; small lapses: unused `required_integer`, the CBD check repeated in create and set, `require` then `required_string` checks a parameter twice. |
| CU | 3 | 1 | CU/src/queue.rs:297 | CU/src/broker.rs:208 | Clean `Hold` enum and one receive loop; CBD and dead-letter pair checks are written in both create and set, a macro covers only two types, and `lookup` and `queue_mut` overlap. |
| PA | 3 | 1 | PA/src/types.rs:30 | PA/src/broker.rs:275 | `bounded!` earns its six users and the queue is compact; send packs `fifo_send` into a tuple, unpacks it, then matches it again, and every handler fetches all params and parses them in a second block. |
| TA | 3 | 1 | TA/src/queue.rs:239 | TA/src/request.rs:244 | `append(Incoming)` is shared by send and dead-letter, which is a real second user; small noise: a `name_for_errors` alias, an exhaustive `Value` match to say "not a string", a unit error type that is always mapped away. |
| DO | 2 | 5 | DO/src/store.rs:155 | DO/src/queue.rs:304 | `Queue` wraps `Store` with pass-through methods, there are five error sub-enums for one wire code each, standard and FIFO send arms repeat room/id/push, and an `unreachable!` branch covers a state that cannot happen. |
| HO | 2 | 5 | HO/src/response.rs:11 | HO/src/queue.rs:201 | The plain JSON-map responses are direct, but a `swept_at` cache needs an `invalidate_sweep` call from the broker (one decision in two places), dead-letter validation is copied between create and set, and receive threads a `remains` flag through an if/else chain. |
| PE | 2 | 5 | PE/src/queue.rs:339 | PE/src/broker.rs:292 | Plan/step receive reads well; `receive_message` rewrites the `queue()` lookup inline, `swept_at` plus `refresh`/`expire` adds a cache, and `Command` fields are repacked into an `Outgoing` struct with no other user. |
| SE | 2 | 5 | SE/src/queue.rs:207 | SE/src/fifo.rs:74 | Send is one match on `Kind`, but there is an unrequested dedup eviction queue, closures passed in only to allocate ids and handles, a `Deliveries` log of every handle, and two-step `Envelope` then `Request` parsing. |
| XI | 2 | 5 | XI/src/types.rs:10 | XI/src/queue.rs:386 | Three macros, two of them for two types each, generate an unused `MessageId::parse`; receive mixes `eligible` and `group_blocked` flags out of order; `set_attributes` ends with a bare `self.queue(name)?` and looks the target up twice (`kind_of`, `existing_name`). |
| ZO | 2 | 5 | ZO/src/handles.rs:21 | ZO/src/request.rs:36 | The broker is tidy, but a `Command` enum maps names only to be matched again, an `oldest_sent` cache drives a full `retain` scan, the dedup key is built twice in send, and `ReceiptHandle::number` is never called. |
| HU | 1 | 11 | HU/src/queue.rs:289 | HU/src/message_commands.rs:19 | There is no request type: every handler repeats `require(..)` followed by `.expect("required")`; a VecDeque+Rc dedup eviction the spec does not need; set_queue_attributes calls `expire` twice around `apply`; a global handle Vec plus queue ids. |
| NO | 1 | 11 | NO/src/naming.rs:16 | NO/src/queue.rs:254 | Every handler copies the same require/expect block; `oldest_sent_at` and `prune_deduplication_at` add bookkeeping the spec does not need; create validates `update.dead_letter()?` and throws the result away, then `apply` works out the pairing again; two different `invalid_value` helpers. |

## What separates the top from the bottom

- The top four parse each line once into a typed request enum, so handlers start from typed values (CI/src/request.rs:66, TA/src/request.rs:79). HU and NO skip that step and each handler restates `require` + `string()?.expect("required")` (HU/src/queue_commands.rs:82, NO/src/broker/message_commands.rs:57), which is the most widespread copied block in the set.
- Retention: the top crates keep a `(sent_at, arrival)` BTreeSet and pop from the front (CI/src/queue.rs:267, TA/src/queue.rs:199). The lower ones add caches to avoid a full scan instead of removing the scan: `swept_at` (HO/src/queue.rs:201, PE/src/queue.rs:166), `oldest_sent_at` (NO/src/queue.rs:105, ZO/src/queue.rs:247). These are extra state whose correctness depends on other call sites (HO/src/broker.rs:161).
- Dedup windows: the top crates keep one `HashMap` lookup with a time filter (CI/src/queue.rs:346). SE, HU and NO add eviction machinery the spec never asks for (SE/src/fifo.rs:74, HU/src/queue.rs:225, NO/src/queue.rs:254).
- Shared queue validation: the stronger crates factor dead-letter resolution into one helper used by both create and set (CI/src/broker.rs:197, PA/src/broker.rs:409). HO, HU and NO copy the lookup-and-type-check block into both handlers (HO/src/broker.rs:113 and :145, NO/src/broker/queue_commands.rs:62 and :100).
- Extra layers: DO's `Queue` over `Store` (DO/src/queue.rs:201-210 just forward) and ZO's `Command` enum before `Request` (ZO/src/request.rs:36) add a hop with no second user.

## Patterns seen in most crates

- No `#[allow]`, `dbg!`, `todo!` or leftover debugging anywhere; module layout (types / request / queue / broker / response) fits the problem size in all twelve.
- Nearly all repeat the `ContentBasedDeduplication && !fifo` check in both CreateQueue and SetQueueAttributes rather than once; it is small, but it is the same rule in two places.
- Nearly all write the `require(names)` pre-pass and then the typed getter, which checks presence twice; it exists to get the spec's MissingParameter-before-InvalidParameterValue order, but only PA and TA get that order from the shape of the code (fetch all, then convert) without the double check.
- Most split receive into plan and apply (`Step`/`Verdict`), which reads well; the single-pass versions (CI, CU, SE) are just as clear and shorter.
