# Implementation Plan

## Approach

Client side: per-key async mutex map for single-flight spawn. Server side: reuse the
existing oneshot shutdown channel from `--shutdown-after-job`, generalized into an
activity-reset idle timer driven by an axum middleware touch point.

## Files

- `src/web_server/parse_sidecar_client.rs`
  - Add `fn spawn_locks() -> &'static Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>`.
  - In `ensure_sidecar`: fetch/create the per-key lock, hold it across
    re-check → `spawn_sidecar` → registry insert; drop the global registry lock while
    spawning so other keys proceed.
  - Pass `--idle-shutdown-ms <value>` for non-job keys when env/config enables it
    (default 900_000; skip the flag entirely when set to 0).
- `src/parse_sidecar.rs`
  - `ParseSidecarOptions` + `ParseSidecarState`: add `idle_shutdown_ms: u64`.
  - Add `last_activity: Arc<Mutex<Instant>>` (or atomic millis); touch it from an
    axum `middleware::from_fn` wrapping all routes after `authorize`, and on WS
    connect/message; mark busy while a CLI job is running (reuse job registry).
  - Spawn a watchdog task: tick every 30s; if idle ≥ threshold and no running job,
    fire the existing shutdown oneshot (same path as `schedule_shutdown_after_job`).
- `src/main.rs`
  - `serve` subcommand: add `--idle-shutdown-ms` arg, plumb into options
    (mirror the existing `--shutdown-after-job` / delay flags at main.rs:957-1145).

## Risks

- Idle timer vs long WS subscriptions: treat any open WS as activity to avoid killing
  a sidecar a UI is watching.
- Per-key lock map grows unbounded; prune entries after spawn completes (map is tiny,
  prune is one `remove` in the same critical section).

## Validation

- Static inspection of lock ordering: per-key lock → registry lock, never nested in
  reverse, to avoid deadlock.
- Manual: parallel scan requests → single process; idle expiry respawn; job in
  progress not killed.
- `cargo fmt`. No tests, per repository rule.
