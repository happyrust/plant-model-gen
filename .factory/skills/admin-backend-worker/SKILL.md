---
name: admin-backend-worker
description: Implements `/api/admin/*` routing, models, orchestration, and runtime/log contract changes for the plant-model-gen admin mission.
---

# admin-backend-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for backend features touching the admin orchestration surface, especially:
- `src/web_server/admin_handlers.rs`
- `src/web_server/managed_project_sites.rs`
- `src/web_server/models.rs`
- `src/web_server/mod.rs`

## Required Skills

- `verification-before-completion` — invoke before claiming the feature is done so the recorded evidence matches the final state.
- `systematic-debugging` — invoke whenever the isolated `3333` instance, runtime polling, or conflict semantics behave unexpectedly.

## Work Procedure

1. Read the feature, mission `AGENTS.md`, `.factory/services.yaml`, and the feature's claimed assertions.
2. Use `ace-tool` first to locate the exact backend surfaces you need.
3. Identify whether the feature changes:
   - route reachability
   - request/response envelope
   - validation/conflict semantics
   - async parse/start/stop orchestration
   - runtime/log payload shape
4. Implement the minimum backend changes needed for the contract.
5. Required verification:
   - `cargo fmt --all`
   - `cargo check --features web_server --bin web_server`
   - run or reuse the mission `web_server` on `127.0.0.1:3333`
   - validate the changed behavior with `curl` / POST requests
   - for async actions, poll `/runtime` and `/logs` until the assertion is actually proven
6. Record payload-level evidence, not just UI observations.
7. Do not use Rust tests for this mission.

## Runtime Guidance

- Reuse shared SurrealDB on `127.0.0.1:8021`; do not stop it.
- Never reuse the unrelated `127.0.0.1:3100` instance as proof for this mission.
- If you create a validation site, prefer unique ports and clean it up when the feature is done unless the next validator explicitly needs it.

## Example Handoff

```json
{
  "salientSummary": "Stabilized the isolated 3333 admin instance and fixed `/api/admin/sites/{id}/runtime` so unrelated port occupants no longer report the managed site as running.",
  "whatWasImplemented": "Updated the admin runtime ownership checks and startup path so the isolated mission instance on 127.0.0.1:3333 boots with the intended config, `/api/admin/sites` responds from that same instance, and `/runtime` now requires managed-site ownership evidence instead of treating any listener on the configured port as this site being online.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo fmt --all",
        "exitCode": 0,
        "observation": "Formatting succeeded."
      },
      {
        "command": "cargo check --features web_server --bin web_server",
        "exitCode": 0,
        "observation": "web_server compiled successfully."
      },
      {
        "command": "curl -sf http://127.0.0.1:3333/api/admin/sites",
        "exitCode": 0,
        "observation": "Returned the expected admin envelope from the isolated mission instance."
      }
    ],
    "interactiveChecks": []
  },
  "tests": {
    "added": []
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature requires console/Vue work or broader product decisions outside `/admin`.
- The isolated `3333` instance cannot expose `/admin` or `/api/admin/*` in a way that matches mission boundaries.
- You discover a contract ambiguity that would invalidate the feature's assertions if guessed.
