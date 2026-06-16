# Tasks: Custom Project Output Namespace

**Input**: Design documents from `/specs/016-custom-project-output-namespace/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/custom-project-output-namespace-contract.md`

**Tests**: Do not add or run `cargo test`/Rust test-target tests for `web_server`. Validation tasks use running admin service, HTTP/POST flows, CLI/json, generated TOML, runtime files, and logs.

**Organization**: Tasks are grouped by user story to enable independent implementation and verification.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Confirm the current identity/path behavior and prepare shared terminology.

- [ ] T001 Inspect current identity helpers in `src/web_server/managed_project_sites.rs` and note every caller of source-project vs output-project helpers.
- [ ] T002 Inspect current admin parse DB type defaults in `ui/admin/src/components/sites/parse-db-types.ts` and `ui/admin/src/components/sites/SiteDrawer.vue`.
- [ ] T003 Capture current failing evidence for `quicktest-250160-3-8082` in `specs/016-custom-project-output-namespace/quickstart.md` completion notes or a linked validation log.
- [ ] T004 Capture current model-generation precheck failing evidence for `quicktest-250160-8080`, including the `项目: 9001` log line and exit code 101.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish explicit helper semantics before changing user-story behavior.

- [ ] T005 Define deployment/output identity helper semantics in `src/web_server/managed_project_sites.rs`, including that `project_name` is the collection-level database/directory/external access namespace.
- [ ] T006 Define source E3D identity helper semantics in `src/web_server/managed_project_sites.rs`.
- [ ] T007 Replace ambiguous helper names or add comments in `src/web_server/managed_project_sites.rs` so output consumers cannot accidentally call source identity helpers.
- [ ] T008 Audit all generated-output path consumers in `src/web_server/managed_project_sites.rs` and classify them as deployment-output or source-discovery consumers.

**Checkpoint**: Helpers clearly separate custom deployment identity from source E3D identity, and no helper assumes the deployment project name must match an E3D project name.

---

## Phase 3: User Story 1 - Custom Name Owns Runtime Output (Priority: P1) MVP

**Goal**: Generated runtime/output artifacts use the admin-provided custom project name.

**Independent Test**: Generate/redeploy a site with custom project `9002` and source E3D `AvevaPlantSample`; verify active output and generated `project_name` use `9002`.

### Implementation for User Story 1

- [ ] T009 [US1] Update DbOption generation in `src/web_server/managed_project_sites.rs` so generated `project_name` uses the custom deployment project name.
- [ ] T010 [US1] Update active output root helpers in `src/web_server/managed_project_sites.rs` so `scene_tree`, `parquet`, and generated manifest paths use `output/<custom_project_name>`.
- [ ] T011 [US1] Update parquet validation root selection in `src/web_server/managed_project_sites.rs` to use the custom output namespace.
- [ ] T012 [US1] Update viewer/project URL generation in `src/web_server/managed_project_sites.rs` to use the active custom output project when resolving generated files.
- [ ] T013 [US1] Verify old output folders are not migrated or deleted by changes in `src/web_server/managed_project_sites.rs`.
- [ ] T014 [US1] Run Scenario 1 and Scenario 2 from `specs/016-custom-project-output-namespace/quickstart.md` against a running admin service.

**Checkpoint**: User Story 1 is functional and independently verifiable.

---

## Phase 4: User Story 2 - Preserve E3D Source Project Names (Priority: P1)

**Goal**: Source scanning continues to use original E3D project names and paths.

**Independent Test**: Generated DbOption uses custom `project_name` but keeps E3D source names in `included_projects` and paths in `project_dirs`.

### Implementation for User Story 2

- [ ] T015 [US2] Preserve `included_projects` generation from source E3D project names in `src/web_server/managed_project_sites.rs`.
- [ ] T016 [US2] Preserve `project_dirs` generation from source E3D project paths in `src/web_server/managed_project_sites.rs`.
- [ ] T017 [US2] Validate single-project fallback behavior in `src/web_server/managed_project_sites.rs` when `projects[]` is empty but `associated_project` or `project_path` exists.
- [ ] T018 [US2] Validate multi-project source behavior in `src/web_server/managed_project_sites.rs` so each source E3D entry keeps its own name/path.
- [ ] T019 [US2] Run Scenario 1 from `specs/016-custom-project-output-namespace/quickstart.md` and record `included_projects`/`project_dirs` evidence.

**Checkpoint**: User Story 2 works without depending on CATA closure alignment.

---

## Phase 5: User Story 3 - CATA Closure Uses Active Output Namespace (Priority: P1)

**Goal**: CATA partial parse reads the generated manifest from the custom output namespace and narrows the final CATA parse plan.

**Independent Test**: Re-parse `manual_db_nums=[250160]` and verify manifest path, before/after CATA counts, and final manifest-covered CATA DB list.

### Implementation for User Story 3

- [ ] T020 [US3] Update `cata_manifest_path_for_site` in `src/web_server/managed_project_sites.rs` to resolve through the active custom output namespace.
- [ ] T021 [US3] Update any CATA db-index or scene-tree lookup helper in `src/web_server/managed_project_sites.rs` to use the active custom output namespace.
- [ ] T022 [US3] Ensure `align_parse_plan_cata_with_manifest` in `src/web_server/managed_project_sites.rs` receives the manifest loaded from `output/<custom_project_name>/scene_tree/cata_closure.json`.
- [ ] T022a [US3] Ensure manifest-covered DESI template dependencies are included in the final parse plan rather than dropped by CATA-only type filtering.
- [ ] T023 [US3] Add or update parse log lines in `src/web_server/managed_project_sites.rs` to include manifest path and CATA before/after counts.
- [ ] T024 [US3] Preserve manifest-missing fail-open behavior in `src/web_server/managed_project_sites.rs` and log the fallback reason.
- [ ] T025 [US3] Run Scenario 3 from `specs/016-custom-project-output-namespace/quickstart.md` and record final manifest-covered CATA/DESI dependency DB list evidence.

**Checkpoint**: User Story 3 fixes the observed CATA full-parse regression after successful closure generation.

---

## Phase 6: User Story 5 - Model Generation Precheck Uses Source Projects (Priority: P1)

**Goal**: Model-generation precheck repairs tree/db-meta prerequisites using source E3D projects, not custom output names.

**Independent Test**: Trigger generation for `quicktest-250160-8080` with `project_name=9001` and source `AvevaPlantSample`; verify precheck no longer logs `项目: 9001` as the source project or panics on project path resolution.

### Implementation for User Story 5

- [ ] T026 [US5] Update `auto_generate_indextree` in `src/data_interface/db_meta_manager.rs` to derive source projects from resolvable `included_projects`.
- [ ] T027 [US5] Add fallback/error handling in `src/data_interface/db_meta_manager.rs` for configs with no resolvable source projects.
- [ ] T028 [US5] Replace unchecked `get_project_path(...).unwrap()` calls in parse/tree generation paths in `src/versioned_db/database.rs` with actionable errors.
- [ ] T029 [US5] Ensure precheck logs distinguish output project from source project list.
- [ ] T030 [US5] Rebuild `cargo build --release --bin aios-database` without running tests.
- [ ] T031 [US5] Replace packaged `release/bin/aios-database.exe` in `dist/package/Plant3D-AIOS-win-x64/release` with the rebuilt binary.
- [ ] T032 [US5] Disable automatic room computation by default in `src/web_server/managed_project_sites.rs`, leaving explicit opt-in/manual room compute available.
- [ ] T033 [US5] Rebuild and replace packaged `release/bin/web_server.exe` after the room-compute policy change.
- [ ] T034 [US5] Run Scenario 6 from `specs/016-custom-project-output-namespace/quickstart.md` against the packaged release and record job status/log evidence, including that room compute is skipped unless explicitly enabled.

**Checkpoint**: User Story 5 fixes the observed model-generation precheck regression.

---

## Phase 7: User Story 4 - Prevent Accidental Full CATA Selection (Priority: P2)

**Goal**: Quick deploy/create/edit preserve parse DB type intent and do not silently expand scoped selections to full system parsing.

**Independent Test**: Create, edit, clone, and quick-deploy scoped sites; verify saved `parse_db_types` do not become all supported types unless full system parsing is selected.

### Implementation for User Story 4

- [ ] T035 [US4] Review quick deploy request construction in `src/web_server/managed_project_sites.rs` and ensure scoped/default parse DB type intent is explicit.
- [ ] T036 [US4] Review create/update normalization in `src/web_server/managed_project_sites.rs` so empty legacy values and explicit scoped selections are distinguishable.
- [ ] T037 [US4] Update admin create/edit defaults in `ui/admin/src/components/sites/SiteDrawer.vue` to avoid accidental full-system save for scoped/default flows.
- [ ] T038 [US4] Update parse preset labeling or state handling in `ui/admin/src/components/sites/parse-db-types.ts` if needed to distinguish full system from scoped/custom selections.
- [ ] T039 [US4] Update detail display in `ui/admin/src/components/sites/SiteConfigSections.vue` if needed so admins can identify scoped vs full parsing.
- [ ] T040 [US4] Run Scenario 5 from `specs/016-custom-project-output-namespace/quickstart.md` and record persisted `parse_db_types` evidence.

**Checkpoint**: User Story 4 prevents the secondary full-CATA regression path.

---

## Final Phase: Polish & Cross-Cutting Concerns

**Purpose**: Format, build, and record validation.

- [ ] T041 Run `cargo fmt` if Rust files changed.
- [ ] T042 Run a non-test compile/check command only if it does not compile test targets and is acceptable for the current repo rules; otherwise document why skipped.
- [ ] T043 Run targeted admin UI type/lint check if TypeScript/Vue files changed.
- [ ] T044 Update `specs/016-custom-project-output-namespace/quickstart.md` with actual validation evidence paths/results after implementation.
- [ ] T045 Update `AGENTS.md` Spec Kit pointer if this becomes the active feature plan.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on Setup; blocks implementation stories.
- **User Story 1 (Phase 3)**: Depends on Foundational; MVP.
- **User Story 2 (Phase 4)**: Depends on Foundational; can run in parallel with US1 if helper semantics are stable.
- **User Story 3 (Phase 5)**: Depends on US1 because manifest lookup must use the active output namespace.
- **User Story 5 (Phase 6)**: Depends on US2 source identity preservation; can be validated independently after rebuilding sidecar.
- **User Story 4 (Phase 7)**: Depends on Foundational; can run after US1 or in parallel with US3/US5 if different files are assigned.
- **Polish**: Depends on selected stories being complete.

### User Story Dependencies

- **US1 (P1)**: Required first for active output namespace correctness.
- **US2 (P1)**: Required to avoid breaking source scanning.
- **US3 (P1)**: Requires US1 output namespace fix.
- **US5 (P1)**: Requires US2 source identity semantics and protects model generation precheck.
- **US4 (P2)**: Regression guard; can follow MVP.

### Parallel Opportunities

- T001 and T002 can be done in parallel.
- T014 and T015 can be done in parallel after helper semantics are established.
- T026 and T028 can be done in parallel if `db_meta_manager.rs` and `database.rs` are assigned separately.
- T035, T036, and T037 can be done in parallel if UI files are assigned separately.
- Validation scenarios can be recorded independently after their corresponding story checkpoints.

## Implementation Strategy

### MVP First

1. Complete Phase 1 and Phase 2.
2. Complete US1 and US2 together enough to generate correct DbOption identity split.
3. Complete US3 to fix CATA manifest lookup/alignment.
4. Complete US5 to fix model generation precheck source resolution.
5. Validate Scenarios 1, 2, 3, and 6 before touching parse type UI guardrails.

### Incremental Delivery

1. Ship custom output namespace and source identity preservation.
2. Ship CATA closure manifest lookup/alignment fix.
3. Ship model generation precheck source-resolution fix.
4. Ship parse type selection guardrails.
5. Record quickstart validation evidence and update docs.

## Notes

- Avoid adding fallback behavior that silently reads stale `output/<source_e3d_name>` as active output.
- Do not migrate or delete historical output folders.
- Do not add `cargo test` or Rust test-target validation for `web_server`.
