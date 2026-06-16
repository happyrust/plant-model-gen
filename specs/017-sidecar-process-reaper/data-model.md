# Data Model: Sidecar Process Lifecycle Reaper

## Sidecar Process

A spawned `aios-database serve` HTTP process.

**Attributes**:

- `key`: sidecar key (e.g., `site:<id>`, `job:<hash>`, `db-index:<hash>`, `resolve:<hash>`, `scan:<hash>`, `preview:<hash>`, `mdb:<hash>`).
- `pid`: process id.
- `start_token`: process start time used to detect PID reuse.
- `bind_port`: local HTTP port.
- `token`: bearer token for the sidecar HTTP API.
- `runtime_dir`: `<cwd>/runtime/admin_sidecars/<safe_key>/`.

**Lifecycle states**:

- spawning -> healthy -> (terminated by reaper | killed by Job Object on parent death | self-exit on idle timeout | self-exit after job for `job:`).

## Sidecar Registry (in-process)

The existing `HashMap<String, SidecarHandle>` for one web_server instance.

**Rules**:

- Authoritative only while the owning web_server process is alive.
- Used by shutdown reaper to terminate active sidecars.
- Not relied upon across restarts (startup reaper uses the filesystem/process scan instead).

## Owner Marker

Per-sidecar marker file at `runtime/admin_sidecars/<safe_key>/owner.json`.

**Fields**:

- `owner_pid`: web_server process id that spawned the sidecar.
- `owner_start_token`: web_server process start time.
- `sidecar_pid`: spawned sidecar pid.
- `sidecar_start_token`: spawned sidecar start time.
- `bind_port`: sidecar HTTP port.
- `key`: sidecar key.
- `created_at`: RFC3339 timestamp.

**Rules**:

- Written atomically on spawn.
- Read by reaper and maintenance sweep to confirm ownership scope.
- A marker whose `sidecar_pid`/`sidecar_start_token` no longer matches a live process is stale and may be removed.

## Admin Sidecars Root

The ownership-scoping directory `<cwd>/runtime/admin_sidecars/`.

**Rules**:

- All ownership decisions are scoped to processes whose `--runtime-dir` is under this root.
- Different instances normally have different roots (distinct working directories).

## Reaper Actions (observability)

Structured log fields emitted on each reaping pass.

**Fields**:

- `phase`: `startup` | `shutdown` | `site-stop` | `site-delete` | `maintenance`.
- `scope_root`: admin_sidecars root used.
- `scanned`: number of candidate processes.
- `killed`: number terminated.
- `skipped_not_owned`: number skipped due to ownership/start-token mismatch.
- `by_kind`: counts per key kind.
