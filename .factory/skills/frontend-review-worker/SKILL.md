---
name: frontend-review-worker
description: Implements review UI, embed-mode recovery, role landing, and viewer-location features in plant3d-web.
---

# frontend-review-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features in `plant3d-web` that change embed-mode entry behavior, role-based landing, task/review-data restoration, task/record save UI, viewer loading, or model location from task/opinion/attachment context.

## Work Procedure

1. Read the assigned feature, `mission.md`, `AGENTS.md`, and the referenced validation assertions before editing.
2. Inspect the relevant Vue components, composables, stores, API adapters, and any existing tests/e2e coverage for the affected behavior.
3. Add or extend focused tests first (red): component/store tests for state recovery and UI landing logic; if the feature is viewer-related, add the smallest observable test seam available.
4. Implement the front-end changes needed to satisfy the feature while preserving existing review flows outside the mission scope.
5. For role-landing or recovery work, prove a unique UI landing state exists (specific panel, CTA, or visible state), not just a generic page load.
6. For model-location work, prove the viewer receives the correct model references from task/opinion/attachment context and show what happens on failure.
7. Run focused front-end tests during iteration, then run the front-end validators from `.factory/services.yaml`.
8. Manually verify the user-visible flow in a browser when the feature changes navigation, restoration, or viewer behavior.

## Example Handoff

```json
{
  "salientSummary": "Implemented role-based landing for embedded review opens and restored saved task context on reopen. Verified Vue tests plus a manual browser check that designer and reviewer land on distinct panels for the same form_id.",
  "whatWasImplemented": "Updated the embed-mode startup flow to restore the existing task context from the current form_id, route designers to the draft/edit workspace, route reviewers to the review workspace, and keep the visible landing state stable across refreshes.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "npm run test -- review",
        "exitCode": 0,
        "observation": "Targeted review UI tests passed for embed restore and role landing cases."
      },
      {
        "command": "npm run type-check",
        "exitCode": 0,
        "observation": "No type errors after the embed-mode and store changes."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Open the same form_id as a designer in the browser",
        "observed": "The app landed directly on the draft/edit panel with the draft action visible."
      },
      {
        "action": "Open the same form_id as a reviewer in the browser",
        "observed": "The app landed directly on the review workspace with review actions visible and the same task lineage."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "src/components/review/__tests__/embed-role-landing.spec.ts",
        "cases": [
          {
            "name": "designer lands on draft workspace",
            "verifies": "A designer opening an embedded form lands on the draft/edit UI with the correct CTA visible."
          },
          {
            "name": "reviewer lands on review workspace",
            "verifies": "A reviewer opening the same form lands on the review workspace instead of the draft workspace."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The required front-end behavior depends on backend payloads that do not yet exist or are contradictory.
- The feature needs cross-repo wiring or test harness changes that are larger than the current feature scope.
- Viewer behavior cannot be made observable enough to satisfy the validation assertion without new integration support.
