---
name: room-materialization-worker
description: Owns inst_mesh_meta schema, materialization, fallback reads, and semantic parity for room-compute detail-path work.
---

# room-materialization-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that introduce or evolve the `inst_mesh_meta` materialized detail path used by room compute. Typical examples:

- defining the `inst_mesh_meta` record model
- batch materialization / upsert write paths
- room-compute batch reads from `inst_mesh_meta`
- safe fallback to legacy detail queries
- parity checks for normal instances, bool results, and TUBI

Do **not** use this skill for generic panel-hot-path or concurrency-only refactors unless they are directly part of materialized detail integration.

## Work Procedure

1. Read the assigned feature, mission `AGENTS.md`, `validation-contract.md`, and all `fulfills` assertions before editing.
2. Write down the semantic mapping you must preserve from the legacy detail path:
   - normal instances
   - bool results
   - TUBI
3. Add the smallest parity-first regression proof before implementation when practical.
4. Implement the schema/write/read/fallback change in small slices:
   - schema or record model first
   - write path second
   - read path third
   - fallback / logging last
5. Keep fallback explicit and observable; never silently downgrade to partial semantics.
6. Run the required validators for every slice:
   - `cargo check --release --bin aios-database`
   - the narrowest CLI or compare validation that exercises the changed path
7. Gather parity evidence, not just compile evidence.
8. Record coverage statistics, fallback behavior, and any semantic uncertainty in the handoff.

## Example Handoff

```json
{
  "salientSummary": "Added the inst_mesh_meta materialized detail path with explicit normal/bool/TUBI fields, batch upsert support, and batch read fallback to the legacy query path. The compare flow showed zero diffs for the covered parity samples.",
  "whatWasImplemented": "Added the inst_mesh_meta record model and supporting materialization helpers, wired batched writes from the generation/export path, and updated room-compute detail loading to prefer batched inst_mesh_meta reads when records are fully materialized. Missing or incomplete batches now log the fallback reason and safely defer to the legacy detail query path without changing room-compute-visible semantics.",
  "whatWasLeftUndone": "Final end-to-end performance gate remains for the milestone validator; this feature focused on correctness and materialized-path integration.",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo check --release --bin aios-database",
        "exitCode": 0,
        "observation": "Release CLI target compiles with the materialized detail path changes."
      },
      {
        "command": "cargo run --release --bin aios-database -- room verify-json --input tests/fixtures/room_compute_validation.json",
        "exitCode": 0,
        "observation": "The covered validation cases stayed green after enabling the materialized detail path in the scoped test configuration."
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
            "name": "inst_mesh_meta_fallback_preserves_legacy_semantics",
            "verifies": "Incomplete materialized batches fall back cleanly and keep the same normal/bool/TUBI-visible results."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The semantic model cannot represent bool/TUBI behavior cleanly without a broader architecture decision.
- The feature would require changing the approved SQLite coarse-filter boundary.
- Fallback behavior is ambiguous enough that the worker would be guessing at correctness.
- Validation data is insufficient to prove parity for the feature's claimed scope.
