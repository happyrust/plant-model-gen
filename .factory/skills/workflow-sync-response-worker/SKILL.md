---
name: workflow-sync-response-worker
description: Investigate the backend `workflow/sync` endpoint and prove how its response fields are assembled in plant-model-gen.
---

# workflow-sync-response-worker

Use this skill for backend analysis of `workflow/sync` response construction.

## Procedure
1. Read mission `mission.md`, `AGENTS.md`, `validation-contract.md`, and `features.json`.
2. Read `.factory/library/workflow-sync-backend-investigation.md`.
3. Inspect `src/web_api/platform_api/workflow_sync.rs` and any helper modules it calls.
4. Trace the assembly of:
   - `title`
   - `current_node`
   - `task_status`
   - `models`
   - `opinions`
   - `attachments`
5. Produce a field-source table with exact file references and any fallback logic.
6. Clearly mark anything still requiring runtime verification.
7. Do not modify product code.

## Expected Handoff Shape
- field-source table
- proven vs unproven notes
- runtime verification gaps
