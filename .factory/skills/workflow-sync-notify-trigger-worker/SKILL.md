---
name: workflow-sync-notify-trigger-worker
description: Trace the backend submit/return handlers to prove when and how `notify_workflow_sync_async` is triggered.
---

# workflow-sync-notify-trigger-worker

Use this skill for the `/submit|return` -> notify trigger path.

## Procedure
1. Read mission `mission.md`, `AGENTS.md`, `validation-contract.md`, and `features.json`.
2. Read `.factory/library/workflow-sync-backend-investigation.md`.
3. Inspect `src/web_api/review_api.rs` and any review workflow helpers used by submit/return.
4. Map:
   - which code branches trigger `notify_workflow_sync_async`
   - which branches skip it
   - what `action`, `task_id`, `operator_id`, and comment/reason values are passed
5. Return a branch map and parameter map with exact file references.
6. Do not modify product code.

## Expected Handoff Shape
- submit/return branch map
- notify parameter mapping
- proven vs unproven notes
