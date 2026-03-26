---
name: integration-worker
description: Aligns cross-repo contracts, local services, workflow sync, and end-to-end validation for the review loop.
---

# integration-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that cross `plant-model-gen` and `plant3d-web`: local port alignment, embed/api wiring, workflow sync semantics, e2e automation, and cross-role/browser verification.

## Work Procedure

1. Read the assigned feature, `mission.md`, `AGENTS.md`, `.factory/services.yaml`, and the feature's `fulfills` assertions before making changes.
2. Before using exact-text search, use `ace-tool` first for the initial codebase retrieval pass. Treat `grep`/`rg` only as secondary confirmation tools after `ace-tool`, unless the identifier is already known or the task explicitly requires exhaustive literal matching.
3. Identify exactly which repository owns each part of the change; avoid broad edits in both repos when only one side needs to change.
4. If the feature changes behavior, add or update automated verification first where practical:
   - targeted integration tests
   - e2e tests
   - service smoke scripts
5. Implement the cross-repo changes needed to make the flow work under the mission's local runtime contract (`3100` backend, `3101` frontend).
6. Prove the same `form_id` lineage across the relevant steps with payload-level evidence, not just screenshots.
7. Run the relevant backend and frontend validators plus the targeted e2e or smoke flow for this feature.
8. Manually exercise at least one browser flow when the feature changes end-to-end behavior or role transitions.
9. Record both success-path and failure-path observations when the feature claims resilience or denial behavior.

## Example Handoff

```json
{
  "salientSummary": "Aligned the local 3100/3101 wiring, updated workflow sync payload expectations, and added e2e coverage for the open-save-submit-reopen-locate loop plus an unauthorized submit failure path.",
  "whatWasImplemented": "Updated the backend and frontend local runtime contract to use 3100/3101, ensured the workflow sync payload returns the saved models/opinions/attachments for the same form_id, and added e2e tests that prove the same form lineage across open, save, submit, reopen, and locate actions.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test --features web_server review_api",
        "exitCode": 0,
        "observation": "Backend workflow and sync tests passed under the updated local runtime contract."
      },
      {
        "command": "npm run test:e2e -- review-flow",
        "exitCode": 0,
        "observation": "The success-path and negative-path review flow tests passed and produced artifacts."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Open the embedded review flow in a browser on 3101 with the backend on 3100",
        "observed": "The app loaded correctly, reused the same form_id lineage, and reached the expected role landing state."
      },
      {
        "action": "Trigger the unauthorized submit path",
        "observed": "The UI showed an explicit permission failure and the task state remained unchanged."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "e2e/review-form-flow.spec.ts",
        "cases": [
          {
            "name": "embedded form flow success path",
            "verifies": "The same form_id flows through open, save, submit, reopen, and locate model behavior."
          },
          {
            "name": "unauthorized submit is rejected",
            "verifies": "A non-owner cannot submit the current workflow node and the UI shows a clear error."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature requires mission boundary changes (different ports, external services, or broader repo edits).
- The local environment cannot support the required end-to-end validation and the limitation is external to the repo.
- The cross-repo contract is still ambiguous enough that an implementation would risk invalidating the validation assertions.
