# Verification of review 8
## Score verdicts
| dimension | reviewer | verdict | corrected | evidence |
|---|---|---|---|---|
| Structure | +2 | confirmed | +2 | state.rs 477 lines, 21 submodules under state/ and state/queue/. The 11 `pub(super)` additions are exact (git diff baseline..HEAD). `pub use` globs are 19 lines in state.rs, not 18, and src glob-use density rose 4.03 -> 4.65 per KLOC; real but already named by the reviewer as the worst hunk |
| Duplication | +1 | confirmed | +1 | 5108af4fd removes two ~14-line ARN matches for `queue_named_by` (start_message_move_task.rs:450). a41704580 leaves 1 of 4 `usize::try_from(max_messages)` (receive_message.rs:446) |
| Error handling | +1 | confirmed | +1 | a41704580 drops the second identical `ReceiveRequestAttemptId` check (baseline receive_message.rs ~345-352); the first, kept at HEAD in `Request::parse`, returns earlier on the same input. `.expect(` 0.71 -> 0.70, unchanged |
| Domain types | 0 | confirmed | 0 | `queue_key` still `(String, String)` (addressing.rs:24); `Request`/`Taken` are control-flow structs |
| Ownership | 0 | confirmed | 0 | `.policy().map(str::to_string)` kept at receive_message.rs:714 and send_message.rs:344; clone() count unchanged in send_message.rs (12 vs 12) |
| Comments | +1 | confirmed (weak) | +1 | Banners gone (0.61 -> 0.00 per KLOC), rationale re-homed (refusals.rs:5-14, limits.rs:3-10). Offsetting: 63a315d74 leaves a stray bare `//` and a misplaced comment (see Missed changes); share 43.4 -> 42.9 is negligible. Borderline 0/+1, kept |
| Control flow | +1 | confirmed | +1 | receive `handle` 302-345 (43 lines) vs ~395 in baseline; send and batch handlers shrunk by extraction. `Prepared` 10-field bag is the weak part, already noted |
| Lint suppressions | +1 | confirmed | +1 | 586edbacb removes 3 crate allows (6 lints) from lib.rs; clippy.txt `allow_attributes_without_reason` 196 -> 190, total 360 -> 354 |
| Tests | 0 | zero confirmed | 0 | no path outside src/ in `git diff --stat baseline HEAD` for the crate |
| Commit hygiene | +2 | confirmed (generous) | +2 | 24 single-purpose commits, bodies state why. Small mixes: 440ac3623 is move plus module reorder; a41704580 is split plus dead-check removal (disclosed) |
| Behavior risk | 0 | confirmed | 0 | see below |

## Missed changes
- 63a315d74 `ops/send_message_batch.rs:~323-326`: after extracting the parameter loop and the measurement block, the comment "`MessageSystemAttributes` is honoured per entry ... by `prepare` below." now ends in a bare `//` line directly above `let sized = measure_entries(&input)?;`. The comment no longer describes the line under it and the `//` is a leftover fragment. Cosmetic, a small comment regression the review does not mention; does not move the Comments score.
- 440ac3623 / eca16422f: `forget_superseded_handles` gains `pub(super)` (counted by the reviewer in its 11).
- Duplication, Error handling, Domain types, Ownership, Control flow, Lint suppressions, Tests: zero/+ scores stand. Moved bodies in all 18 move commits diffed by multiset of trimmed lines: only `use` lists, `pub(super)`, module `//!` headers, dropped banners remain. 9b888ebfe shows no residue at all.

## Behavior risk
none found. Checked: a41704580 (parse still precedes `service_state`, `region` computed earlier but pure; `<Self as Operation>::NAME` equals `Self::NAME`; `take_messages` keeps materialise -> authorize -> replay/hand_over order; dropped check provably dead). 5048c7b0a (`prepare` is called after authorization at the same point the rules ran; dedup, bound and push order intact; extra `.clone()` of system attributes already existed in baseline). 63a315d74 (only comments sit between the old and new positions of the two extracted loops, so refusal order is unchanged). 5108af4fd (source classified first, blank destination skipped, same error types).

## Audit status verdicts
- S-14/S-15 partial: confirmed (duplicate attempt-id check gone, 3 of 4 try_from gone; the other DynamoDB/clock defaults are out of scope).
- S-18 partial: confirmed (the cited receive_message exemplar is split; send and batch handlers still 200+ lines; other giants untouched).
- S-19 partial: confirmed but generous. Banner removal matches one cited rule (docs-no-step-or-banner-comments); the finding's measured prevalence barely moved (comment share 43.4 -> 42.9, em dash 48.9 -> 47.7) and the exemplar remains at state/queue.rs:340-341.
- All others untouched: nothing found touching them.

## Hunks check
- Best 1 receive_message.rs:312-326: exists as quoted. Best 2 start_message_move_task.rs:217-227: exists as quoted (comment lines at 219-221 sit between the two statements in the file, omitted in the quote).
- Worst 1 send_message.rs:355-366: exists as quoted. Worst 2 state.rs:208-225: exists; the quote's "..." elides 14 globs, and the file has 19 `pub use` lines vs the review's "18".
- Other citations (addressing.rs:24, queue.rs:489, receive_message.rs:446/588/714, send_message.rs:344, message_move.rs:162, lib.rs:3-5 as the removed lines) check out.
