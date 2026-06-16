# Tasks: Site Project Identity & Parse Configuration

**Input**: Design documents from `/specs/014-site-project-identity-config/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/site-project-identity-contract.md`

**Tests**: Do not add or run cargo tests. Validation tasks use HTTP/admin UI and CLI/json inspection per repository rules.

**Organization**: Tasks are grouped by user story so each story can be implemented and validated independently.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel if assigned to different files or non-overlapping areas.
- **[Story]**: Maps to the user story in `spec.md`.
- Include exact file paths.

## Phase 1: Setup & Inspection

**Purpose**: Confirm current defaults, schema, and UI binding before implementation.

- [ ] T001 Inspect managed site schema/default columns in `src/web_server/managed_project_sites.rs` and document where `cata_partial_parse` and project paths are persisted.
- [ ] T002 Inspect generated DbOption writing in `src/web_server/managed_project_sites.rs` and identify all project-name/database-name fields that must be rewritten after rename.
- [ ] T003 Inspect create/edit/detail UI bindings in `ui/admin/src/components/sites/SiteDrawer.vue` and `ui/admin/src/components/sites/SiteConfigSections.vue` for missing partial parse visibility.
- [ ] T004 Confirm active routes in `src/web_server/admin_handlers.rs` for site update and decide whether rename preview/apply needs new endpoints or a specialized update branch.

---

## Phase 2: Foundational Identity Helpers

**Purpose**: Shared backend behavior that blocks all stories.

- [ ] T005 Implement normalized project identity validation/helper in `src/web_server/managed_project_sites.rs` or `src/web_server/site_project_identity.rs`.
- [ ] T006 Add conflict detection for normalized project identity, reusing/extending `project_name_conflict_with_conn` in `src/web_server/managed_project_sites.rs`.
- [ ] T007 Define project-scoped effective path helpers for runtime/data/output/config while preserving compatibility for old site-id paths in `src/web_server/managed_project_sites.rs`.
- [ ] T008 Ensure `ManagedProjectSite` API payloads in `src/web_server/models.rs` expose enough effective path/parse configuration state for UI display.
- [ ] T009 Run `cargo fmt` if Rust files were edited.

**Checkpoint**: Backend can compute/validate project identity without changing user behavior yet.

---

## Phase 3: User Story 1 - Isolated Database Startup By Project Name (Priority: P1)

**Goal**: New sites and generated configs use project-name-scoped database identity and paths.

**Independent Test**: Create two sites with different project names; confirm each reports isolated data/config paths and generated DbOption project/database identity.

### Implementation

- [ ] T010 [US1] Update create-site path derivation in `src/web_server/managed_project_sites.rs` so new sites use project identity for effective runtime/data/output paths.
- [ ] T011 [US1] Update `write_site_files` / `write_site_files_with_parse_plan` in `src/web_server/managed_project_sites.rs` so generated DbOption files use current `project_name` for E3D database/project identity.
- [ ] T012 [US1] Add create/update validation so duplicate normalized project identities are rejected before runtime directories are created.
- [ ] T013 [US1] Preserve lookup/start compatibility for older site-id-based records in `src/web_server/managed_project_sites.rs`.
- [ ] T014 [US1] Validate via HTTP create/get/start flow and inspect generated config/data paths; record evidence in implementation notes or PR summary.

**Checkpoint**: Newly created sites isolate database startup by project name.

---

## Phase 4: User Story 2 - Visible Partial Parse Configuration (Priority: P1)

**Goal**: Create, edit, preview, and details all expose the same partial parse configuration.

**Independent Test**: Toggle CATA partial parse and dependency parse in create/edit; reload site details and preview values.

### Implementation

- [ ] T015 [P] [US2] Update `ui/admin/src/components/sites/SiteDrawer.vue` to show `auto_parse_related_dbnums` and `cata_partial_parse` controls in create mode.
- [ ] T016 [P] [US2] Update `ui/admin/src/components/sites/SiteDrawer.vue` to prefill and save those controls in edit mode.
- [ ] T017 [US2] Ensure create defaults in `SiteDrawer.vue` keep `cata_partial_parse=true` unless explicitly changed.
- [ ] T018 [P] [US2] Update `ui/admin/src/components/sites/SiteConfigSections.vue` to display dependency partial parse and CATA partial parse state.
- [ ] T019 [P] [US2] Update `ui/admin/src/components/sites/parse-db-types.ts` labels/details if needed to explain when CATA partial parse is effective.
- [ ] T020 [US2] Verify `ui/admin/src/types/site.ts` and `ui/admin/src/api/sites.ts` payload types remain aligned with backend fields.
- [ ] T021 [US2] Validate through admin UI or HTTP payloads that create/edit/preview/detail values are consistent.

**Checkpoint**: Operators can see and change partial parse settings without reading logs.

---

## Phase 5: User Story 3 - Rename Project Name After Deployment (Priority: P2)

**Goal**: Stopped deployed sites can rename project identity safely, with preview, conflict checks, filesystem/config updates, and history preservation.

**Independent Test**: Rename a stopped deployed site, restart it, and verify site id/history persists while project paths/config identity change.

### Implementation

- [ ] T022 [US3] Add backend `ProjectRenamePlan` and related DTOs in `src/web_server/models.rs` or a focused module.
- [ ] T023 [US3] Implement rename blocker checks in `src/web_server/managed_project_sites.rs` for running status, active parse/generate/deploy tasks, process PIDs, conflicts, and file path collisions.
- [ ] T024 [US3] Implement rename preview function in `src/web_server/managed_project_sites.rs` listing affected paths and names.
- [ ] T025 [US3] Add admin handler/routes for rename preview/apply in `src/web_server/admin_handlers.rs` and route registration.
- [ ] T026 [US3] Implement rename apply under existing operation lock: move/rename owned folders where safe, rewrite configs, update SQLite record, preserve site id history.
- [ ] T027 [US3] Add failure handling/rollback or preflight ordering so failed operations do not report mixed-state success.
- [ ] T028 [US3] Update `ui/admin/src/api/sites.ts` with rename preview/apply functions and TypeScript response types in `ui/admin/src/types/site.ts`.
- [ ] T029 [US3] Update `SiteDrawer.vue` or a dedicated rename confirmation UI to require preview/confirmation when `project_name` changes on an existing deployed site.
- [ ] T030 [US3] Update `SiteDetailView.vue`/detail config actions to make deployed-site project rename discoverable.
- [ ] T031 [US3] Validate stopped rename success, running rename blocker, duplicate project-name blocker, and history preservation via HTTP/admin UI.

**Checkpoint**: Project name can be safely renamed after deployment when the site is stopped and idle.

---

## Phase 6: Polish & Documentation

**Purpose**: Cross-story cleanup and verification.

- [ ] T032 [P] Update admin UI copy/help text to explain CATA partial parse and dependency parse interaction.
- [ ] T033 [P] Add/update smoke script `scripts/smoke/site_project_identity_smoke.ps1` if repeatable HTTP validation is useful.
- [ ] T034 Run `cargo fmt` if Rust files changed.
- [ ] T035 Run frontend type/lint/build command appropriate for UI changes if available and not prohibited by repo rules.
- [ ] T036 Execute `quickstart.md` scenarios and capture blockers/residual risks.

---

## Dependencies & Execution Order

- Phase 1 must complete before implementation.
- Phase 2 blocks all user stories.
- US1 and US2 can proceed in parallel after Phase 2 if files do not conflict.
- US3 depends on Phase 2 and benefits from US1 path helpers.
- Polish follows selected user stories.

## MVP First

1. Complete Phase 1 and Phase 2.
2. Complete US1 and US2 for P1 value.
3. Validate create/edit/detail parse config and new-site project isolation.
4. Add US3 rename workflow as P2.
