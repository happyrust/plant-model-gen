# Tasks: Room Tree Compute And Display

**Input**: Design documents from `specs/020-room-tree-compute-display/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/room-tree-api-contract.md`, `quickstart.md`

**Validation Rule**: Do not run `cargo test` for `web_server`. Use running-service HTTP/GET/POST, CLI/json, frontend smoke, and artifact inspection.

## Phase 1: Current-State Inventory

- [x] T001 Review backend room tree route registration in `src/web_api/mod.rs`
- [x] T002 Review backend room tree query and DTO logic in `src/web_api/room_tree_api.rs`
- [x] T003 Review frontend API client in `../plant3d-web/src/api/genModelRoomTreeApi.ts`
- [x] T004 Review frontend room tree view model in `../plant3d-web/src/composables/useRoomTree.ts`
- [x] T005 Review frontend model tree tab wiring in `../plant3d-web/src/components/model-tree/ModelTreePanel.vue`

## Phase 2: Spec Kit Documentation

- [x] T006 Create feature spec in `specs/020-room-tree-compute-display/spec.md`
- [x] T007 Create implementation plan in `specs/020-room-tree-compute-display/plan.md`
- [x] T008 Create research notes in `specs/020-room-tree-compute-display/research.md`
- [x] T009 Create data model in `specs/020-room-tree-compute-display/data-model.md`
- [x] T010 Create API contract in `specs/020-room-tree-compute-display/contracts/room-tree-api-contract.md`
- [x] T011 Create validation quickstart in `specs/020-room-tree-compute-display/quickstart.md`
- [ ] T012 Complete requirements checklist in `specs/020-room-tree-compute-display/checklists/requirements.md`

## Phase 3: Backend Runtime Validation (P1)

**Goal**: Prove the backend serves room tree data through HTTP against a real computed site.

**Independent Test**: Run the backend against a site with `room_relate` rows and verify all `/api/room-tree/*` endpoints by HTTP.

- [x] T013 [US1] Start web_server with route logging and confirm `/api/room-tree/*` routes are mounted
- [x] T014 [US1] HTTP-verify `GET /api/room-tree/root`
- [ ] T015 [US1] HTTP-verify `GET /api/room-tree/children/room-root`（**阻塞**：站点无 `room_panel_relate` 表；需先跑房间计算。连通性已修复，见 quickstart 附录）
- [ ] T016 [US1] HTTP-verify one room group, one room, and one component group expansion
- [ ] T017 [US3] HTTP-verify `GET /api/room-tree/ancestors/{component_refno}` for a known room-contained component
- [ ] T018 [US1] HTTP-verify `POST /api/room-tree/search`（同 T015：表缺失）
- [x] T019 [US4] HTTP-verify behavior when `room_relate` is empty or missing（**2026-07-16**：children/search 快速返回 `success=false`，`error_message` 含 `table 'room_panel_relate' does not exist`）

## Phase 4: Frontend Display Validation (P1)

**Goal**: Prove `../plant3d-web` can display and operate the room tree against the validated backend.

**Independent Test**: Open `plant3d-web`, point it at the validated backend, switch to the room tree tab, and perform expand/search/selection operations.

- [ ] T020 [US1] Open `plant3d-web` against the validated backend API base
- [ ] T021 [US1] Switch model tree from PDMS tab to room tab and confirm room groups appear
- [ ] T022 [US1] Expand group -> room -> component group -> component in the UI
- [ ] T023 [US2] Verify room-tree visibility toggle affects loaded room subtree
- [ ] T024 [US2] Verify isolate/xray and fly-to on a loaded room subtree
- [ ] T025 [US3] Verify containing-room focus from a known component
- [ ] T026 [US4] Switch back to PDMS tree and confirm PDMS state is not corrupted

## Phase 5: Gap Fixing If Validation Fails

**Goal**: Keep source changes scoped to observed gaps only.

- [ ] T027 [P] If route mounting fails, fix backend route assembly and update `src/web_api/mod.rs`
- [ ] T028 [P] If ID serialization is incompatible, fix normalization in `../plant3d-web/src/api/genModelRoomTreeApi.ts` or backend DTO serialization
- [ ] T029 [P] If missing data state is unclear, improve `room_tree_api.rs` error messages or frontend room-tree empty/error UI
- [ ] T030 [P] If room tree operations leak into PDMS tree state, fix isolation in `ModelTreePanel.vue` or `useRoomTree.ts`

## Dependencies

- Phase 1 and Phase 2 are complete before runtime validation.
- Phase 3 blocks Phase 4 because frontend display needs a verified backend and known sample IDs.
- Phase 5 only runs for gaps found during Phase 3 or Phase 4.

## Parallel Opportunities

- T014, T015, and T018 can be run in one HTTP validation script after backend startup.
- T023 and T024 can be checked in the same frontend smoke session.
- T027-T030 are independent conditional fixes if multiple validation gaps appear.

## MVP Scope

MVP is complete when backend HTTP root/children/search/ancestors work against a computed site and `plant3d-web` shows the room tree tab with expandable room groups and rooms.

## Format Validation

All tasks use checklist format, sequential IDs, story labels where applicable, and concrete file paths.
