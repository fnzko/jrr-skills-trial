# p1 / comments
## Method
No `BASE` directory exists, so every crate was judged as a whole. Same procedure for all 15 crates:
1. Counted total `.rs` lines and comment lines (`//`, `///`, `//!`) in `src/` and `tests/`.
2. Pulled every line containing `//` from `src/` and `tests/` with `grep -rn` and read the full list for each crate.
3. Paired every single-line `///` doc in `src/` with the item it documents (an awk pass), to spot docs that only restate the signature or body.
4. Grepped every crate for banners, `TODO`/`FIXME`, `Step N`, separators and `#[allow]` next to comments.
5. Opened source only to confirm a suspected lapse (FI tests/queues.rs rule labels, PO naming.rs, HE values.rs).
No crate got a closer read than another.

Comment volume per crate is 48 to 150 lines out of 2100 to 3600 total, so it is in proportion everywhere.
No crate has banners, history or plan talk, TODOs or leftover debugging comments.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| BI | 4 | 1 | BI/src/queue.rs:173 | BI/src/service.rs:67 | Field and method docs state invariants (lower bound, canonical handle spelling); almost no restating |
| QO | 4 | 1 | QO/src/deliveries.rs:10 | QO/src/queue.rs:378 | Module docs give invariants ("fields are private so values are in range"); one weak restate |
| TO | 4 | 1 | TO/src/queue.rs:263 | TO/src/queue.rs:277 | Docs explain why retention needs no other check and what the serde gotcha is; tests commented sparingly |
| VU | 4 | 1 | VU/src/queue.rs:259 | VU/src/lib.rs:15 | Fewest comments of the set, nearly every one a reason (decide-then-apply, Debug equals the wire name) |
| WU | 4 | 1 | WU/src/queue.rs:126 | WU/src/request.rs:58 | Explains non-obvious choices (sent order is not arrival order, ASCII byte length) |
| ZE | 4 | 1 | ZE/src/receipts.rs:24 | ZE/src/error.rs:21 | Contracts on receipts and moves are stated; restating docs are rare and short |
| BO | 3 | 7 | BO/src/params.rs:66 | BO/src/queue.rs:19 | Good gotcha notes; a few `Whether ...`/`Removes ...` docs repeat the function name |
| CE | 3 | 7 | CE/src/queue.rs:132 | CE/tests/queues.rs:94 | Strong field invariants; tests lean on spec rule numbers ("Rule 1 comes before rule 2") |
| FU | 3 | 7 | FU/src/text.rs:40 | FU/src/queue.rs:427 | Good contract comments on `Borrow`; some method docs paraphrase the signature |
| HE | 3 | 7 | HE/src/values.rs:101 | HE/src/values.rs:159 | Excellent gotcha docs, but the same restating doc is copied onto three `take_next` methods |
| LI | 3 | 7 | LI/src/broker.rs:184 | LI/src/command.rs:67 | Reasons where they matter; several docs restate (`Parses one input line.`, `Runs one input line...`) |
| MI | 3 | 7 | MI/src/main.rs:26 | MI/src/command.rs:197 | Explains framing edge cases; restating `Whether ...` docs and "steps 1-3 / 4-5" spec numbering |
| TA | 3 | 7 | TA/src/queue.rs:321 | TA/src/queue.rs:42 | Best rationale comments of the set (cursor walk, old config before new) but the highest share of comments, some on self-evident variants |
| FI | 2 | 14 | FI/src/queue.rs:125 | FI/tests/queues.rs:83 | Fine invariant docs in src, but tests are full of bare step labels (`// Rule 2.`, `// Rule 5.`) and volume is the highest |
| PO | 2 | 14 | PO/src/broker.rs:98 | PO/src/naming.rs:23 | `(SPEC section N, check M)` tags in about ten docs, and `True if ...` docs that repeat one-line bodies |

## What separates the top from the bottom
- The top crates comment fields with invariants a reader could not get from the type, e.g. BI/src/queue.rs:173 (lower bound that lets sweeps skip), ZE/src/receipts.rs:24 (why receipts are never dropped), QO/src/deliveries.rs:10 (the log is also the set of issued handles).
- The bottom two use spec numbering instead of saying what is being checked: FI/tests/queues.rs:81-90 and 208-218 are bare `// Rule N.` labels, and PO puts `(SPEC section 2, check 1)` on docs (PO/src/request.rs:21, PO/src/broker/message_commands.rs:203).
- Restating docs on trivial helpers separate the 3s from the 4s: PO/src/naming.rs:10-23, MI/src/command.rs:190-197, LI/src/command.rs:67, FI/src/domain.rs:126, and HE/src/values.rs:159/180/212 (one doc copied three times).
- Gotcha comments on library behaviour (serde `arbitrary_precision`) show up in the stronger crates: TO/src/request.rs:316, HE/src/request.rs:293, BO/src/params.rs:66, WU/src/params.rs:64.
- Comment volume works both ways. VU (48 lines) loses nothing with so few, while TA (133) and FI (150) have the best rationale paragraphs and also the most comments that add nothing.

## Patterns seen in most crates
- Nearly all have a short module doc (`//!`) per file saying what the file owns, and a one-line doc on the error enum saying the set is closed.
- Almost every crate writes the same doc on `purge` ("Sequence numbers and deduplication carry on") and the same note on the queue map ("ordered by name, which is the ListQueues order"). These are contract statements, not restatements.
- Test comments explain the expected outcome of a scenario ("a1 is dead-lettered, which frees group A within the same call") instead of narrating the steps. That is good everywhere.
- None has banners, history, TODOs, or debugging leftovers. The only `#[allow]` with a comment is LI/tests/common/mod.rs:1. The weak spots are mild: restating one-liners on small helpers, and spec rule or step numbers in place of a description.
