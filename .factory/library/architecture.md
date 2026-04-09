# Architecture

Architecture notes for the `/admin` local site-orchestration workbench mission.

## System Shape

- **Backend service**: Rust `web_server` process.
- **Admin entrypoint**: `/admin` and `/admin/` are explicitly served by route wiring in `src/web_server/mod.rs`.
- **Admin assets**: `src/web_server/static/admin/index.html`, `admin.css`, and `admin.js` are served under `/admin/static/*`.
- **Admin API surface**: `/api/admin/*` from `src/web_server/admin_handlers.rs`.
- **Validation instance**: isolated mission `web_server` on `127.0.0.1:3333` only.

## In-Scope Product Surface

This mission covers the local `/admin` workbench only:
- site list and summary cards
- create/edit operator workflow
- detail hydration and entry URL exposure
- runtime summary and status strip
- parse/start/stop/delete lifecycle controls
- parse/db/web log visibility

Out of scope:
- `/console/*`
- deployment-registry-only flows
- SSH / remote precheck / remote automatic deployment

## Persistence vs Runtime Truth

### Persisted configuration truth
- Local admin orchestration records live in SQLite via `managed_project_sites`.
- These records are the source of truth for configuration and identity:
  - `site_id`, project metadata, ports, bind host
  - generated config/runtime/data paths
  - persisted status fields and stored PIDs

### Synthesized runtime truth
- Runtime is **not** sourced from SQLite alone.
- `/api/admin/sites/{id}/runtime` is synthesized from:
  - persisted managed-site record
  - PID checks
  - port occupancy checks
  - recent log snapshots and summaries
- `/admin` hydrates from multiple backend reads:
  - `/api/admin/sites/{id}` for configuration/detail data
  - `/api/admin/sites/{id}/runtime` for live state
  - `/api/admin/sites/{id}/logs` for stream summaries and contents

## Filesystem / Runtime Layout

For each managed site, the orchestration layer generates and uses local runtime artifacts under `runtime/admin_sites/<site_id>/`, including:
- generated config/runtime files
- local data paths
- log directory contents

Concrete operator-visible log streams are:
- parse log
- SurrealDB log
- web_server log

## Lifecycle Model

### Create / update
- Create and update both rewrite site files/config needed for the local runtime.
- Update also resets operator-visible runtime state back to draft/pending semantics.

### Parse / start
- Parse and start are asynchronous spawned workflows.
- Runtime transitions are centralized through backend state updates, not only through browser assumptions.
- Start is a composed flow:
  1. ensure DB readiness / startup path
  2. parse if needed
  3. spawn web_server
  4. probe spawned site readiness before declaring success

### Stop / delete
- Stop clears recorded runtime state and also performs best-effort residual process/port cleanup; workers should not assume ownership-perfect cleanup is already guaranteed.
- Delete currently removes the persisted record and the `runtime/admin_sites/<site_id>` directory for that site.

## Frontend Reality

Current `/admin` behavior already includes:
- list selection
- create/edit hydration
- auto-refresh / manual refresh
- toast-based submission feedback
- detail/runtime/log panel refreshes

The current UI does **not yet fully implement** the target browser-side guardrail matrix from the validation contract. Workers should assume the following are still mission work to complete rather than already-solved architecture guarantees:
- explicit button disablement by state
- duplicate-submission prevention
- richer destructive-action guidance beyond the current delete confirm


## Contract-sensitive Notes

- Validation-contract wording may be stricter than the current shipped `/admin` shell. In particular, workers should consult the contract for required header copy and operator guardrails rather than assuming the current browser text already satisfies them.

## Critical Mission Invariants

- `/admin` must remain a static server-served workbench, not a console SPA.
- Validation evidence must come from the isolated `3333` instance, never from `3100` or `3101`.
- Browser-visible state must converge with `/api/admin/*` payloads after every mutation.
- The current implementation may over-infer runtime from port occupancy; fixing that false-positive behavior is part of this mission, not an already-satisfied invariant.
- Failure diagnosis must align status strip, runtime error surface, and the log stream indicated by runtime activity.
