# Research: Sidecar Process Lifecycle Reaper

## Evidence: what the leaked processes actually are

A live process snapshot found 23 `aios-database.exe` processes, all in `serve --site-key ... --http-port ... --runtime-dir runtime/admin_sidecars/<key>/` form, idle at 1-2 MB. They are not OS zombies (defunct); they are orphaned long-lived HTTP sidecars.

- Key kinds observed: `db-index:*` (majority), `resolve:*`, `site:*`.
- Duplicate keys: `resolve:c65996e1f9725c8f` had 4 distinct PIDs spawned at different times by different parents.
- Parents span multiple builds/repos: `target/debug`, `dist/.../release/bin`, `runtime/codex_validation/target_19097/debug`, sibling repo `plant-model-gen-lazy-cold-start`; several parents already dead.

## Decision: cleanup-first, no cross-restart reuse

**Decision**: Optimize for eliminating leaks, not for reusing sidecars across restarts.

**Rationale**: The urgent problem is orphan accumulation. Cross-restart reuse requires persisting handles/tokens to disk and is incompatible with kill-on-close (the strongest cleanup primitive). The two goals conflict on the kill-on-close axis.

**Alternatives considered**:
- Persistent registry + reuse across restarts. Rejected for now: adds stale-token/port-collision complexity and blocks kill-on-close.

## Decision: OS-level parent-death binding as backbone

**Decision**: Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; Unix `PR_SET_PDEATHSIG`.

**Rationale**: Only an OS-level binding cleans up on crash, Ctrl-C, and hard kill, which a userspace reaper cannot guarantee. The current `CREATE_NEW_PROCESS_GROUP` only isolates console signals and does nothing for parent death; it is orthogonal and can be kept.

**Alternatives considered**:
- Reaper-only (startup + shutdown). Rejected as sole mechanism: abnormal exits never run the shutdown path; startup reaper only helps on the next start of the same instance.
- Idle-timeout-only. Rejected as sole mechanism: leaves a long leak window and depends on the sidecar staying healthy enough to self-exit.

## Decision: ownership scoping by admin_sidecars root + owner marker

**Decision**: A web_server owns only sidecars whose `--runtime-dir` is under its own `<cwd>/runtime/admin_sidecars/` root; confirm with an `owner.json` marker.

**Rationale**: Multiple builds/repos and the packaged release run on one machine. Killing by process name alone would terminate unrelated production sidecars. Runtime-dir root is already instance-relative (CWD-based), making it a natural ownership boundary.

**Alternatives considered**:
- Kill all `aios-database serve`. Rejected: unsafe, would kill release/sibling sidecars.
- Track only by in-memory registry. Rejected: lost on exit, cannot reap prior-run orphans.

## Decision: PID + start-token verification before kill

**Decision**: Before terminating, verify the PID's process start time matches the recorded start token.

**Rationale**: PIDs are reused by the OS; killing by stale PID could hit an unrelated process. The codebase already has `process_start_token`/`same_sidecar_process` helpers to reuse.

## Decision: idle self-shutdown for all serve sidecars

**Decision**: Add `--idle-timeout-secs` (default 1800) to `aios-database serve`, applied to all serve kinds.

**Rationale**: Defense in depth where Job Object creation is denied and reaper missed a process. Currently only `job:` sidecars self-exit.

**Alternatives considered**:
- No idle timeout. Rejected: removes the third safety net.
- Aggressive short timeout. Rejected by default: could churn frequently-used `db-index`/`resolve` sidecars; keep configurable with a conservative default.

## Decision: extend cleanup coverage to all key kinds

**Decision**: Reaper and `shutdown_site_sidecars` handle `site`, `job`, `db-index`, `resolve`, `scan`, `preview`, `mdb`.

**Rationale**: The observed leak is dominated by `db-index`/`resolve`, which current cleanup ignores entirely.

## Decision: one-off maintenance sweep for pre-existing orphans

**Decision**: Ship `scripts/cleanup_orphan_sidecars.ps1` scoped by admin_sidecars root.

**Rationale**: Orphans created before this feature will not be reaped until their owning instance restarts, and some have no living owner. A scoped manual sweep is the safe immediate remedy.
