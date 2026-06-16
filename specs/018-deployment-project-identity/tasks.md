# Tasks: Deployment Project Identity Over E3D Collection

**Input**: Design documents from `/specs/018-deployment-project-identity/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/deployment-identity-contract.md`

**Tests**: Do not add or run `cargo test` / Rust test-target tests for `web_server`. Validation uses running service HTTP/POST, generated config inspection, the static guard, and admin UI checks.

**Organization**: Tasks grouped by user story.

## Phase 1: Setup

- [ ] T001 Audit outward-identity consumers in `src/web_server/managed_project_sites.rs` and confirm each resolves via `site_deployment_project_name`.
- [ ] T002 Audit source-only consumers (`site_source_project_name`, `site_parse_project_names`, `existing_project_roots`, `site_included_projects_and_dirs`) and confirm they feed only `included_projects`/`project_dirs`/source roots.

---

## Phase 2: Foundational

- [ ] T003 Document the canonical outward vs source helper boundary as code comments in `src/web_server/managed_project_sites.rs` (single source of truth for reviewers).

---

## Phase 3: User Story 1 - Deployment Name Is The Sole Outward Identity (Priority: P1) MVP

- [ ] T004 [US1] Verify DB name / runtime dir / output dir / viewer output_project / parquet root all use `site_deployment_project_name` in `src/web_server/managed_project_sites.rs`; fix any deviation.
- [ ] T005 [US1] Run Quickstart Scenario 1 against a running service and record evidence in `specs/018-deployment-project-identity/quickstart.md`.

---

## Phase 4: User Story 2 - E3D Names Are Source-Only And Independent (Priority: P1)

- [ ] T006 [US2] Verify `included_projects`/`project_dirs` derive from `site.projects` only, independent of `project_name`, in `src/web_server/managed_project_sites.rs`.
- [ ] T007 [US2] Implement coincidence detection (normalized `project_name` equals any E3D source name) producing a non-blocking warning in create/edit/clone/quick-deploy/preview paths in `src/web_server/managed_project_sites.rs`.
- [ ] T008 [US2] Surface the coincidence warning field in responses if needed in `src/web_server/models.rs`.
- [ ] T009 [US2] Run Quickstart Scenarios 2 and 3 and record evidence.

---

## Phase 5: User Story 3 - Independence Regression Guard (Priority: P2)

- [ ] T010 [US3] Create `scripts/guard/deployment_identity_guard.ps1` mirroring `scripts/guard/web_server_parse_boundary_guard.ps1`, asserting outward-identity functions do not call `site_source_project_name` (with an allowlist of source-only sites).
- [ ] T011 [US3] Run the guard on current code (expect pass) and with a deliberate regression (expect fail); record both in `specs/018-deployment-project-identity/quickstart.md`.

---

## Phase 6: User Story 4 - Consistent Model Across Surfaces (Priority: P3)

- [ ] T012 [US4] Present deployment name as outward identity and list E3D collection in `ui/admin/src/components/sites/SiteConfigSections.vue`.
- [ ] T013 [US4] Show coincidence warning and keep uniqueness messaging in `ui/admin/src/components/sites/SiteDrawer.vue`.
- [ ] T014 [US4] Run Quickstart Scenarios 4 and 6 and record evidence.

---

## Final Phase: Polish & Cross-Cutting Concerns

- [ ] T015 Run `cargo fmt` if Rust files changed.
- [ ] T016 Run `cargo check --features web_server` if Rust files changed (no test targets).
- [ ] T017 Run admin UI type-check if UI files changed.
- [ ] T018 Update `specs/018-deployment-project-identity/quickstart.md` with actual evidence.
- [ ] T019 Update `AGENTS.md` Spec Kit pointer if this becomes the active feature plan.

---

## Dependencies & Execution Order

- Setup (Phase 1) -> Foundational (Phase 2) -> US1/US2 (P1) -> US3 (P2) -> US4 (P3) -> Polish.
- US1 and US2 can proceed in parallel after Phase 2 (different concerns: outward verification vs source/independence).
- US3 guard depends on the helper boundary being settled (Phase 2).
- US4 UI can proceed in parallel once warning semantics (US2) are defined.

## Parallel Opportunities

- T004 (outward) and T006 (source) can be done in parallel.
- T010 (guard) is independent of UI tasks.
- T012 and T013 can be done in parallel.

## Implementation Strategy

### MVP First

1. Phase 1 + Phase 2.
2. US1 (outward identity verified) + US2 (independence + warning).
3. Validate Scenarios 1-3.

### Incremental Delivery

1. Ship outward-identity verification + independence warning.
2. Ship regression guard.
3. Ship UI presentation consistency.

## Notes

- Reuse 016's `site_deployment_project_name` as the single outward-identity helper.
- Reuse 014's uniqueness enforcement.
- Do not migrate legacy artifacts; regeneration adopts the deployment identity.
- Do not add `cargo test` / Rust test-target validation for `web_server`.
