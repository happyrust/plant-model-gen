# Tasks: System-Library MDB Dependency Discovery

**Input**: Design documents from `specs/019-system-mdb-dependency-discovery/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/system-mdb-discovery-contract.md`, `quickstart.md`

**Validation Rule**: Do not run `cargo test` for `web_server`. Use `cargo fmt`, `cargo check`, running-service HTTP/POST, CLI/json, and artifact inspection.

## Phase 1: Setup

- [x] T001 Review current MDB discovery behavior in `src/data_interface/mdb_candidates.rs`, `src/parse_sidecar.rs`, and `src/web_server/managed_project_sites.rs`
- [x] T002 Confirm admin request payload shape in `ui/admin/src/views/SitesView.vue` and `ui/admin/src/types/site.ts`
- [x] T003 Prepare a local validation fixture or real project root under `D:\AVEVA\Projects\E3D2.1`

## Phase 2: Foundational

- [x] T004 Define supported MDB source DB types and source priority in `src/data_interface/mdb_candidates.rs`
- [x] T005 Add source evidence fields for MDB candidates in `src/data_interface/mdb_candidates.rs`
- [x] T006 Update endpoint/request documentation comments in `src/parse_sidecar.rs`, `src/web_server/models.rs`, and `src/web_server/admin_handlers.rs`

## Phase 3: User Story 1 - Quick Deploy Finds Dependencies From MDB Name (P1)

**Goal**: MDB-name quick deploy resolves the full project collection and target DB from parsed system-library facts.

**Independent Test**: Submit `mbd_name` plus `search_roots`; verify resolved `projects`, `project_path`, `dbnum`, and `db_file`.

- [x] T007 [US1] Extend system-library parsing iteration from SYST-only to SYST/GLOB/GLB in `src/data_interface/mdb_candidates.rs`
- [x] T008 [US1] Preserve source-priority deduplication for same-project same-MDB candidates in `src/data_interface/mdb_candidates.rs`
- [x] T009 [US1] Ensure `resolve_quick_deploy_mbd_request` fills resolved `projects`, `project_path`, `dbnum`, and `db_file` in `src/web_server/managed_project_sites.rs`
- [x] T010 [US1] Verify quick deploy with `mbd_name` and `search_roots` by HTTP/POST against a running web server

## Phase 4: User Story 2 - Incomplete Or Ambiguous Dependencies Fail Early (P1)

**Goal**: Missing or ambiguous MDB members stop quick deploy before site creation/generation.

**Independent Test**: Use incomplete or duplicate dependency roots and verify failure before deployment creation.

- [x] T011 [US2] Confirm missing member detection reports missing DB details in `src/data_interface/mdb_candidates.rs`
- [x] T012 [US2] Confirm ambiguous member detection lists all candidate file paths in `src/data_interface/mdb_candidates.rs`
- [x] T013 [US2] Ensure quick deploy rejects non-ready MDB candidates in `src/web_server/managed_project_sites.rs`
- [x] T014 [US2] Validate missing dependency failure with a narrowed `search_roots` HTTP/POST request
- [x] T015 [US2] Validate ambiguous dependency failure with duplicate member DB files in the search scope

## Phase 5: User Story 3 - Operators Can Inspect Discovery Evidence (P2)

**Goal**: Candidate responses expose source system-library evidence and member locate evidence.

**Independent Test**: Call MDB candidates endpoint and inspect `source_file`, `source_db_type`, member `source_project`, `file_path`, and `candidates`.

- [x] T016 [US3] Ensure candidate responses include `source_file` and `source_db_type` in `src/data_interface/mdb_candidates.rs`
- [x] T017 [US3] Preserve `syst_file` compatibility for older consumers in `src/data_interface/mdb_candidates.rs`
- [x] T018 [US3] Verify MDB candidates endpoint output against `specs/019-system-mdb-dependency-discovery/contracts/system-mdb-discovery-contract.md`

## Phase 6: User Story 4 - Drawer Path Fill Remains A Convenience Only (P3)

**Goal**: The admin drawer path helper is useful but never claims semantic dependency validation.

**Independent Test**: Fill root/name in the drawer and verify it only updates path fields and scan root.

- [x] T019 [US4] Review helper copy and state updates in `ui/admin/src/components/sites/SiteDrawer.vue`
- [x] T020 [US4] Ensure quick deploy/preview flows, not drawer fill, surface dependency discovery warnings in `ui/admin/src/views/SitesView.vue` and related site components

## Phase 7: Polish & Cross-Cutting

- [x] T021 Run `cargo fmt -- src/data_interface/mdb_candidates.rs src/parse_sidecar.rs src/web_server/models.rs src/web_server/admin_handlers.rs`
- [x] T022 Run `cargo check --bin web_server --no-default-features --features "ws,gen_model,manifold,project_hd,surreal-save,write-to-surrealdb,sqlite-index,web_server,parquet-export,rvm-import"`
- [x] T023 Run admin UI type check/build if UI files changed: `npx vue-tsc -b` or `npm run build` from `ui/admin`
- [x] T024 Update validation notes in `specs/019-system-mdb-dependency-discovery/quickstart.md` if observed payloads differ from the planned contract

## Dependencies

- Phase 1 must complete before all other phases.
- Phase 2 blocks all user-story phases.
- US1 and US2 are both P1 and should be completed before US3/US4.
- US3 can run after US1 because source evidence is returned by the same candidate model.
- US4 can run independently after Phase 2 if UI wording is the only change.

## Parallel Opportunities

- T001, T002, and T003 can run in parallel.
- T011 and T012 can run in parallel after T007.
- T016 and T017 can run in parallel after T005.
- T019 and T020 can run in parallel with backend validation if UI text changes are independent.

## MVP Scope

MVP is Phase 1 + Phase 2 + US1 + US2: MDB-name quick deploy resolves complete dependencies from parsed system libraries and fails early on missing/ambiguous dependencies.

## Format Validation

All tasks use the required checklist format with sequential IDs, story labels where applicable, and concrete file paths.
