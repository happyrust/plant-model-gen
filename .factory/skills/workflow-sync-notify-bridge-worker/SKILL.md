---
name: workflow-sync-notify-bridge-worker
description: Investigate `notify_workflow_sync_async` / `notify_workflow_sync` downstream behavior, target resolution, and fire-and-forget semantics.
---

# workflow-sync-notify-bridge-worker

Use this skill for the notify helper itself.

## Procedure
1. Read mission `mission.md`, `AGENTS.md`, `validation-contract.md`, and `features.json`.
2. Read `.factory/library/workflow-sync-backend-investigation.md`.
3. Inspect the notify helper implementation and any related config/HTTP helper modules.
4. Determine:
   - what downstream URL/path/function is actually called
   - how that target is resolved
   - whether responses are read, logged, or ignored
   - whether retries exist
   - whether the async wrapper is true fire-and-forget
5. Return a referenced bridge-semantics summary and any runtime-only gaps.
6. Do not modify product code.

## Expected Handoff Shape
- downstream target summary
- response-handling summary
- retry/fire-and-forget summary
- runtime verification gaps
