# Tasks: BRAN Scoped Generation

**Input**: Design documents from `/specs/013-bran-scoped-generation/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md)

**Tests**: Validation tasks are included because the spec requires HTTP/POST backend validation and frontend automation. Do not create or run `cargo test` tests for web_server.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel because it touches different files and has no dependency on unfinished work.
- **[Story]**: Maps to user stories in `spec.md`.
- Every task includes exact file paths.

## Phase 1: Setup (Shared Context)

**Purpose**: Confirm current request/response, quick-deploy orchestration, generation pipeline, and frontend URL behavior.

- [ ] T001 Review `QuickDeployTestRequest` and `QuickDeployTestResponse` in `src/web_server/models.rs`
- [ ] T002 Review `quick_deploy_test`, `quick_deploy_admin`, and `quick_deploy` flow in `src/web_server/admin_handlers.rs` and `src/web_server/managed_project_sites.rs`
- [ ] T003 [P] Review existing generation pipeline entry points and output wiring in `src/web_server/managed_project_sites.rs`
- [ ] T004 [P] Review existing descendant expansion helpers such as `query_multi_descendants_with_self` usage in `src/cli_modes.rs` and generation/query-provider modules
- [ ] T005 [P] Review plant3d-web URL contract for `show_refno`, `mbd_refno`, and `data_source=parquet` in `D:/work/plant-code/plant3d-web/src/components/dock_panels/ViewerPanel.vue`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Add shared data structures and validation helpers required by every user story.

**CRITICAL**: No user story work should begin until this phase is complete.

- [ ] T006 Add optional `target_root_refno` to `QuickDeployTestRequest` in `src/web_server/models.rs`
- [ ] T007 Add scoped metadata fields such as `target_root_refno`, `scoped_refno_count`, and `scoped_viewer_url` to `QuickDeployTestResponse` in `src/web_server/models.rs`
- [ ] T008 Implement slash/underscore target refno normalization helper in `src/web_server/managed_project_sites.rs`
- [ ] T009 Implement scoped BRAN target validation helper in `src/web_server/managed_project_sites.rs`
- [ ] T010 Implement scoped refno expansion helper reusing existing descendant query semantics in `src/web_server/managed_project_sites.rs`
- [ ] T011 Add request-scoped storage/metadata path for scoped generation target in `src/web_server/managed_project_sites.rs`

**Checkpoint**: Request/response and validation primitives exist, but scoped generation is not yet active.

---

## Phase 3: User Story 1 - Generate One BRAN For Fast Testing (Priority: P1) MVP

**Goal**: Quick deploy can generate only one valid BRAN subtree, such as `2013286704/476`, instead of the whole dbnum.

**Independent Test**: POST quick-deploy with `target_root_refno=2013286704/476`, then inspect response/logs/artifacts for scoped generation marker, scoped count, and reduced output.

### Validation for User Story 1

- [ ] T012 [P] [US1] Add an HTTP/POST validation payload example for `2013286704/476` in `specs/013-bran-scoped-generation/quickstart.md`
- [ ] T013 [P] [US1] Add a JSON response capture path for successful scoped quick-deploy output in `runtime/scoped-bran-generation-success.json`

### Implementation for User Story 1

- [ ] T014 [US1] Call scoped target validation after dbnum resolution and before site creation in `src/web_server/managed_project_sites.rs`
- [ ] T015 [US1] Thread scoped target metadata from quick-deploy request into generation pipeline context in `src/web_server/managed_project_sites.rs`
- [ ] T016 [US1] Apply scoped refno set before or during model generation in `src/web_server/managed_project_sites.rs`
- [ ] T017 [US1] Apply scoped refno set to export/parquet output boundaries in `src/web_server/managed_project_sites.rs`
- [ ] T018 [US1] Log `scoped_generation=true`, target root refno, and scoped refno count in `src/web_server/managed_project_sites.rs`
- [ ] T019 [US1] Ensure scoped output includes BRAN pipe segment/TUBI data required by MBD pipe annotation in `src/web_server/managed_project_sites.rs`

**Checkpoint**: Valid BRAN scoped quick deploy can produce a reduced generated output.

---

## Phase 4: User Story 2 - Reject Invalid Scoped Targets Clearly (Priority: P1)

**Goal**: Invalid scoped targets fail before generation and never fall back to full generation.

**Independent Test**: POST quick-deploy with invalid format, missing target, dbnum mismatch, and non-BRAN refno; verify each response is a clear failure and generation does not start.

### Validation for User Story 2

- [ ] T020 [P] [US2] Add invalid-format HTTP/POST validation command to `specs/013-bran-scoped-generation/quickstart.md`
- [ ] T021 [P] [US2] Add non-BRAN HTTP/POST validation command to `specs/013-bran-scoped-generation/quickstart.md`
- [ ] T022 [P] [US2] Add dbnum-mismatch or missing-target validation command to `specs/013-bran-scoped-generation/quickstart.md`

### Implementation for User Story 2

- [ ] T023 [US2] Return clear parse error for invalid `target_root_refno` in `src/web_server/managed_project_sites.rs`
- [ ] T024 [US2] Return clear not-found error when target does not exist in selected dbnum in `src/web_server/managed_project_sites.rs`
- [ ] T025 [US2] Return clear BRAN-only error when target noun is not `BRAN` in `src/web_server/managed_project_sites.rs`
- [ ] T026 [US2] Ensure validation failures happen before `create_site()` or generation dispatch in `src/web_server/managed_project_sites.rs`

**Checkpoint**: Invalid scoped requests fail safely and do not start full generation.

---

## Phase 5: User Story 3 - Open The Scoped Result In The Frontend Automatically (Priority: P2)

**Goal**: Successful scoped quick deploy returns a viewer URL that loads the same BRAN and triggers MBD pipe annotation.

**Independent Test**: Open `scoped_viewer_url` and verify frontend loads the target BRAN, opens MBD pipe panel, and toggles flow direction.

### Validation for User Story 3

- [ ] T027 [P] [US3] Add frontend Playwright validation script or command notes for `scoped_viewer_url` in `specs/013-bran-scoped-generation/quickstart.md`
- [ ] T028 [P] [US3] Add expected URL assertion for `show_refno=2013286704_476` and `mbd_refno=2013286704_476` in `specs/013-bran-scoped-generation/quickstart.md`

### Implementation for User Story 3

- [ ] T029 [US3] Build `scoped_viewer_url` from site/frontend URL and target refno in `src/web_server/managed_project_sites.rs`
- [ ] T030 [US3] Include `scoped_viewer_url`, canonical `target_root_refno`, and `scoped_refno_count` in `QuickDeployTestResponse` in `src/web_server/models.rs`
- [ ] T031 [US3] Ensure quick-deploy logs include the generated scoped viewer URL in `src/web_server/managed_project_sites.rs`
- [ ] T032 [US3] Verify frontend does not require new route changes by using existing `show_refno` and `mbd_refno` parameters in `D:/work/plant-code/plant3d-web/src/components/dock_panels/ViewerPanel.vue`

**Checkpoint**: Successful scoped quick deploy gives a usable frontend validation URL.

---

## Phase 6: User Story 4 - Preserve Full Generation Defaults (Priority: P2)

**Goal**: Existing quick-deploy behavior remains unchanged when `target_root_refno` is absent.

**Independent Test**: POST existing quick-deploy payload without `target_root_refno` and verify full-generation behavior and response shape remain compatible.

### Validation for User Story 4

- [ ] T033 [P] [US4] Add backward-compat HTTP/POST validation command to `specs/013-bran-scoped-generation/quickstart.md`

### Implementation for User Story 4

- [ ] T034 [US4] Keep scoped validation and metadata inactive when `target_root_refno` is absent in `src/web_server/managed_project_sites.rs`
- [ ] T035 [US4] Ensure normal quick-deploy response serializes scoped fields as absent/null when not scoped in `src/web_server/models.rs`
- [ ] T036 [US4] Ensure scoped target metadata is not persisted into later requests in `src/web_server/managed_project_sites.rs`

**Checkpoint**: Normal quick-deploy requests remain compatible.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Final validation, docs, and evidence capture.

- [ ] T037 Run Rust formatting for modified Rust files using `rustfmt` on `src/web_server/models.rs` and `src/web_server/managed_project_sites.rs`
- [ ] T038 Run a build/check command appropriate for this repo without `cargo test`, recording command and result in `specs/013-bran-scoped-generation/quickstart.md`
- [ ] T039 Run HTTP/POST scoped quick-deploy validation and save response to `runtime/scoped-bran-generation-success.json`
- [ ] T040 Run HTTP/POST invalid target validations and record results in `specs/013-bran-scoped-generation/quickstart.md`
- [ ] T041 Run frontend automation against `scoped_viewer_url` and record result in `specs/013-bran-scoped-generation/quickstart.md`
- [ ] T042 [P] Update `specs/013-bran-scoped-generation/quickstart.md` with final validation record and any environment prerequisites discovered

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on setup review and blocks all stories.
- **US1 (Phase 3)**: Depends on foundational validation and metadata helpers. MVP.
- **US2 (Phase 4)**: Depends on foundational validation helpers; can run alongside US1 implementation after T009.
- **US3 (Phase 5)**: Depends on successful scoped response metadata from US1.
- **US4 (Phase 6)**: Depends on scoped code path being implemented, then verifies absent target behavior.
- **Polish (Phase 7)**: Depends on selected implementation stories.

### User Story Dependencies

- **US1 (P1)**: Core scoped generation path.
- **US2 (P1)**: Safety gate; should be completed before running expensive scoped generation validation broadly.
- **US3 (P2)**: Frontend loop; depends on scoped success metadata.
- **US4 (P2)**: Compatibility; can be validated after scoped branch is added.

### Parallel Opportunities

- T003, T004, and T005 can run in parallel.
- T012 and T013 can run in parallel.
- T020, T021, and T022 can run in parallel.
- T027 and T028 can run in parallel.
- T033 can be prepared in parallel with US3 validation notes.

---

## Parallel Example: User Story 2

```text
Task: "Add invalid-format HTTP/POST validation command to specs/013-bran-scoped-generation/quickstart.md"
Task: "Add non-BRAN HTTP/POST validation command to specs/013-bran-scoped-generation/quickstart.md"
Task: "Add dbnum-mismatch or missing-target validation command to specs/013-bran-scoped-generation/quickstart.md"
```

## Implementation Strategy

### MVP First

1. Complete setup and foundational helpers.
2. Implement US2 validation safety first enough to prevent accidental full generation.
3. Implement US1 scoped generation/export.
4. Stop and validate scoped quick-deploy with `2013286704/476`.

### Incremental Delivery

1. Add request/response contract and target validation.
2. Add scoped refno expansion and generation/export scoping.
3. Add scoped viewer URL response.
4. Add compatibility and failure validation.
5. Run frontend automated validation.

### Notes

- Do not create or run `cargo test`.
- Do not silently fall back to full generation for invalid scoped targets.
- Do not add generic EQUI/ZONE scoped generation in v1.
- Keep frontend changes minimal by reusing existing URL parameters.
