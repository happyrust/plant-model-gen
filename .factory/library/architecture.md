# Architecture

Architecture decisions, boundaries, and implementation notes for this mission.

**What belongs here:** Cross-repo contracts, domain boundaries, persistence rules, key design choices.

---

- This mission spans two codebases:
  - `plant-model-gen`: backend APIs, persistence, workflow, sync
  - `plant3d-web`: embed-mode UI, role landing, viewer/model location, e2e
- `form_id` is the externally visible business key and must remain stable across open, save, submit, sync, and reopen flows.
- `task_id` is the internal workflow/task identity and must be explicitly traceable from the current `form_id`.
- Save semantics are intentionally separated:
  - draft/task save
  - review record/opinion save
  - workflow submit/return
- Model location must be driven by explicit review context references (`refno` / `model_refnos`) rather than inferred viewer state.
- E2E should validate the same business lineage across front-end and back-end rather than treating each API/UI step independently.
- Release architecture for this mission is intentionally split by repository:
  - `plant-model-gen` owns backend tag-driven build and deploy
  - `plant3d-web` owns frontend tag-driven build and deploy
- Production deployment must use versioned release directories with a stable `current` pointer rather than single-path overwrite deploys.
- Runtime model-generation data such as `output/` is not part of automated code deployment and must remain in shared persistent storage.
- Release metadata must be traceable across four surfaces:
  - CI artifacts / logs
  - server release files
  - backend `/api/version`
  - frontend Help/About UI
