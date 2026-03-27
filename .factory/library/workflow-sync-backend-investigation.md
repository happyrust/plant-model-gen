# Backend Workflow/Sync Investigation Note

## Mission Objective

This mission verifies the backend implementation behind `workflow/sync` and `notify_workflow_sync_async` in `plant-model-gen`, so we can confirm or correct the prior frontend mission's passive-sync conclusions.

## Core Questions

1. Where are `workflow/sync` response fields assembled?
2. What is the exact source of `title` in the response?
3. How are `current_node`, `task_status`, `models`, `opinions`, and `attachments` populated?
4. Which `/submit|return` branches trigger `notify_workflow_sync_async`?
5. What does `notify_workflow_sync_async` actually call downstream?
6. Does the notify path read or ignore downstream responses?
7. Which parts still require live runtime verification after code inspection?

## High-Value Evidence Surfaces

### workflow/sync response assembly
- `src/web_api/platform_api/workflow_sync.rs`
- any helper modules it calls for review/task/form aggregation
- shell examples under `shells/platform_api_json/`

### submit/return -> notify trigger chain
- `src/web_api/review_api.rs`
- any review workflow helper functions referenced by submit/return handlers

### notify bridge semantics
- helper functions named around `notify_workflow_sync_async` / `notify_workflow_sync`
- config resolution for external workflow sync path / URL
- any HTTP client helper used by the notify path

## Source-of-Truth Rules

- Backend code is the source of truth for control flow and field assembly.
- Audit docs/comments can support interpretation, but must not override code.
- If code does not prove a runtime guarantee, label it as requiring runtime verification.

## Output Expectations

Every worker handoff should clearly label:
- Proven from backend code
- Supported by docs/comments only
- Requires live runtime verification

The final synthesis must explicitly answer:
- whether the frontend "plant3d is the passive sync side" inference holds
- whether a title change visible in the simulator can really be traced back to backend `workflow/sync` response assembly
