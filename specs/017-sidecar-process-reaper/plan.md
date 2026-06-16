# Implementation Plan: Sidecar Process Lifecycle Reaper

**Branch**: `[017-sidecar-process-reaper]` | **Date**: 2026-06-16 | **Spec**: `specs/017-sidecar-process-reaper/spec.md`

**Input**: Feature specification from `/specs/017-sidecar-process-reaper/spec.md`

## Summary

Stop the accumulation of orphaned `aios-database serve` sidecars by adding ownership-scoped reaping (startup + shutdown), an OS-level parent-death binding (Windows Job Object KILL_ON_JOB_CLOSE / Unix PDEATHSIG), and an idle self-shutdown for serve sidecars. Cleanup is extended to cover all sidecar key kinds and is strictly scoped to the current instance's admin_sidecars root to avoid killing other builds/release sidecars. Cross-restart reuse is out of scope.

## Technical Context

**Language/Version**: Rust backend (web_server + aios-database sidecar binary).

**Primary Dependencies**: existing sidecar client (`src/web_server/parse_sidecar_client.rs`), sidecar server (`src/parse_sidecar.rs`), web_server bootstrap/shutdown (`src/web_server/mod.rs`), `sysinfo` for process enumeration, `tokio::process` for spawning, Windows job-object APIs (via `windows`/`winapi` or std + raw FFI), `libc` for Unix PDEATHSIG.

**Storage**: Per-sidecar owner marker file under `runtime/admin_sidecars/<safe_key>/owner.json`; no schema/database change required.

**Testing**: No `cargo test` / Rust test-target for `web_server`. Validate via running web_server, process enumeration, restart cycles, and logs; `cargo check`/`cargo fmt` only.

**Target Platform**: Windows primary (observed leak), Unix supported via PDEATHSIG fallback.

**Project Type**: Rust web service + spawned CLI/HTTP sidecar.

**Performance Goals**: Reaper scans process table once at startup/shutdown; O(processes). No steady-state polling required for the OS binding path.

**Constraints**: Must never kill sidecars outside the current instance's admin_sidecars root. Must verify PID + start-token before kill. Keep CREATE_NEW_PROCESS_GROUP behavior; Job Object is orthogonal.

**Scale/Scope**: Tens of sidecars per machine across multiple instances.

## Constitution Check

Repository rule forbids `cargo test`/test-target validation for `web_server` and requires running-service validation. This plan follows that. `.specify/memory/constitution.md` has placeholders only.

## Project Structure

### Documentation (this feature)

```text
specs/017-sidecar-process-reaper/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── sidecar-reaper-contract.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
src/
├── web_server/
│   ├── parse_sidecar_client.rs     # spawn binding, owner marker, registry, reaper, expanded cleanup
│   ├── mod.rs                       # startup reaper call + shutdown reaper call
│   └── managed_project_sites.rs     # extend shutdown_site_sidecars coverage callers if needed
└── parse_sidecar.rs                 # serve: add --idle-timeout-secs self-shutdown

scripts/
└── cleanup_orphan_sidecars.ps1      # one-off maintenance sweep scoped to an admin_sidecars root
```

**Structure Decision**: Keep all client-side lifecycle logic in `parse_sidecar_client.rs`; add the idle timeout in the sidecar server `parse_sidecar.rs`; wire startup/shutdown reaping in `mod.rs`. No new module unless the job-object/PDEATHSIG platform code grows large, in which case extract `sidecar_process_guard.rs`.

## Technical Approach

### 1. OS-level parent-death binding (backbone)

- Windows: create one Job Object per web_server process at startup with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; assign every spawned sidecar process to it. When the web_server handle closes (normal exit, crash, kill), the OS terminates all assigned sidecars.
- Unix: in the child `pre_exec`, call `prctl(PR_SET_PDEATHSIG, SIGKILL)` so the child dies when the parent dies; keep `process_group(0)`.
- Keep `CREATE_NEW_PROCESS_GROUP` for console-signal isolation; it is independent of the Job Object.
- If Job Object creation fails, log a warning and rely on reaper + idle timeout.

### 2. Owner marker

- On spawn, write `runtime/admin_sidecars/<safe_key>/owner.json` with: owner web_server pid, owner start_token, sidecar pid, sidecar start_token, bind port, key, created_at.
- Used by reaper to confirm ownership and by the maintenance script to scope sweeps.

### 3. Startup reaper

- At web_server startup (before serving), enumerate processes whose command line is `aios-database serve` and whose `--runtime-dir` is under this instance's `<cwd>/runtime/admin_sidecars/` root.
- These cannot be in the fresh in-memory registry, so they are stale leftovers of this instance; terminate them (graceful then forced), verifying PID + start-token.
- Log the count and kinds.

### 4. Shutdown reaper

- Hook into the existing graceful shutdown path in `mod.rs` (the oneshot receiver). On shutdown signal, best-effort terminate all sidecars in the in-memory registry, then proceed to stop axum.
- Bounded by a short timeout to not block shutdown indefinitely.

### 5. Idle self-shutdown for serve sidecars

- Add `--idle-timeout-secs` to `aios-database serve`; default 1800 (configurable via env or flag).
- The sidecar tracks last-request time; a background task exits the process after the idle window.
- Applies to all serve kinds, not just `job:`.

### 6. Expand cleanup coverage

- Generalize `shutdown_site_sidecars` and the reaper to handle all key prefixes (`site`, `job`, `db-index`, `resolve`, `scan`, `preview`, `mdb`).
- Keep site-association behavior for `site:`/`job:`; for shared kinds, scope by ownership root rather than by site id.

### 7. Maintenance sweep script

- `scripts/cleanup_orphan_sidecars.ps1 -Root <admin_sidecars root>`: enumerate `aios-database serve` whose `--runtime-dir` is under the given root and terminate them, printing what was killed. Default root = `./runtime/admin_sidecars`.

## Validation Plan

- Spawn-then-graceful-exit: trigger sidecars, stop web_server gracefully, confirm 0 owned sidecars remain.
- Spawn-then-hard-kill: trigger sidecars, `taskkill /F` the web_server, confirm Job Object terminated owned sidecars.
- Restart cycle: start → spawn → kill → start again; confirm startup reaper cleared the prior run and steady-state count does not grow.
- Cross-instance safety: run a second instance (different cwd) with its own sidecars; reap instance A; confirm instance B untouched.
- All-kinds: trigger `db-index`/`resolve`/`scan`/`preview`/`mdb`/`site`/`job` sidecars and confirm all are reaped.
- Idle timeout: spawn a serve sidecar, wait past timeout, confirm exit.
- Maintenance script: run scoped sweep and confirm only in-scope orphans removed.
- `cargo fmt`; `cargo check --features web_server` (no test targets).

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Platform-specific Job Object / PDEATHSIG code | Only OS-level binding guarantees cleanup on crash/hard-kill | Reaper-only cannot cover abnormal exits; idle-timeout-only leaves long windows of leaks |
| Owner marker files | Needed to scope reaping safely on shared machines | Killing by process name alone would take down other builds/release |
