# Tasks

- [x] Add per-key spawn locks to `ensure_sidecar` (single-flight, no cross-key serialization).
- [x] Add `--idle-shutdown-ms` to `serve` CLI and `ParseSidecarOptions`.
- [x] Implement activity tracking (authorize touch + WS counter + running-job busy check).
- [x] Implement idle watchdog reusing the oneshot shutdown channel.
- [x] Pass idle flag from `spawn_sidecar` for non-job keys; env override plumbed.
- [ ] Manually verify: duplicate-spawn race gone; idle exit + transparent respawn; job not killed mid-run.
- [x] Format changed Rust files.
- [x] Update CHANGELOG.
