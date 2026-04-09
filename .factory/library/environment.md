# Environment

Environment notes for the `/admin` site-management mission.

## Machine Snapshot

- OS: darwin 25.3.0
- CPU cores: 10
- Memory: 16 GiB
- Validation dry run observed very low free memory; keep browser validation serial.

## Runtime Assumptions

- Mission `web_server` must run on `127.0.0.1:3333`.
- Shared SurrealDB remains on `127.0.0.1:8021` and must not be stopped.
- Existing `127.0.0.1:3100` and `127.0.0.1:3101` instances are off-limits.

## Constraints

- Do not run Rust tests or compile test targets for this mission.
- Validate with running `web_server` + `curl`/POST + browser checks.
- `/console/*` is out of scope.
- Treat current uncommitted repo changes as the baseline to continue from.

## Validation Readiness

- `agent-browser` is installed and available.
- The first milestone must fix the isolated 3333 validation environment so `/admin` and `/api/admin/sites` are both reachable from the same mission instance.
