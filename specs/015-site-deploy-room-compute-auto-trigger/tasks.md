# Tasks: Site Deploy Room Compute Auto Trigger

**Input**: Design documents from `/specs/015-site-deploy-room-compute-auto-trigger/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`

**Tests**: Include targeted helper tests only where existing unit-test coverage is cheap and local. Main validation uses running admin service, sidecar logs, and quickstart flows.

**Organization**: Tasks are grouped by user story to keep MVP behavior independently verifiable.

## Phase 1: Setup

**Purpose**: Establish helper surfaces and log paths without changing lifecycle behavior.

- [ ] T001 Add `room_compute_log_path(site_id)` in `src/web_server/managed_project_sites.rs`.
- [ ] T002 Extend `SidecarCliJobKind` in `src/web_server/managed_project_sites.rs` with `RoomCompute`, including `label()`, `key()`, and `log_path()`.
- [ ] T003 Add room-compute log snapshot entry wherever site logs are collected in `src/web_server/managed_project_sites.rs`.
- [ ] T004 Extend log path resolution and `tail_log()` support for `room-compute` in `src/web_server/managed_project_sites.rs`.
- [ ] T005 Add room-compute completion marker path helpers in `src/web_server/managed_project_sites.rs`.

---

## Phase 2: Foundational Policy Helpers

**Purpose**: Implement side-effect-free trigger and scope logic before wiring lifecycle.

- [ ] T006 Add `auto_room_compute_policy(site: &ManagedProjectSite, caller_flow: ...)` helper in `src/web_server/managed_project_sites.rs`.
- [ ] T007 Add `room_compute_scope(site: &ManagedProjectSite)` helper in `src/web_server/managed_project_sites.rs`.
- [ ] T008 [P] Add focused unit coverage for db-scope derivation if this file already has suitable local tests.
- [ ] T009 Add helper to build `room compute` CLI args from site policy/scope; default to omitting `--keywords`.
- [ ] T010 Verify sidecar capability failure behavior for missing `room compute`/`sqlite-index` support and map it to a blocking error.
- [ ] T011 Verify existing room relation persistence is idempotent for repeated scoped runs, or define required scoped cleanup before auto trigger.

**Checkpoint**: Policy can be reviewed without executing sidecar jobs.

---

## Phase 3: User Story 1 - Automatically Compute Rooms During Deploy (Priority: P1) MVP

**Goal**: Full deploy and generation-then-start run room compute after generation and before successful start/deploy completion.

**Independent Test**: Deploy a generated site and observe `generate -> room compute -> start` order in task status/logs.

### Implementation

- [ ] T012 Implement `run_room_compute_pipeline(site_id: String, caller_flow: ...)` in `src/web_server/managed_project_sites.rs`.
- [ ] T013 Ensure `run_room_compute_pipeline` reloads the latest site and credentials and writes current site files before submitting the CLI job.
- [ ] T014 Submit sidecar CLI job using `run_sidecar_cli_job_with_site_events()` with `SidecarCliJobKind::RoomCompute`.
- [ ] T015 Adjust DB process lifecycle in `run_generation_pipeline()` so room compute connects to the generated site's DB before cleanup, with cleanup on success/failure/cancel.
- [ ] T016 Call `run_room_compute_pipeline(site_id.clone(), caller_flow).await?` after successful `spawn_generation_process()` when policy is enabled.
- [ ] T017 Make room-compute sidecar failure set `ManagedSiteStatus::Failed` and a room-compute-specific `last_error`.
- [ ] T018 Log skip reasons when auto room compute is disabled for a generation flow.
- [ ] T019 Write room-compute completion marker/report on success and failure.
- [ ] T020 Add deploy validation check that requires successful completion marker when auto room compute was expected.

**Checkpoint**: Full deploy blocks on room compute and fails if room compute fails.

---

## Phase 4: User Story 2 - Use The Managed Site's Generation Scope (Priority: P1)

**Goal**: Room compute uses current site config and generation db scope.

**Independent Test**: Deploy with `generate_db_nums=[24383]` and inspect room-compute CLI args/logs.

### Implementation

- [ ] T021 Ensure `config_no_ext` comes from `generation_config_path(&site.site_id)` via `config_path_without_toml`.
- [ ] T022 Pass `site.generate_db_nums` as `--db-nums` when non-empty.
- [ ] T023 Fall back to `site.manual_db_nums` when `generate_db_nums` is empty.
- [ ] T024 Omit `--db-nums` for full-scope generation.
- [ ] T025 Add room-compute log line summarizing selected scope and scope source.
- [ ] T026 Add warning log when `manual_refnos` is present but MVP does not map it to `--refno-root`.
- [ ] T027 Add trigger-matrix handling for local deploy, generate/start, quick deploy, remote local-generation, parse-only, and start-only flows.

**Checkpoint**: Scoped generation does not compute unrelated db nums.

---

## Phase 5: User Story 3 - Make Room Compute Progress Observable (Priority: P1)

**Goal**: Operators can see room-compute progress and failures distinctly.

**Independent Test**: Poll admin task status while room compute runs and inspect log list/detail.

### Implementation

- [ ] T028 Update `runtime_status()`/`current_stage()` so active `RoomCompute` sidecar jobs surface as `room_computing`.
- [ ] T029 Update `apply_runtime_to_task()` in `src/web_server/admin_task_handlers.rs` to show `房间计算中` for deploy/generation tasks when room-compute job is active.
- [ ] T030 Ensure task failure messages preserve room-compute error context.
- [ ] T031 Ensure stop/cancel paths include active `RoomCompute` jobs.
- [ ] T032 Update UI log labels only if backend log snapshot labels are not already rendered generically.

**Checkpoint**: Admin status/logs distinguish room compute from generation/start.

---

## Phase 6: User Story 4 - Preserve Manual Room Compute Flows (Priority: P2)

**Goal**: Existing explicit room compute flows keep current behavior.

**Independent Test**: Run documented CLI validation command outside deploy.

### Implementation

- [ ] T033 Verify no semantic changes are made to `src/cli_modes.rs::room_compute_mode`.
- [ ] T034 Verify no deploy code path depends on `/api/room/compute`.
- [ ] T035 Run or document manual validation for `room compute` and `room compute-panel`.

---

## Phase 7: Validation & Documentation

**Purpose**: Prove the feature works through real lifecycle flows.

- [ ] T036 Run quickstart full deploy validation and capture relevant log/status/marker evidence.
- [ ] T037 Run scoped deploy validation with `generate_db_nums`.
- [ ] T038 Run quick deploy generate-only/start validation.
- [ ] T039 Run remote deploy local-generation validation or explicitly document MVP behavior if not executed.
- [ ] T040 Run skip validation for `gen_spatial_tree=false` or parse-only operation.
- [ ] T041 Run failure validation by forcing a missing room-compute prerequisite.
- [ ] T042 Run zero-matching-room validation or document why sidecar success marker is accepted without row-count assertion.
- [ ] T043 Run repeated scoped generation/room-compute validation for stale relation cleanup.
- [ ] T044 Update any operator documentation that describes managed-site deploy stages.

---

## Dependencies & Execution Order

- Phase 1 before all other work.
- Phase 2 before lifecycle wiring.
- US1 is the MVP and blocks US2/US3 validation.
- US2 can implement after helpers exist and before or alongside US1 wiring.
- US3 depends on `SidecarCliJobKind::RoomCompute`.
- US4 validation can happen after US1 to detect regressions.

## Parallel Opportunities

- T003-T005 can run in parallel after T001/T002.
- T008 can run in parallel with T009.
- T021-T027 can run after scope helper exists and mostly in parallel with observability work.
- T028-T032 can be split from lifecycle execution once the sidecar kind exists.

## Implementation Strategy

1. Build helpers and sidecar kind.
2. Wire room compute into generation pipeline with DB lifecycle handled safely.
3. Add status/log visibility.
4. Validate deploy, scoped deploy, skip, failure, and manual CLI compatibility.
