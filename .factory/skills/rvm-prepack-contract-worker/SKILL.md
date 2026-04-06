---
name: rvm-prepack-contract-worker
description: Investigates and implements the RVM semantic/export contract across instances_v3, geometry manifest, GLB assets, and backend serveability in plant-model-gen.
---

# rvm-prepack-contract-worker

Use this skill for backend/export features in the RVM delivery prepack mission when the work spans semantic normalization, `instances_v3`, `geometry_manifest`, GLB packaging, or backend static-serving compatibility.

## Procedure
1. Read the assigned feature, `mission.md`, `validation-contract.md`, `features.json`, and mission `AGENTS.md`.
2. Read `.factory/library/rvm-prepack-contract.md` and `.factory/library/user-testing.md`.
3. Identify which layer the feature owns:
   - semantic normalization/debug contract
   - `instances_v3` contract and merge flow
   - geometry/prepack manifest + GLB assets
   - backend HTTP/static serveability
4. Before exact-text search, use `ace-tool` for the initial retrieval pass.
5. Keep edits scoped; do not broaden the export family surface unless the feature explicitly requires convergence.
6. Validate with scoped CLI exports, JSON/manfiest audits, `cargo check`, and `curl` against the mission `web_server` as required by the feature.
7. Record exact output paths, URLs, and assertion evidence in the handoff.

## Guardrails
- Do not rely on broad Rust tests as the primary proof.
- Do not claim frontend viewer correctness from backend-only checks.
- Preserve unrelated dirty changes in `src/main.rs`, `src/cli_modes.rs`, and `src/fast_model/export_model/export_dbnum_instances_v3.rs`.
- Respect the mission runtime contract: SurrealDB on `8021`, `web_server` on `3200`.
