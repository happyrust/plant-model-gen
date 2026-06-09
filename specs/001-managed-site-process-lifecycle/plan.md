# Implementation Plan

## Approach

Use the existing guarded process registry for DB/web/viewer. Extend sidecar management to remember spawned sidecar PIDs and expose site-scoped cleanup helpers. Use process-tree termination for sidecar CLI jobs.

## Files

- `src/web_server/parse_sidecar_client.rs`
  - Track sidecar PID in `SidecarHandle`.
  - Add process-tree kill helper for sidecar server and CLI job child.
  - Add public cleanup function for site-owned sidecars.
- `src/web_server/managed_project_sites.rs`
  - Call sidecar cleanup from `stop_site`.
  - Call sidecar cleanup from delete fallback path.
  - Prefer guarded/unregistering cleanup for failure paths where practical.

## Validation

- Static inspection of changed lifecycle paths.
- `cargo fmt` only if Rust files are edited.
- No test or test compilation, per repository rule.
