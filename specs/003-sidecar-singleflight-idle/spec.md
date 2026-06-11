# Feature Specification: Sidecar Single-Flight Spawn and Idle Shutdown

## User Need

Two residual gaps from spec 001 (confirmed in
`docs/code-review-2026-06-10/INCREMENTAL_REVIEW.md`, W-2 residuals):

1. `ensure_sidecar` is check-then-spawn outside one critical section: concurrent
   requests for the same key spawn duplicate sidecars, and the later registry insert
   overwrites the earlier handle, leaking an untracked process.
2. Non-job sidecars (`site:`, `scan:`, `resolve:`, `preview:`, `db-index:`) never exit
   on their own. They run in their own process group (`process_group(0)`), so an admin
   crash or restart orphans them permanently, and the new admin instance spawns a fresh
   set on demand.

## Scope

- Spawn path and registry in `src/web_server/parse_sidecar_client.rs`.
- Idle self-shutdown inside the sidecar server (`src/parse_sidecar.rs`) plus the
  `serve` CLI flags in `src/main.rs`.

## Requirements

1. Concurrent `ensure_sidecar` calls with the same key resolve to exactly one spawned
   process; per-key locking must not serialize spawns of different keys (health-check
   wait can take seconds).
2. Every non-job sidecar shuts itself down after an idle period with no authorized
   HTTP/WS activity. Default 15 minutes, overridable via
   `ADMIN_SIDECAR_IDLE_SHUTDOWN_MS` (0 disables).
3. Any authorized request or active WS connection resets the idle timer; an active
   CLI job counts as activity for its whole duration.
4. Job sidecars keep the existing `--shutdown-after-job` behavior unchanged.
5. A sidecar that shut down idle is transparently respawned by the next
   `ensure_sidecar` call (existing health-check miss path already covers this).

## Non-Goals

- No persistent sidecar registry across admin restarts (idle shutdown is the bound
  on orphan lifetime; full-system sweep stays out of scope).
- No change to job-cancel process-tree kill semantics from spec 001.
- Do not run Rust tests or compile test targets.

## Acceptance Criteria

- Two parallel first requests for the same scan key produce one `aios-database serve`
  process (observable via process list during the spawn window).
- A `scan:`/`preview:` sidecar with no traffic exits within idle-timeout + grace, and
  a follow-up request transparently respawns it.
- Killing the admin, waiting past the idle timeout, confirms no surviving sidecars
  from the old instance.
- Sidecar serving an in-flight CLI job does not idle-exit mid-job.
