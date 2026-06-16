# Contract: Sidecar Process Lifecycle Reaper

## Purpose

Define externally verifiable behavior for sidecar spawning binding, owner markers, startup/shutdown reaping, idle self-shutdown, and the maintenance sweep.

## Ownership Scoping Contract

- A sidecar is OWNED by a web_server instance if and only if its `--runtime-dir` path is under that instance's `<cwd>/runtime/admin_sidecars/` root.
- Reaping MUST only target owned sidecars.
- Reaping MUST verify PID plus start-token before terminating; on mismatch it MUST skip and count it as `skipped_not_owned`.

## Spawn Binding Contract

On spawning any sidecar:

- The process MUST be bound to the instance's OS parent-death mechanism:
  - Windows: assigned to a Job Object created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
  - Unix: child sets `PR_SET_PDEATHSIG` to a fatal signal.
- An `owner.json` marker MUST be written into the sidecar runtime dir with the fields defined in the data model.
- Existing `CREATE_NEW_PROCESS_GROUP` (Windows) / `process_group(0)` (Unix) behavior MAY be retained; it is independent of the binding.

### Parent-death behavior

- When the owning web_server exits for ANY reason (graceful, panic, `taskkill /F`, `kill -9`), all owned sidecars MUST be terminated by the OS binding where the binding was successfully established.
- If the binding could not be established, the system MUST log a warning and the sidecar remains subject to reaper + idle timeout.

## Startup Reaper Contract

On web_server startup, before accepting traffic:

- Enumerate `aios-database serve` processes whose `--runtime-dir` is under the current admin_sidecars root.
- Terminate them (graceful then forced), since a freshly started instance has an empty registry and therefore cannot own a still-running sidecar legitimately.
- Emit a structured log with `phase=startup`, `scanned`, `killed`, `by_kind`.

## Shutdown Reaper Contract

On graceful shutdown signal:

- Best-effort terminate all sidecars currently in the in-memory registry, across all key kinds.
- Bounded by a short timeout so shutdown is not blocked indefinitely.
- Emit a structured log with `phase=shutdown`, `killed`, `by_kind`.

## Idle Self-Shutdown Contract

For `aios-database serve`:

- Accept `--idle-timeout-secs <N>`; default 1800 when not provided.
- The sidecar MUST exit on its own after `N` seconds with no inbound requests.
- Applies to all serve kinds, not only `job:`.

## Cleanup Coverage Contract

- `shutdown_site_sidecars` and the reaper MUST handle all key kinds: `site`, `job`, `db-index`, `resolve`, `scan`, `preview`, `mdb`.
- Site stop/delete MUST continue to terminate the stopped/deleted site's associated sidecars.

## Maintenance Sweep Contract

`scripts/cleanup_orphan_sidecars.ps1`:

- Accept `-Root <path>` defaulting to `./runtime/admin_sidecars`.
- Enumerate and terminate `aios-database serve` processes whose `--runtime-dir` is under `-Root`.
- Print each terminated process (pid, key, runtime_dir) and a final count.
- MUST NOT terminate processes whose runtime dir is outside `-Root`.

## Observability Contract

Every reaping pass MUST log: `phase`, `scope_root`, `scanned`, `killed`, `skipped_not_owned`, and `by_kind` counts.
