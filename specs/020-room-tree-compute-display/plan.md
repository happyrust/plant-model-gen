# Implementation Plan: Room Tree Compute And Display

**Branch**: `[020-room-tree-compute-display]` | **Date**: 2026-06-22 | **Spec**: `specs/020-room-tree-compute-display/spec.md`

**Input**: Feature specification from `/specs/020-room-tree-compute-display/spec.md`

## Summary

Capture and validate the current room tree path from explicit room computation to web display. Backend room tree routes already exist in `plant-model-gen-cata-closure`; frontend room tree display logic already exists in sibling `../plant3d-web`. The implementation plan is therefore focused on contract documentation, runtime verification, and hardening any gaps found by HTTP/frontend smoke validation.

## Technical Context

**Language/Version**: Rust backend with Axum web server and SurrealDB model database; Vue 3 + TypeScript frontend in `../plant3d-web`.

**Primary Dependencies**:

- Backend: `src/fast_model/room_model.rs`, `src/web_api/room_tree_api.rs`, `src/web_api/mod.rs`, `src/web_server/mod.rs`.
- Frontend: `../plant3d-web/src/api/genModelRoomTreeApi.ts`, `../plant3d-web/src/composables/useRoomTree.ts`, `../plant3d-web/src/components/model-tree/ModelTreePanel.vue`, `../plant3d-web/src/utils/apiBase.ts`.

**Storage**: Existing room relation data in model database, especially `room_relate`. No new persistent schema is planned by this spec.

**Testing**: Do not run `cargo test` for `web_server`. Validate backend through running-service HTTP GET/POST. Validate frontend by type check/build only if changed, plus browser/smoke observation when possible.

**Target Platform**: Windows local model web server plus `plant3d-web` viewer.

**Project Type**: Cross-repository backend/frontend integration spec.

**Performance Goals**: Room tree root/group expansion should remain interactive. Child requests should be bounded by the existing `limit` parameter and avoid loading the full model tree at once.

**Constraints**:

- Room tree display must not trigger room compute.
- Missing room compute is a normal state and must not break PDMS tree.
- Backend API IDs must stay compatible with frontend `normalizeRoomTreeId()`.
- Validation must preserve existing dirty working-tree changes.

## Current Code Findings

- `src/web_api/mod.rs` declares `room_tree_api`, exports `create_room_tree_routes`, merges it in `assemble_stateless_web_api_routes()`, and lists the route paths for diagnostics.
- `src/web_api/room_tree_api.rs` exposes root, children, ancestors, and search endpoints.
- The backend hierarchy is `room-root` -> `room-group:{group}` -> room refno -> `comp-group:{room_refno}:{group}` -> component refno.
- `../plant3d-web/src/api/genModelRoomTreeApi.ts` calls `/api/room-tree/*` and normalizes enum/string/refno payload shapes.
- `../plant3d-web/src/composables/useRoomTree.ts` initializes only when enabled, lazy-loads children, supports search, ancestor focus, visibility, selection, isolate, and fly-to.
- `../plant3d-web/src/components/model-tree/ModelTreePanel.vue` has `activeTree: 'pdms' | 'room'` and routes tree operations to `useRoomTree` while the room tab is active.

## Project Structure

### Documentation (this feature)

```text
specs/020-room-tree-compute-display/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── room-tree-api-contract.md
└── checklists/
    └── requirements.md
```

### Source Code (repository root and sibling frontend)

```text
plant-model-gen-cata-closure/
└── src/
    ├── fast_model/
    │   └── room_model.rs              # room compute and room relation generation
    ├── web_api/
    │   ├── room_tree_api.rs           # room tree HTTP contract
    │   └── mod.rs                     # stateless route registration
    └── web_server/
        └── mod.rs                     # web server assembly

../plant3d-web/
└── src/
    ├── api/
    │   └── genModelRoomTreeApi.ts     # client contract and ID normalization
    ├── composables/
    │   └── useRoomTree.ts             # room tree view model and scene operations
    ├── components/model-tree/
    │   └── ModelTreePanel.vue         # PDMS/room tab switching
    └── utils/
        └── apiBase.ts                 # backend API base resolution
```

**Structure Decision**: Keep compute and API in `plant-model-gen-cata-closure`; keep display and viewer interactions in `../plant3d-web`; document their cross-repo contract in this spec kit.

## Technical Approach

### 1. Room compute source contract

- Treat explicit room compute as the source of `room_relate`.
- Keep automatic room compute disabled by default unless configured by `AIOS_AUTO_ROOM_COMPUTE`.
- Document that room tree display is a read-only consumer of computed room data.

### 2. Backend room tree API

- Preserve the existing route set:
  - `GET /api/room-tree/root`
  - `GET /api/room-tree/children/{id}?limit=...`
  - `GET /api/room-tree/ancestors/{id}`
  - `POST /api/room-tree/search`
- Keep virtual node IDs stable for frontend expansion and ancestor path reconstruction.
- Return `success=false` with `error_message` for query failures instead of HTTP 500 when possible.

### 3. Frontend display

- Keep room tree activation scoped to `activeTree === 'room'`.
- Keep PDMS tree and room tree state separate.
- Use lazy loading for room tree children.
- Use loaded subtree IDs for viewer selection/visibility/isolation and avoid forcing model generation from room tree display.

### 4. Runtime validation

- Start backend against a site with known room compute output.
- Verify the four API endpoints with HTTP.
- Open `plant3d-web`, switch to room tab, expand nodes, search room, focus containing room, and use isolate/fly-to.
- Repeat with a site without room compute to verify empty/error state and PDMS tree isolation.

## Validation Plan

- Inspect route registration with `AIOS_PRINT_ROUTES=1` or startup logs and confirm `/api/room-tree/*` appears.
- `GET /api/room-tree/root` returns `success=true` and `node.id=room-root`.
- `GET /api/room-tree/children/room-root?limit=2000` returns room groups or an empty/clear failure state when room data is missing.
- Expand one group, one room, and one component group by HTTP and verify expected IDs and counts.
- `POST /api/room-tree/search` with a known room keyword returns room nodes.
- `GET /api/room-tree/ancestors/{component_refno}` returns component/room/group/root path for a known room-contained component.
- Frontend smoke: switch to room tab, expand tree, search, select, isolate, fly-to, and switch back to PDMS tree without state corruption.

## Post-Design Constitution Check

- **No cargo tests for web_server**: PASS. Validation uses running-service HTTP and frontend smoke.
- **aios-database validation via CLI/json**: PASS if compute fixtures need CLI setup.
- **No unexpected source churn**: PASS. Current plan is documentation and validation first.
- **Explicit room compute boundary**: PASS. Display does not trigger compute.

## Complexity Tracking

No constitution violations or complexity exceptions.
