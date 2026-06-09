# Feature Specification: Managed Site Process Lifecycle Closure

## User Need

Admin managed-site operations must leave no owned `aios-database`, `web_server`, `surreal`, or viewer processes behind after parsing, generation, stop, delete, or failure paths. Cleanup must avoid killing unrelated user processes.

## Scope

- Managed site parse/generate sidecar lifecycle.
- Site stop/delete cleanup of site-owned sidecars and child processes.
- Cancellation behavior for `aios-database -c <config>` jobs.
- Startup/failure cleanup for managed SurrealDB and site web server.

## Requirements

1. Stopping a site cancels any active parse/generate sidecar job and waits briefly for terminal state.
2. Stopping or deleting a site terminates sidecars owned by that site, including `site:<site_id>` and job sidecars associated with parse/generate keys.
3. Cancelling a sidecar CLI job terminates the direct child and its descendant processes where the platform supports process-tree termination.
4. Managed `surreal` and site `web_server` processes remain protected by PID plus start-token checks.
5. Cleanup must not terminate the global admin `web_server` or unrelated SurrealDB instances.
6. Failed startup paths must unregister killed managed process records when the kill path bypasses guarded cleanup.

## Non-Goals

- Do not add a new task cancellation UX.
- Do not change deployment semantics or site configuration schema.
- Do not run Rust tests or compile test targets.

## Acceptance Criteria

- A stopped site has no registered DB/web/viewer process records and no site-owned sidecar handle in the admin process.
- A cancelled parse/generate job kills the job process tree instead of only the direct process.
- Deleting a running site first performs the stop cleanup and refuses deletion if ports are still occupied by external processes.
- Existing global admin web server and unrelated SurrealDB processes are not targeted by site stop/delete.
