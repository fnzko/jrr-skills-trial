# p1 / ownership

## Method
Same steps for all 15 crates, no `cargo`:
1. Grepped `src/` for `.clone()`, `to_string`/`to_owned`/`String::from`/`format!`, sort/ordered collections, `collect` and `retain`, and read the code around every `.clone()`.
2. Read each crate's queue module in full (storage, retention, send, receive, delete and change-visibility, dedup), and in the broker or service the send, receive and dead-letter code.
3. Grepped for how `Body` is parsed, and how the queue map is typed, to see whether ListQueues sorts or scans on each call.
I judged the cost of retention, receipt-handle lookup and the FIFO group check per command, whether stored data is moved in or borrowed and then copied, and whether a clone exists only to end a borrow.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| BO | 4 | 1 | BO/src/queue.rs:291 | BO/src/queue_commands.rs:116 | `(sent_at,key)` index for retention, dedup ids pruned from a time-ordered deque, borrowed group set; one settings clone to apply a patch |
| CE | 4 | 1 | CE/src/queue.rs:137 | CE/src/broker.rs:302 | by-age index, handle map, `HashSet<&str>` groups; the only waste is cloning the DLQ name on every receive |
| FI | 4 | 1 | FI/src/queue.rs:251 | FI/src/broker.rs:253 | `ready`/`hidden` indexes, so receive never walks hidden messages; an owned dedup id is copied with `as_str().to_owned()` |
| FU | 4 | 1 | FU/src/queue.rs:390 | FU/src/command.rs:240 | indexes for age and handles, `HashSet<&GroupId>`; strings are copied out of a borrowed `Value` |
| ZE | 4 | 1 | ZE/src/queue.rs:216 | ZE/src/broker.rs:123 | owned `SendRequest` moved into storage, `by_sent` index, receipt log is a `Vec` lookup |
| BI | 3 | 6 | BI/src/queue.rs:206 | BI/src/queue.rs:341 | oldest-bound skip, but a full scan plus a `min` recompute when anything expires; a `String` allocated per group and the whole message cloned on each receive |
| LI | 3 | 6 | LI/src/queue.rs:120 | LI/src/queue.rs:206 | good indexes, but `send(&SendRequest)` clones body, group and dedup id instead of moving them |
| PO | 3 | 6 | PO/src/queue.rs:255 | PO/src/queue.rs:111 | amortized dedup pruning, borrowed group set; retention does a full scan, then a second scan for `min` |
| QO | 3 | 6 | QO/src/queue.rs:384 | QO/src/queue.rs:387 | owned `NewMessage`, borrowed groups; retention rescans the whole queue whenever the bound is hit |
| TA | 3 | 6 | TA/src/queue.rs:326 | TA/src/queue.rs:238 | visible/hidden indexes and per-group heads, but stored body and group are cloned from `&SendMessage`, and the config is cloned to patch it |
| TO | 3 | 6 | TO/src/receipt.rs:61 | TO/src/queue.rs:267 | owned sends, handles in an indexed `Vec`; every Tick runs `retain` over all messages and dedup ids of every queue |
| VU | 3 | 6 | VU/src/queue.rs:64 | VU/src/queue.rs:149 | owned `NewMessage`, `Arc<str>` group shared into deliveries; same full rescan when the oldest-bound check is hit |
| MI | 2 | 13 | MI/src/queue.rs:202 | MI/src/queue.rs:207 | full retention scan on every new clock value, `HashSet<String>` with a clone per group, body copied twice (owned -> `&str` -> `to_owned`) |
| WU | 2 | 13 | WU/src/queue.rs:195 | WU/src/queue.rs:300 | good indexes, but a clone just to release a borrow on each receive (dead-letter config, `Command`), a `String` per group, a whole-message clone per delivery, and a cloned dedup id |
| HE | 1 | 15 | HE/src/broker.rs:60 | HE/src/queue.rs:485 | zero-copy `Command<'_>` parsing, but messages are a `Vec`: delete and change-visibility scan linearly for the handle and `Vec::remove` shifts the tail; stored body taken as `&str` |

## What separates the top from the bottom
- Retention cost. BO, CE, FU, LI, TA, WU, ZE and FI keep a `(sent_at, arrival)` ordered index and pop only what expired (CE/src/queue.rs:137). BI, PO, QO and VU skip work with an oldest-bound check, but once it is hit they rescan the whole queue and recompute the minimum (PO/src/queue.rs:111-120). MI rescans whenever the clock changes (MI/src/queue.rs:207), and TO and HE rescan every queue on Tick (TO/src/queue.rs:267).
- Receipt-handle lookup. Most crates map handle -> arrival key (`HashMap`) or index a `Vec` log by handle number (ZE/src/receipts.rs:49). HE alone finds the message with `iter().position` over the whole queue (HE/src/queue.rs:485), and then removes it with an O(n) `Vec::remove`.
- Moving owned input into storage. ZE, QO, VU, TO, FU and BI destructure the owned request and move `body` in (ZE/src/queue.rs:216). LI and TA borrow an owned request and clone the fields they store (LI/src/queue.rs:206, TA/src/queue.rs:238). MI turns an owned `String` back into `&str` and calls `to_owned` again (MI/src/queue.rs:298).
- The FIFO group check. Most collect `HashSet<&str>` or `&GroupId` borrowed from the map (QO/src/queue.rs:550). MI, WU, HE and BI allocate a `String` or `GroupId` per group on every receive (MI/src/queue.rs:370, WU/src/queue.rs:343). FI and TA keep per-group heads, so receive needs no set at all (FI/src/queue.rs:357).
- Clones that only end a borrow. WU clones the whole dead-letter config before the loop on every receive (WU/src/queue.rs:300) and clones `Command` (WU/src/request.rs:71). Most others clone only the DLQ name, and only when something was dead-lettered (BI/src/service.rs:328).

## Patterns seen in most crates
- Every crate except HE uses an ordered map keyed by an arrival counter, so receive walks in arrival order and stops at `MaxMessages`. Deletes go through a key rather than a scan.
- Nearly all parse parameters from a borrowed `serde_json::Value` and copy each string (`text.clone()` / `to_owned()`) instead of taking the `Map` by value. It is one copy per field, but it is everywhere.
- Delivering a message clones its body and group into the response struct. This is unavoidable without `Rc`, and only VU shares the group through `Arc<str>`.
- To dead-letter, the DLQ name is cloned to end the borrow on the source queue before the target is looked up (CE/src/broker.rs:302, FU/src/service.rs:325, ZE/src/broker.rs:123). This is a mild borrow-checker clone that almost all of them share.
- `GetQueueAttributes` counts states with a full pass in every crate. That fits the spec, since state depends on `now`.
