---
name: room-performance-worker
description: Implements hot-path Rust performance work for room compute while preserving CLI-visible semantics and producing performance evidence.
---

# room-performance-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for Rust features in `plant-model-gen` that target room-compute hot paths and performance structure rather than data-model materialization. Typical examples:

- spatial index refresh reuse and steady-state fast paths
- cache semantics fixes in `src/fast_model/room_model.rs`
- panel prepared cache work
- room/panel execution model changes
- candidate evaluator preparation, batching, and concurrency

Do **not** use this skill for `inst_mesh_meta` schema/write/read ownership; that belongs to `room-materialization-worker`.

## Work Procedure

1. Read the assigned feature, mission `AGENTS.md`, `validation-contract.md`, and the feature's `fulfills` assertions before editing.
2. Identify the exact hot path the feature is supposed to change and write down the semantic invariant it must preserve.
3. Add the smallest reliable failing regression proof first when practical:
   - focused Rust regression test, or
   - CLI compare/verify expectation that fails before the change
4. Implement only the scoped performance change; do not broaden into unrelated cleanup.
5. Keep the fast path observable:
   - add or preserve stage timing logs
   - emit cache/fallback/concurrency clues when the feature changes those behaviors
6. Run focused validation first, then the required mission validators:
   - `cargo check --release --bin aios-database`
   - targeted CLI help / smoke commands if the feature touches command surfaces
   - the smallest relevant `room compute-panel` or `room compute` validation for the feature
7. Compare before/after evidence whenever the feature claims a speedup.
8. Ensure no orphaned processes remain; stop any long-running command you started.
9. Fill the handoff with exact evidence and any residual risk.

## Example Handoff

```json
{
  "salientSummary": "Refactored the room-compute steady-state path to reuse the existing spatial index and introduced a prepared panel cache. Ran release cargo check plus repeated compute-panel validations showing lower panel preparation cost with unchanged within_refnos.",
  "whatWasImplemented": "Updated src/fast_model/room_model.rs so the steady-state compute path reuses an already-valid spatial index instead of forcing a rebuild on every run, replaced the read-and-evict geometry cache behavior with reusable lookups, and added a prepared panel cache for merged panel AABB and transformed panel mesh state. The CLI logs now report index reuse and prepared-cache hit information needed by milestone validators.",
  "whatWasLeftUndone": "Did not touch inst_mesh_meta schema or materialized detail reads; those remain for the dedicated materialization feature.",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo check --release --bin aios-database",
        "exitCode": 0,
        "observation": "Release CLI target compiles after the performance refactor."
      },
      {
        "command": "cargo run --release --bin aios-database -- room compute-panel --panel-refno 24381/35798",
        "exitCode": 0,
        "observation": "Second run reused the spatial index and prepared panel state; panel result set matched the first run."
      }
    ],
    "interactiveChecks": []
  },
  "tests": {
    "added": [
      {
        "file": "src/fast_model/room_model.rs",
        "cases": [
          {
            "name": "prepared_panel_cache_preserves_room_refnos",
            "verifies": "Prepared panel reuse does not change the computed within_refnos for the covered case."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature requires changing bool/TUBI semantics rather than preserving them.
- The required performance gain appears to depend on `inst_mesh_meta` or another unimplemented upstream feature.
- Validation cannot separate a true regression from missing compare/fixture support.
- The feature would require a new service, persistent daemon, or non-approved infrastructure change.
