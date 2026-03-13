---
name: room-verify-worker
description: Implements the Rust CLI post-compute verification flow for `room verify-json`, including shared fixture parsing, persisted-result checks, reporting, and CLI-based validation.
---

# room-verify-worker

NOTE: Startup and cleanup are handled by the mission runner. This skill defines the worker procedure.

## When to Use This Skill

Use this skill for features that modify the Rust CLI in `plant-model-gen` for the post-compute room verification workflow:
- adding `room verify-json` under the existing `room` CLI
- extracting or sharing the JSON fixture contract used by tests and CLI
- implementing read-only verification helpers over persisted room-compute results
- improving case-level reporting, summaries, and CLI-based validation flows

## Work Procedure

1. Read the assigned feature, `mission.md`, the mission `AGENTS.md`, and the feature's `fulfills` assertions before editing.
2. Confirm the command stays post-compute and read-only by default.
3. Prefer direct CLI validation over adding or relying on test commands:
   - help/usage validation
   - required-argument failure validation
   - missing-input-file validation
   - real `room compute -> room verify-json` acceptance when the environment is ready
4. Implement the smallest Rust changes needed in the CLI and supporting modules.
5. Reuse persisted result sources; do not satisfy the feature by calling recompute/save paths from verification.
6. Run the required validators:
   - `cargo check --bin aios-database --quiet`
7. Run lightweight CLI smoke checks:
   - `cargo run --bin aios-database --quiet -- room --help`
   - `cargo run --bin aios-database --quiet -- room verify-json --help`
   - `cargo run --bin aios-database --quiet -- room verify-json`
   - `cargo run --bin aios-database --quiet -- room verify-json --input /tmp/room-verify-missing.json`
8. If the environment is ready, run the two-step acceptance flow with the provided fixture and report exact outcomes.
9. Record whether verification remained read-only and whether any limitations came from DB/data availability.

## Implementation Guardrails

- Do not implement `verify-json` by calling `room_compute_panel_mode` or recomputation-heavy helpers.
- Do not write `room_relate` or `room_panel_relate` from the default verification path.
- Keep rebuild/index repair behavior opt-in and explicit if the feature requires it.
- Prefer focused helpers over broad refactors; this repo is already dirty.

## Example Handoff

```json
{
  "salientSummary": "Added `room verify-json` as a post-compute, read-only CLI verifier for fixture-driven room validation.",
  "whatWasImplemented": "Wired a new `room verify-json --input <file>` subcommand, extracted the shared fixture contract, added read-only persisted-result verification helpers, and improved per-case/summary reporting.",
  "whatWasLeftUndone": "Manual acceptance against a real DB was not completed because the local compute scope did not match the provided fixture.",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo check --bin aios-database --quiet",
        "exitCode": 0,
        "observation": "CLI target compiles after the verification changes."
      },
      {
        "command": "cargo run --bin aios-database --quiet -- room verify-json --help",
        "exitCode": 0,
        "observation": "The new subcommand is visible with the expected flags."
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

- The feature requires violating read-only-by-default verification semantics.
- The best implementation path appears to depend on broader runtime/service changes beyond the Rust CLI.
- Validation cannot distinguish missing compute coverage from real mismatches without a product decision.
