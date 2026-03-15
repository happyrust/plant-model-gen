# User Testing

User-testing guidance for the room-compute 3x mission.

This mission validates through the **real CLI surface**, not a browser UI.

---

## Validation Surface

### CLI Surface

- Primary binary: `aios-database`
- Primary commands:
  - `room compute`
  - `room compute-panel`
  - `room rebuild-spatial-index`
  - compare / verify CLI flows introduced by the mission

### Tooling

- `cargo`
- direct CLI invocation
- log inspection
- JSON report inspection

## Validation Concurrency

### CLI Validators

- Max concurrent validators: **1**

#### Rationale

- The host has enough CPU and memory for parallel processes, but this mission's CLI validations share stateful resources:
  - persisted room relations
  - SQLite coarse-filter artifacts
  - local DB-backed room-compute inputs
- Parallel validators would risk state contamination and misleading compare results.

## Flow Validator Guidance

### Flow Group: baseline-and-regression-cli

- Stay within the CLI boundary.
- Do not open browsers or invent new services.
- Prefer the narrowest command that proves the assigned assertion.
- Capture terminal output and any generated JSON report paths.
- If a flow requires a rebuild or recompute, note it explicitly.

### Flow Group: final-performance-gate

- Run alone; do not overlap with other validators.
- Capture before/after JSON reports and timing logs.
- Record whether the run reused or rebuilt the spatial index.
- If a cold build dominates runtime, separate build cost from command runtime in the report.

## Known Quirks

- Cold Rust builds are expensive; the planning dry run needed about 7m46s before `room --help` returned.
- Repeated validators should try to reuse built artifacts when possible.
- Missing or stale `spatial_index.sqlite` is a valid blocker if the assertion assumes steady-state reuse.
- API status may show `database_connected: false` even when connected (check logs)
- WebSocket updates may have 1-2 second delay (normal)
- Frontend dev server may need restart after major changes

## CLI Surface: `aios-database room verify-json`

- Test with terminal CLI commands only; do not use browser automation for this milestone.
- Primary fixture path: `verification/room/compute/room_compute_validation.json`.
- Service setup: reuse the existing SurrealDB/local DB configuration; do not start or stop a shared instance from validation automation.
- Healthcheck: `lsof -i :8020` or `lsof -i :8009` should detect an existing DB listener before running validation.
- Required operator flow: run `room compute ...` first when validating happy-path persisted data, then run `room verify-json --input <file>`.
- Safe baseline checks that do not require fixture-matching persisted data: `--help`, missing required args, and missing input file.

## Validation Concurrency

- Surface `cli-room-verify-json`: max concurrent validators = 1.
- Reason: all assertions touch the same shared persisted DB state and the same fixture path; parallel runs could interfere with compute coverage and read-only repeatability evidence.

## Flow Validator Guidance: cli-room-verify-json

- Stay within the shared repository at `/Volumes/DPC/work/plant-code/plant-model-gen`.
- Do not create or mutate alternate databases, ports, or fixture files unless explicitly assigned.
- Do not start a second SurrealDB instance and do not stop the shared one.
- Prefer read-only CLI checks first; if a flow requires `room compute`, treat that compute scope as shared global state and serialize it with other validators.
- Save command transcripts and any generated evidence under the assigned evidence directory only.
