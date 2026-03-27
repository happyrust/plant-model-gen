---
name: workflow-sync-backend-scrutiny-validator
description: Final scrutiny validator for the backend workflow/sync investigation; checks references, assertion coverage, and unsupported claims.
---

# workflow-sync-backend-scrutiny-validator

Use this skill after all backend investigation and synthesis features are complete.

## Procedure
1. Read mission `AGENTS.md`, `validation-contract.md`, `validation-state.json`, and `features.json`.
2. Read `.factory/library/workflow-sync-backend-investigation.md`.
3. Review all completed worker handoffs and the final synthesis artifact.
4. Optionally run mission-scoped sanity commands from `.factory/services.yaml` if useful.
5. Validate that:
   - each assertion is covered by completed features via `fulfills`
   - key conclusions cite exact backend file references
   - cross-repo conclusions do not over-claim beyond backend evidence
   - residual unrelated validator instability is recorded as validation debt rather than misreported as a workflow/sync finding
6. Return findings first; if no findings exist, say so explicitly and mention any remaining runtime-only gaps.

## Expected Handoff Shape
- findings-first scrutiny report
- assertion coverage status
- residual runtime gaps / validation debt
