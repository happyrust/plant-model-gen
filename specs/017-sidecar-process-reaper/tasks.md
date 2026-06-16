# Tasks: Sidecar Process Lifecycle Reaper

**Input**: Design documents from `/specs/017-sidecar-process-reaper/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/sidecar-reaper-contract.md`

**Tests**: Do not add or run `cargo test` / Rust test-target tests for `web_server`. Validation tasks use a running web_server, process enumeration, restart cycles, and logs.

**Organization**: Tasks grouped by user story for independent implementation and verification.

## Phase 1: Setup (Shared Infrastructure)

- [ ] T001 Confirm current sidecar spawn/cleanup in `src/web_server/parse_sidecar_client.rs` and the spawn detach flags (`isolate_sidecar_process_group`).
- [ ] T002 Confirm web_server startup and graceful-shutdown hook points in `src/web_server/mod.rs`.
- [ ] T003 Confirm `aios-database serve` argument handling and request handling loop in `src/parse_sidecar.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Ownership primitives that all reaping depends on.

- [ ] T004 Define admin_sidecars root resolution helper in `src/web_server/parse_sidecar_client.rs` (`<cwd>/runtime/admin_sidecars`).
- [ ] T005 Implement owner marker write (`owner.json`) on spawn in `src/web_server/parse_sidecar_client.rs` per data-model fields.
- [ ] T006 Implement ownership check helper (runtime-dir under root + PID/start-token verify) in `src/web_server/parse_sidecar_client.rs`, reusing `process_start_token`/`same_sidecar_process`.
- [ ] T007 Add structured reaper logging fields (`phase`, `scope_root`, `scanned`, `killed`, `skipped_not_owned`, `by_kind`) helper in `src/web_server/parse_sidecar_client.rs`.

**Checkpoint**: Ownership scoping and logging usable by all reaping paths.

---

## Phase 3: User Story 1 - No Orphans Survive Parent Death (Priority: P1) MVP

**Goal**: Owned sidecars die when the web_server dies, and stale ones are reaped on startup.

**Independent Test**: Kill web_server abruptly; confirm no owned sidecars remain. Restart; confirm startup reaper cleared leftovers.

- [ ] T008 [US1] Create per-instance Windows Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) at startup in `src/web_server/parse_sidecar_client.rs` (or new `src/web_server/sidecar_process_guard.rs`).
- [ ] T009 [US1] Assign each spawned sidecar to the Job Object on Windows in `spawn_sidecar` (`src/web_server/parse_sidecar_client.rs`).
- [ ] T010 [US1] Add Unix `PR_SET_PDEATHSIG` via `pre_exec` in `spawn_sidecar` (`src/web_server/parse_sidecar_client.rs`).
- [ ] T011 [US1] Implement startup reaper (scan + terminate owned stale sidecars) in `src/web_server/parse_sidecar_client.rs`.
- [ ] T012 [US1] Call startup reaper early in web_server bootstrap in `src/web_server/mod.rs`.
- [ ] T013 [US1] Implement shutdown reaper (terminate in-registry sidecars, bounded timeout) in `src/web_server/parse_sidecar_client.rs`.
- [ ] T014 [US1] Invoke shutdown reaper in the graceful-shutdown path in `src/web_server/mod.rs`.
- [ ] T015 [US1] Run Quickstart Scenarios 1, 2, 3 and record evidence in `specs/017-sidecar-process-reaper/quickstart.md`.

**Checkpoint**: Parent-death and startup leaks eliminated.

---

## Phase 4: User Story 2 - Cleanup Covers All Sidecar Kinds (Priority: P1)

**Goal**: All key kinds are reaped, not just site/job.

**Independent Test**: Spawn all kinds; stop; confirm all terminated.

- [ ] T016 [US2] Generalize key-kind handling in reaper scan/terminate in `src/web_server/parse_sidecar_client.rs` to cover `site`, `job`, `db-index`, `resolve`, `scan`, `preview`, `mdb`.
- [ ] T017 [US2] Extend `shutdown_site_sidecars` coverage and callers in `src/web_server/parse_sidecar_client.rs` and `src/web_server/managed_project_sites.rs` as needed.
- [ ] T018 [US2] Run Quickstart Scenario 5 and record `by_kind` evidence.

**Checkpoint**: No key kind leaks.

---

## Phase 5: User Story 3 - Safe Ownership Boundary (Priority: P1)

**Goal**: Reaping never kills another instance's sidecars.

**Independent Test**: Two instances; reap one; other untouched.

- [ ] T019 [US3] Enforce ownership root scoping in all reaper/terminate calls in `src/web_server/parse_sidecar_client.rs`.
- [ ] T020 [US3] Add owner-marker confirmation before kill in `src/web_server/parse_sidecar_client.rs`.
- [ ] T021 [US3] Run Quickstart Scenario 4 with two distinct-cwd instances and record evidence.

**Checkpoint**: Cross-instance safety proven.

---

## Phase 6: User Story 4 - Idle Sidecars Self-Terminate (Priority: P2)

**Goal**: serve sidecars self-exit after idle timeout.

**Independent Test**: Spawn serve sidecar, idle past timeout, confirm exit.

- [ ] T022 [US4] Add `--idle-timeout-secs` arg (default 1800) to `aios-database serve` in `src/parse_sidecar.rs`.
- [ ] T023 [US4] Track last-request time and spawn an idle-watchdog that exits the process in `src/parse_sidecar.rs`.
- [ ] T024 [US4] Pass `--idle-timeout-secs` for all serve kinds from `spawn_sidecar` in `src/web_server/parse_sidecar_client.rs`.
- [ ] T025 [US4] Run Quickstart Scenario 6 with a short timeout and record evidence.

**Checkpoint**: Idle self-shutdown safety net active.

---

## Phase 7: User Story 5 - Sweep Pre-Existing Orphans (Priority: P3)

**Goal**: Safe one-off cleanup of orphans created before this feature.

**Independent Test**: Run scoped sweep; only in-scope orphans removed.

- [ ] T026 [US5] Create `scripts/cleanup_orphan_sidecars.ps1` with `-Root` scoping per contract.
- [ ] T027 [US5] Run Quickstart Scenario 7 and record output.

**Checkpoint**: Pre-existing orphans removable safely.

---

## Final Phase: Polish & Cross-Cutting Concerns

- [ ] T028 Run `cargo fmt` if Rust files changed.
- [ ] T029 Run `cargo check --features web_server` (no test targets).
- [ ] T030 Update `specs/017-sidecar-process-reaper/quickstart.md` with actual evidence.
- [ ] T031 Update `AGENTS.md` Spec Kit pointer to this plan if it becomes the active feature.

---

## Dependencies & Execution Order

### Phase Dependencies

- Setup (Phase 1): no dependencies.
- Foundational (Phase 2): depends on Setup; blocks all stories.
- US1 (Phase 3): depends on Foundational; MVP.
- US2 (Phase 4): depends on Foundational; can parallel US1 once ownership helpers exist.
- US3 (Phase 5): depends on Foundational; tightly related to US1 (uses same scoping).
- US4 (Phase 6): depends on sidecar server changes; independent of reaper.
- US5 (Phase 7): depends on ownership scoping concept; script-only.
- Polish: after selected stories.

### Parallel Opportunities

- T009 and T010 (Windows vs Unix binding) can be done in parallel.
- T016/T017 (coverage) can proceed alongside US1 once Phase 2 is done.
- T022/T023 (sidecar idle) are in a different file and can parallel reaper work.

## Implementation Strategy

### MVP First

1. Complete Phase 1 and Phase 2.
2. Complete US1 (Job Object/PDEATHSIG + startup/shutdown reaper) - this alone stops the bleeding.
3. Validate Scenarios 1-3 before expanding coverage.

### Incremental Delivery

1. Ship parent-death binding + reaper (US1).
2. Ship all-kinds coverage (US2) and ownership safety (US3).
3. Ship idle self-shutdown (US4).
4. Ship maintenance sweep (US5) and record evidence.

## Notes

- Keep `CREATE_NEW_PROCESS_GROUP` / `process_group(0)`; Job Object / PDEATHSIG are orthogonal.
- Always verify PID + start-token before terminating.
- Never terminate sidecars whose runtime-dir is outside the current instance's admin_sidecars root.
- Do not add cross-restart reuse here; it conflicts with kill-on-close and is a separate feature.
- Do not add `cargo test` / Rust test-target validation for `web_server`.
