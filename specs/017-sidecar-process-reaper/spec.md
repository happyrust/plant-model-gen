# Feature Specification: Sidecar Process Lifecycle Reaper

**Feature Branch**: `[017-sidecar-process-reaper]`

**Created**: 2026-06-16

**Status**: Draft

**Input**: User description: "分析站点部署，为什么会有这么多僵尸进程没有关闭掉。使用 grill-me skill 帮我分析，并编写 spec kit。"

## Problem Context

Admin-managed deployment spawns long-lived `aios-database serve --site-key ... --http-port ...` sidecar HTTP processes under `runtime/admin_sidecars/<key>/`. On the diagnosed machine, 23 such processes were left running and accumulating. They are not OS zombie/defunct processes; they are orphaned, still-listening sidecar servers that were never reaped.

Observed leak characteristics:

- Sidecars are spawned detached from the parent (Windows `CREATE_NEW_PROCESS_GROUP`, Unix `process_group(0)`), with no OS-level parent-death cleanup, so they survive parent exit.
- The sidecar registry is in-process memory only, so any web_server exit (crash, Ctrl-C, kill, restart) loses all handles while the child processes keep running.
- Only `job:`-keyed sidecars self-terminate (`--shutdown-after-job`); `serve`-style sidecars (`site:`, `db-index:`, `resolve:`, `scan:`, `preview:`, `mdb:`) have no idle timeout and no self-shutdown.
- The only cleanup path (`shutdown_site_sidecars`) runs only on explicit site stop/delete and only targets `site:` and `job:` keys, never the shared hash-keyed `db-index:`/`resolve:`/`scan:`/`preview:`/`mdb:` sidecars.
- web_server graceful shutdown does not reap sidecars at all.
- After restart, the empty in-memory registry causes respawn of a new sidecar for the same key instead of reusing the running one, so duplicates accumulate (e.g., one `resolve:` key had 4 live PIDs).

## Grill-Me Analysis Decisions

| Decision Branch | Recommended Answer | Rationale |
|---|---|---|
| Primary goal: leak elimination vs cross-restart reuse | Cleanup-first; do NOT add cross-restart reuse in this feature. | Reuse requires persisted handles and forbids kill-on-close; the urgent pain is orphan accumulation, not cold-start cost. |
| Ownership identification | A web_server owns only sidecars whose `--runtime-dir` is under its own `<cwd>/runtime/admin_sidecars/` root, plus an owner marker file in each sidecar runtime dir. | Multiple repos/builds/release run on one machine; reaping must never kill another instance's sidecars. |
| Startup reaper | On startup, kill `aios-database serve` processes whose runtime dir belongs to this instance's admin_sidecars root (they cannot be tracked by the fresh empty registry, so they are stale). | Reclaims everything leaked by the previous run of this instance. |
| Shutdown reaper | On graceful shutdown, best-effort kill all sidecars in the in-memory registry. | Covers normal restarts. |
| OS-level guarantee | Bind each spawned sidecar to a per-web_server Windows Job Object with KILL_ON_JOB_CLOSE; on Unix set PDEATHSIG. | Guarantees cleanup even on crash / hard kill, which reaper-only cannot cover. |
| Idle self-shutdown | Add a configurable idle timeout to all serve sidecars (default 1800s). | Third safety net where Job Object is unavailable and reaper missed it. |
| Cleanup coverage | Reaper and `shutdown_site_sidecars` cover all key prefixes, not just `site:`/`job:`. | The actual leaked sidecars are predominantly `db-index:`/`resolve:`. |
| Existing orphans | Provide a one-off maintenance script scoped by admin_sidecars root. | The current 23 orphans predate the fix and need a safe manual sweep. |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - No Orphans Survive Parent Death (Priority: P1)

As an operator running the admin service, I need sidecars to be cleaned up when the web_server that spawned them exits for any reason, so the machine does not accumulate stray `aios-database serve` processes.

**Why this priority**: This is the direct cause of the reported leak; every abnormal exit currently leaks all sidecars.

**Independent Test**: Start the admin service, trigger an operation that spawns sidecars, then kill the web_server process abruptly; verify no `aios-database serve` process owned by that instance remains.

**Acceptance Scenarios**:

1. **Given** a web_server has spawned one or more sidecars, **When** the web_server exits gracefully, **Then** all sidecars it owns are terminated.
2. **Given** a web_server has spawned sidecars, **When** the web_server is hard-killed or crashes, **Then** the OS-level binding terminates the owned sidecars without manual intervention.
3. **Given** owned sidecars survived a prior unclean exit, **When** a new web_server instance with the same admin_sidecars root starts, **Then** the startup reaper terminates those stale sidecars.

---

### User Story 2 - Cleanup Covers All Sidecar Kinds (Priority: P1)

As an operator, I need cleanup to apply to every sidecar kind, not only site/job sidecars, so shared `db-index`/`resolve`/`scan`/`preview`/`mdb` sidecars are also reaped.

**Why this priority**: The diagnosed leak is dominated by `db-index:` and `resolve:` sidecars that current cleanup ignores.

**Independent Test**: Spawn sidecars of several key kinds, run reaper/stop, and verify all kinds are terminated.

**Acceptance Scenarios**:

1. **Given** sidecars of kinds `site`, `job`, `db-index`, `resolve`, `scan`, `preview`, `mdb` are running and owned by this instance, **When** startup or shutdown reaping runs, **Then** all of them are terminated.
2. **Given** a site is stopped or deleted, **When** site cleanup runs, **Then** sidecars associated with that site across all kinds are terminated, and shared sidecars not associated with any running site are also eligible for reaping.

---

### User Story 3 - Safe Ownership Boundary (Priority: P1)

As an operator running multiple builds/repos and a packaged release service on one machine, I need reaping to never kill sidecars owned by a different instance.

**Why this priority**: A naive "kill all aios-database serve" would take down the production release service and sibling repos.

**Independent Test**: Run two admin instances with different working directories; reap one and verify the other's sidecars are untouched.

**Acceptance Scenarios**:

1. **Given** two web_server instances with distinct admin_sidecars roots, **When** instance A reaps, **Then** only sidecars under instance A's admin_sidecars root are terminated.
2. **Given** a sidecar runtime dir contains an owner marker for instance A, **When** instance B reaps, **Then** instance B does not terminate that sidecar.

---

### User Story 4 - Idle Sidecars Self-Terminate (Priority: P2)

As an operator, I need serve sidecars to self-terminate after a configurable idle period so leaks are bounded even when reaping and OS binding both fail.

**Why this priority**: Defense in depth for environments where Job Object creation is unavailable.

**Independent Test**: Spawn a serve sidecar, leave it idle past the timeout, and verify it exits on its own.

**Acceptance Scenarios**:

1. **Given** a serve sidecar with idle timeout configured, **When** it receives no requests for the timeout duration, **Then** it exits.
2. **Given** the idle timeout is configured to a custom value, **When** the sidecar is spawned, **Then** it honors the configured value.

---

### User Story 5 - Sweep Pre-Existing Orphans (Priority: P3)

As an operator, I need a safe one-off way to clean up orphans created before this feature shipped.

**Why this priority**: The current 23 orphans will not be reaped by the new logic until their owning instance restarts, and some have no living owner.

**Independent Test**: Run the maintenance sweep scoped to an admin_sidecars root and verify only those orphans are removed.

**Acceptance Scenarios**:

1. **Given** orphan sidecars under a specified admin_sidecars root, **When** the maintenance sweep runs against that root, **Then** those orphans are terminated and others are left alone.

---

### Edge Cases

- A sidecar PID was reused by an unrelated process after the original exited (verify start-token before killing).
- Job Object creation is not permitted in the environment (fall back to reaper + idle timeout).
- Two instances accidentally share the same admin_sidecars root (working directory collision).
- A sidecar is mid-job when reaping is requested (graceful then forced, consistent with current kill semantics).
- Reaper runs while a healthy in-registry sidecar is actively serving (must not kill in-registry healthy ones at shutdown until shutdown is actually requested).
- Owner marker file is stale/corrupt or left from a previous instance with a reused PID.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST ensure sidecars spawned by a web_server instance are terminated when that instance exits, including crash and hard kill, via an OS-level parent-death binding where available.
- **FR-002**: System MUST reap stale owned sidecars at web_server startup before spawning new ones.
- **FR-003**: System MUST best-effort terminate all in-registry sidecars during graceful shutdown.
- **FR-004**: System MUST scope all reaping to sidecars owned by the current instance, identified by admin_sidecars runtime-dir root and an owner marker.
- **FR-005**: System MUST NOT terminate sidecars owned by a different web_server instance, build, or the packaged release service.
- **FR-006**: System MUST apply cleanup to all sidecar key kinds (`site`, `job`, `db-index`, `resolve`, `scan`, `preview`, `mdb`).
- **FR-007**: System MUST verify process identity (PID plus start-token) before killing to avoid PID-reuse mis-kills.
- **FR-008**: System MUST support a configurable idle self-shutdown for serve sidecars, with a documented default.
- **FR-009**: System MUST write an owner marker into each sidecar runtime dir capturing owner identity (web_server pid, start token, bind port, created_at).
- **FR-010**: System MUST log reaping actions (counts, kinds, scope root) for observability.
- **FR-011**: System MUST provide a maintenance procedure/script to sweep pre-existing orphans scoped to a given admin_sidecars root.
- **FR-012**: System MUST keep current explicit site stop/delete cleanup behavior working and extend it to cover all key kinds.

### Key Entities

- **Sidecar Process**: A spawned `aios-database serve` HTTP process keyed by sidecar key, located under an admin_sidecars runtime dir.
- **Sidecar Registry**: The in-process map of active sidecars for one web_server instance.
- **Owner Marker**: A small per-sidecar runtime-dir record identifying which web_server instance owns the sidecar.
- **Admin Sidecars Root**: The `<cwd>/runtime/admin_sidecars/` directory that scopes ownership for one instance.
- **Reaper**: The startup and shutdown routine that terminates owned/stale sidecars.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: After a web_server exits (graceful, crash, or hard kill), the count of owned `aios-database serve` processes drops to 0 without manual action.
- **SC-002**: After restarting the same instance, no sidecars from the prior run remain, and steady-state sidecar count does not grow across repeated restarts.
- **SC-003**: Reaping never reduces the sidecar count of a different instance/build/release running on the same machine.
- **SC-004**: All seven key kinds are demonstrably reaped by startup and shutdown reaping.
- **SC-005**: An idle serve sidecar exits within its configured timeout window.
- **SC-006**: The maintenance sweep removes only orphans under the specified admin_sidecars root.

## Assumptions

- Sidecars continue to run under `<cwd>/runtime/admin_sidecars/<safe_key>/` and accept a `--runtime-dir` argument that reflects this path.
- Each web_server instance has a distinct working directory in normal operation; shared-cwd collisions are an explicit edge case, not the default.
- Validation uses running web_server + process inspection + logs; repository rules forbid adding/running `cargo test` for `web_server`.
- Cross-restart reuse is explicitly out of scope and may be a separate future feature.
