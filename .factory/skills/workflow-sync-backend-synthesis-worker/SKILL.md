---
name: workflow-sync-backend-synthesis-worker
description: Synthesize backend workflow/sync evidence and reconcile it with the prior frontend passive-sync investigation.
---

# workflow-sync-backend-synthesis-worker

Use this skill only after all backend area-analysis workers finish.

## Procedure
1. Read mission `mission.md`, `AGENTS.md`, `validation-contract.md`, and `features.json`.
2. Read `.factory/library/workflow-sync-backend-investigation.md`.
3. Read all completed backend worker handoffs and the prior frontend mission note if referenced by this mission.
4. Produce a final synthesis with sections:
   - confirmed from backend code
   - frontend inference confirmed by backend
   - frontend inference corrected by backend
   - still requires runtime verification
5. Explicitly answer:
   - whether plant3d is only synchronizing data when `workflow/sync` is invoked
   - whether the simulator title change can really be traced to backend `workflow/sync` response assembly
6. Do not introduce unsupported claims.

## Expected Handoff Shape
- concise synthesis summary
- cross-repo confirmation/correction notes
- runtime-gap checklist
