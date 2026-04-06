# User Testing

User-testing guidance for the **RVM delivery prepack integration** mission.

---

## Validation Surface

### CLI / artifact surface (primary)

- Tool: `cargo run`, `cargo check`, JSON inspection helpers, and file-level audits
- What to check:
  - scoped semantic debug export
  - scoped `instances_v3` export and merge flow
  - prepack output layout: `manifest.json`, `geometry_manifest.json`, `instances.json`, `geometry_L*.glb`
  - cross-artifact hash/reference integrity

### API / static-file surface (supporting)

- Base URL: `http://127.0.0.1:3200`
- Tool: `curl`
- What to check:
  - `/api/status` health
  - export task endpoints under `/api/export/*`
  - static JSON under `/files/output/*`
  - mesh/GLB assets under `/files/meshes/*`

### Viewer-readiness boundary (out of repo)

- This mission does **not** validate final external viewer rendering.
- In-repo validation stops at backend contract readiness: artifact structure, URL reachability, and route compatibility.

---

## Validation Concurrency

### CLI / artifact validators

- Max concurrent validators: **2**

Rationale: scoped export commands can stress SurrealDB, serialization, and filesystem output. Keep top-level export validators to two at most.

### API / static-file validators

- Max concurrent validators: **1** heavy export/serve validator, or **2** if one is a lightweight GET-only probe

Rationale: the mission `web_server` shares one process/output tree and should avoid overlapping heavy export tasks during validation.

---

## Known Quirks / Gotchas

- Ports `3100` and `3101` are already occupied locally; use mission `web_server` port `3200`.
- Reuse existing SurrealDB on `127.0.0.1:8021`.
- Repo guidance forbids relying on Rust tests as the main validation path for this mission.
- Static JSON file helpers currently serve `.json` and `.parquet` from the `output` tree; mesh assets are served from `/files/meshes` via configured mesh roots.
- The external viewer repo is out of scope; do not claim rendering success from backend-only checks.

## Flow Validator Guidance: cli-artifact

- Use representative scoped exports, not whole-database runs, unless the assigned feature explicitly requires broader evidence.
- Capture exact command lines, output paths, and the specific files inspected.
- When proving stability assertions, run the same export twice and compare only the fields required by the assertion.
- Prefer read/inspect/compare validation helpers over new broad test code.

## Flow Validator Guidance: api-static

- Target only `http://127.0.0.1:3200` for mission validation.
- Record exact status codes, content types, and the requested URLs.
- Keep static fetches scoped to assigned assertions; avoid mutating shared output directories beyond the assigned export flow.
- Use POST/status/download validation only where the feature explicitly claims export task readiness.
