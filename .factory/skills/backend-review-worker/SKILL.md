---
name: backend-review-worker
description: Implements backend review, form-id, persistence, and workflow features in plant-model-gen.
---

# backend-review-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features in `plant-model-gen` that change review APIs, auth/verify behavior, form-id and task aggregation, draft persistence, record persistence, attachment metadata, workflow submission/return logic, or sync payload generation.

## Work Procedure

1. Read the assigned feature, `mission.md`, `AGENTS.md`, and the validation assertions referenced in the feature's `fulfills` list.
2. Identify the exact backend modules, routes, storage tables/records, and tests that are relevant before editing anything.
3. Write or extend targeted backend tests first (red). Cover both happy-path and the feature's most important denial/failure branch.
4. Implement the minimum backend changes required to make those tests pass (green), preserving existing behavior outside the feature scope.
5. Verify persistence semantics explicitly:
   - `form_id` lineage
   - `task_id` mapping
   - model reference association
   - workflow state transitions, if applicable
6. Run focused tests during iteration, then run the backend validators defined in `.factory/services.yaml` for this mission scope.
7. If the feature affects runtime APIs, manually exercise the relevant endpoint(s) with curl or equivalent and record observed payload behavior.
8. Do not assume front-end behavior proves backend correctness; capture payload-level evidence in the handoff.

## Example Handoff

```json
{
  "salientSummary": "Implemented form-id based task restore for review open flow and added denial coverage for invalid token/form mismatches. Verified targeted Rust tests and curl reads against the local API payload.",
  "whatWasImplemented": "Added backend aggregation for reading the existing review task by form_id, aligned token verification checks with the embedded open flow, and updated the API tests to cover successful restore plus invalid token/form mismatch rejection.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test --features web_server review_api",
        "exitCode": 0,
        "observation": "Targeted review API tests passed, including the new form-id restore cases."
      },
      {
        "command": "cargo test --features web_server jwt_auth",
        "exitCode": 0,
        "observation": "Token verification tests passed, including mismatched form_id rejection."
      },
      {
        "command": "curl -sf http://127.0.0.1:3100/api/review/...",
        "exitCode": 0,
        "observation": "The API returned the existing task context with the same form_id and expected task lineage."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Read the restored task payload for the same form_id after reopen",
        "observed": "The payload reused the existing task and did not create a duplicate record."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "src/web_api/review_api.rs",
        "cases": [
          {
            "name": "restores existing task by form_id",
            "verifies": "A reopen request with the same form_id reads the existing task context instead of creating a new one."
          },
          {
            "name": "rejects mismatched form_id token claims",
            "verifies": "Invalid token/form combinations are rejected before returning review data."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature needs front-end behavior or viewer behavior to validate correctness and backend-only evidence is insufficient.
- The required data shape conflicts with an existing pending feature's contract.
- The local database/runtime setup cannot provide the data needed to prove the assertion, and mocking it would invalidate the mission requirements.
