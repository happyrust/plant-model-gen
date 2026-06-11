# Implementation Plan

## Approach

Single UPDATE statement at startup ("boot-generation recovery"). Reuse the existing
init hook inside `create_admin_task_routes` where `ensure_admin_tasks_table` already
runs, so recovery is guaranteed to precede route availability without touching the
server bootstrap sequence.

## Files

- `src/web_server/admin_task_handlers.rs`
  - Add `fn recover_stale_tasks_on_startup() -> Result<usize, _>`: open
    `open_deployment_sites_sqlite()`, run
    `UPDATE {TABLE_NAME} SET status='Failed', error='admin 重启中断', updated_at=?
     WHERE status IN ('Pending','Running')`, return affected row count.
  - Call it in `create_admin_task_routes()` right after the
    `ensure_admin_tasks_table` block; log recovered count, log-and-continue on error.

## Risks

- Tasks that were genuinely running in another admin instance sharing the same SQLite
  file would be falsely failed. Current deployment model is one admin per DB file, so
  accepted; revisit only if multi-admin appears.

## Validation

- Static inspection: confirm no other code path inserts tasks before the routes exist.
- Manual: start admin → check log line; create task → kill -9 admin → restart →
  verify task is Failed and site accepts a new task.
- `cargo fmt` on the touched file. No tests, per repository rule.
