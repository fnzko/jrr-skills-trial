# Review of run 14

## Summary
Run 14 is a small, targeted attempt: five commits, 75 insertions and 238 deletions, with no code moved between files.
It adds one helper, `authorization::denied_for_queue`, and routes 16 handlers through it. That removes the
repeated `queue.arn.clone()` / `policy().map(str::to_string)` copy and, in TagQueue, UntagQueue, AddPermission
and RemovePermission, the triple lookup that ended in `expect("the queue was resolved out of this same store")`.
It also deletes 23 inert `#[allow(unused_imports)]` and four lib-level clippy allows, drops two `Service` overrides
that only restated the trait defaults, and renames the `scratch_probe` test file. It leaves alone the module
layout, the giant handlers, the stringly state (`queue_key`, attribute maps), the `lift`/`store_unavailable`
boilerplate, the comment volume, and the same lookup pattern in SetQueueAttributes and StartMessageMoveTask.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 9592b7216 | refactor(sqs): authorize against a queue through one helper | dedupe | no |
| 9aacd1c51 | refactor(sqs): resolve the queue once in the four policy and tag handlers | rewrite | no |
| 6eec620a4 | chore(sqs): drop lint allows that suppress nothing | other (lint) | no |
| 5bb98196f | refactor(sqs): drop service overrides that restate the trait defaults | rewrite | no |
| 3b4dd3847 | test(sqs): rename scratch_probe to say what it checks | other (rename) | yes (100% similarity) |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | 0 | No module or file changes. The new helper sits beside `denied` (9592b7216, src/state/authorization.rs:367) and is `pub` like its sibling, so visibility is unchanged. |
| Duplication | +1 | 9592b7216 / 9aacd1c51: 16 copies of the clone-then-`denied` block collapse into a one-line `denied_for_queue` call, e.g. src/ops/tag_queue.rs:197 and src/ops/delete_message.rs. Real dedupe, but narrow. The same block stays in src/ops/set_queue_attributes.rs:332-346 and src/ops/start_message_move_task.rs:291-305. |
| Error handling | +1 | 9aacd1c51: 8 `expect("the queue was resolved out of this same store")` removed by resolving once with `queue_by_url_mut` (src/ops/untag_queue.rs:189, src/ops/add_permission.rs:231). `.expect(` drops from 0.71 to 0.31 per KLOC. 4 remain (set_queue_attributes.rs:248,335,350; start_message_move_task.rs:294). |
| Domain types | 0 | Nothing changed: `queue_key` still returns `(String, String)`, the attribute maps are untouched. |
| Ownership | +1 | 9592b7216 removes 16 `arn.clone()` + `policy().map(str::to_string)` pairs. 9aacd1c51 stops `queue_by_url_mut` from allocating the name (src/state.rs:2402-2404) and drops the `.map(str::to_string)` on the name in four handlers. `clone()` falls from 8.42 to 7.67 per KLOC. |
| Comments | 0 | 5bb98196f deletes about 14 template-hook comment lines in src/service.rs along with the dead overrides. Comment share rises from 43.4% to 43.7% because code shrank; no narration was rewritten. |
| Control flow | +1 | 9aacd1c51: three lookups in four handlers become one `?` chain (src/ops/tag_queue.rs:188-200). 5bb98196f: a two-arm match becomes `is_some_and` (src/service.rs:569-570). Giant handlers stay as they were. |
| Lint suppressions | +1 | 6eec620a4: `#[allow(` drops from 3.78 to 2.52 per KLOC; clippy `allow_attributes` 142 to 96 and `allow_attributes_without_reason` 196 to 146. The lib.rs blanket clippy allows are gone (src/lib.rs:1-5). Generated `ops/types` allows were left on purpose, and the commit says so. |
| Tests | 0 | Only a file rename (3b4dd3847, tests/all.rs:16-17) with identical content. No assertion changed. |
| Commit hygiene | +2 | Each commit holds one kind of change, its message explains why and states the order of checks or the no-behavior claim (9aacd1c51, 5bb98196f), and the history reads in order (helper first, then its reuse). |
| Behavior risk | 0 | Checked 9aacd1c51: `filter(holds_queue)` + `queue` + `queue_mut` against `queue_by_url_mut` uses the same `queue_key(region, name)` lookup (src/state.rs:2344-2356). `tag_refusal(&queue.name, ..)` and the `queue = %queue.name` log field get the same name the key matched. Removed overrides match the trait bodies (vendor/smithy-server-runtime/src/service.rs:390-411). |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | partial | 9aacd1c51 resolves the untag_queue exemplar once (src/ops/untag_queue.rs:189) and 9592b7216 drops 16 `policy().map(str::to_string)` copies. `queue_key` still allocates two Strings per lookup and 4 `expect`s remain. |
| S-14/S-15 | untouched | |
| S-16/S-17 | partial | 6eec620a4 removes 23 `#[allow(unused_imports)]` from hand-written op files. The `lift`/`store_unavailable` boilerplate and the generated-file allows remain. |
| S-18 | untouched | |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
9aacd1c51, src/ops/untag_queue.rs:188-200 (replaces about 30 lines, three lookups and two `expect`s):
```rust
        let queue = store
            .queue_by_url_mut(&region, &input.queue_url)
            .ok_or_else(|| {
                UntagQueueErrorModeled::QueueDoesNotExist(state::queue_does_not_exist())
            })?;
        if let Some(denied) =
            state::authorization::denied_for_queue(&*self.engine, ctx, Self::NAME, queue)
        {
            return Err(denied);
        }
```
Why: one lookup feeds authorization and mutation, so the impossible `expect` is gone and the check order is unchanged.

9592b7216, src/state/authorization.rs:367-374:
```rust
pub fn denied_for_queue<Eng: Send + Sync + 'static, E>(
    engine: &Eng,
    ctx: &::smithy_server_runtime::OperationContext<'_>,
    operation: &str,
    queue: &super::Queue,
) -> Option<crate::scaffolded::Scaffolded<E>> {
    denied(engine, ctx, operation, &queue.arn, queue.policy())
}
```
Why: the choice of which resource and policy to authorize against now lives in one place, and 16 needless clones go with it.

## Worst hunks
9592b7216 / 9aacd1c51, left unconverted, src/ops/set_queue_attributes.rs:332-346:
```rust
        let (queue_arn, policy) = {
            let queue = store
                .queue(&region, &name)
                .expect("the queue was resolved out of this same store");
            (queue.arn.clone(), queue.policy().map(str::to_string))
        };
        if let Some(denied) = state::authorization::denied(
```
Why: the exact pattern the run removed elsewhere survives here and in start_message_move_task.rs:291, so after this run the crate has two styles for the same step. A reader will ask which one is right.

## Silent edits check
No code was moved between files in this run. The one rename (3b4dd3847) has 100% similarity, so the test content is identical.
For the rewritten handlers I compared tag_queue, untag_queue, add_permission and remove_permission with baseline, line by line.
The only changes besides the lookup are `&name` -> `&queue.name` in `tag_refusal` and in the tracing field, which give the same value.
I also checked that the deleted `override_routing_error` / `override_framework_error` bodies match the vendored trait defaults, and that `queue_by_url_mut` keeps its signature and lookup.
Nothing else was edited.
