# Feature Specification: Admin Task Crash Recovery

## User Need

After an admin `web_server` crash or hard restart, residual `Pending`/`Running` task rows
in SQLite permanently block the affected site: the in-flight guard in
`create_and_dispatch_site_task` rejects every new task, while `cancel_task` is a stub,
`delete_admin_task` refuses non-terminal tasks, and `retry_task` only accepts `Failed`.
The operator has no recovery path short of editing SQLite by hand.

Origin: finding NEW-1 in `docs/code-review-2026-06-10/INCREMENTAL_REVIEW.md`
(regression introduced by the single-flight check in `43eb161`).

## Scope

- Startup-time recovery of stale admin task rows in the `admin_tasks` SQLite table.
- Recovery runs before the admin task routes are able to serve requests.

## Requirements

1. On admin startup, every task row with `status IN ('Pending', 'Running')` is marked
   `Failed` with `error = 'admin 重启中断'` (admin restart interruption).
2. Recovery executes exactly once per process start, after `ensure_admin_tasks_table`
   succeeds and before `create_admin_task_routes` returns a usable router.
3. Recovered tasks remain visible in the task list with their original metadata
   (name, type, config, site_id) so `retry_task` can resubmit them unchanged.
4. Recovery failures are logged loudly but do not abort server startup.
5. The single-flight guard semantics in `create_and_dispatch_site_task` are unchanged.

## Non-Goals

- No real task cancellation (`cancel_task` stays a stub; spec 001 already excludes
  cancellation UX, and this spec keeps that boundary).
- No new task states (no `Interrupted` enum variant; reuse `Failed` so the existing
  `retry_task` path closes the loop with zero frontend changes).
- No boot-id / heartbeat columns; no schema migration beyond the UPDATE.
- Do not run Rust tests or compile test targets.

## Acceptance Criteria

- Kill -9 the admin while a site task is `Running`; after restart the task shows
  `Failed` with the restart-interruption error, and the same site accepts new tasks.
- A recovered task can be resubmitted via `POST /api/admin/tasks/{id}/retry`.
- Startup log contains one line stating how many stale tasks were recovered (0 included).
