---
name: room-verify-worker
description: Owns room-compute CLI evidence, JSON reporting, compare/verify flows, and milestone performance or parity validation.
---

# room-verify-worker

NOTE: Startup and cleanup are handled by the mission runner. This skill defines the worker procedure.

## When to Use This Skill

Use this skill for room-compute validation-facing CLI features in `plant-model-gen`, especially when the mission needs evidence rather than another hot-path refactor. Typical examples:
- stage-level timing and JSON performance reporting for `room compute` and `room compute-panel`
- read-only compare / verify CLI flows
- shared fixture or report contracts used by validators
- persisted-result verification helpers
- milestone gates that prove parity, fallback behavior, or performance deltas

## Work Procedure

1. Read the assigned feature, `mission.md`, the mission `AGENTS.md`, and the feature's `fulfills` assertions before editing.
2. Before using exact-text search, use `ace-tool` first for the initial codebase retrieval pass. Treat `grep`/`rg` only as secondary confirmation tools after `ace-tool`, unless the identifier is already known or the task explicitly requires exhaustive literal matching.
3. Confirm whether the feature is:
   - reporting / observability,
   - compare / verify CLI,
   - persisted-result validation, or
   - milestone-gate evidence.
4. Preserve read-only-by-default semantics for compare/verify features unless the feature explicitly says otherwise.
5. Prefer direct CLI validation over broad test suites:
   - help/usage validation
   - missing/invalid argument validation
   - report generation validation
   - compare/parity validation
   - full `room compute` / `compute-panel` evidence runs only when required by the feature
6. Implement the smallest Rust CLI or reporting changes needed in the command surface and supporting modules.
7. Reuse persisted results or report files when the feature is about validation; do not satisfy a read-only feature by recomputing data on the default path.
8. Run the required validators:
   - `cargo check --release --bin aios-database`
9. Run the narrowest relevant CLI smoke checks for the changed surface, such as:
   - `cargo run --bin aios-database -- room --help`
   - `cargo run --bin aios-database -- room compute --help`
   - `cargo run --bin aios-database -- room compute-panel --help`
   - `cargo run --bin aios-database -- room verify-json --help`
10. When the feature claims milestone evidence, capture before/after timing or JSON-report deltas and name the exact files or commands used.
11. Record whether the feature stayed read-only where required and whether any evidence gaps came from DB/data availability.

## Implementation Guardrails

- Do not implement `verify-json` by calling `room_compute_panel_mode` or recomputation-heavy helpers.
- Do not write `room_relate` or `room_panel_relate` from the default verification path.
- Keep rebuild/index repair behavior opt-in and explicit if the feature requires it.
- Do not claim performance success from logs alone when the feature also promised JSON reporting; capture both.
- Prefer focused helpers over broad refactors; this repo is already dirty.

## Example Handoff

```json
{
  "salientSummary": "Added stage-level JSON reporting plus read-only compare output for the room CLI and used them to produce milestone evidence without changing compute semantics.",
  "whatWasImplemented": "Updated the room CLI to emit machine-readable stage timing reports for `room compute` and `room compute-panel`, extended the read-only verification surface to produce compare output that can diff expected versus actual results, and wired the report schema so milestone validators can consume consistent before/after evidence. The default compare path remained read-only and did not invoke recompute or write helpers.",
  "whatWasLeftUndone": "Did not modify the hot-path execution logic itself; this feature only changed evidence and verification surfaces.",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo check --release --bin aios-database",
        "exitCode": 0,
        "observation": "Release CLI target compiles after the reporting and compare changes."
      },
      {
        "command": "cargo run --bin aios-database -- room --help",
        "exitCode": 0,
        "observation": "The room CLI lists the expected verification-facing subcommands and flags."
      },
      {
        "command": "cargo run --bin aios-database -- room verify-json --help",
        "exitCode": 0,
        "observation": "The verification command advertises the expected read-only inputs and report flags."
      }
    ],
    "interactiveChecks": []
  },
  "tests": {
    "added": [
      {
        "file": "src/cli_modes.rs",
        "cases": [
          {
            "name": "room_compare_report_is_read_only_by_default",
            "verifies": "The compare/report path does not mutate persisted room relations on its default execution path."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature requires violating read-only-by-default verification semantics.
- The best implementation path appears to depend on broader runtime/service changes beyond the Rust CLI or approved mission boundaries.
- Validation cannot distinguish missing compute coverage from real mismatches without a product decision.
