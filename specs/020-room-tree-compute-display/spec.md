# Feature Specification: Room Tree Compute And Display

**Feature Branch**: `[020-room-tree-compute-display]`

**Created**: 2026-06-22

**Status**: Draft

**Input**: User description: "分析审核当前房间树的显示，在 ../plant3d-web 里是否能展示房间树了，现在房间树的计算和显示逻辑也整理成一个 spec kit"

## Purpose

Define the end-to-end room tree contract from room relation computation in `plant-model-gen-cata-closure` to display and interaction in sibling frontend `../plant3d-web`. The current codebase already contains a backend `/api/room-tree/*` surface and a frontend room-tree tab in `ModelTreePanel`; this spec captures the conditions under which the tree is actually visible, the API contract it depends on, and the validation work still required.

## Current Display Readiness

`../plant3d-web` can display the room tree when all of these conditions are true:

- The generated model web server exposes the stateless room tree routes from `src/web_api/room_tree_api.rs` through `assemble_stateless_web_api_routes()`.
- The active model database contains room relation data, especially `room_relate`, produced by explicit room computation.
- The frontend API base points at that running backend, either same-origin/proxy or `VITE_GEN_MODEL_API_BASE_URL` / query override.
- The model tree panel is opened and the user switches from the PDMS tree tab to the room tree tab.

The display path is not a room-compute trigger. It reads existing data and should fail gracefully when room data has not been computed.

## Analysis Decisions

| Decision Branch | Recommended Answer | Rationale |
|---|---|---|
| Can `plant3d-web` show the room tree now? | Yes, conditionally. The client and backend API are present, but runtime display depends on computed `room_relate` data and correct API base routing. | `src/components/model-tree/ModelTreePanel.vue` wires `activeTree='room'` to `useRoomTree`; `src/api/genModelRoomTreeApi.ts` calls `/api/room-tree/*`; backend routes are mounted in `src/web_api/mod.rs`. |
| Should opening the room tree compute rooms automatically? | No. Room compute remains an explicit backend operation; the room tree is a viewer over existing room relations. | Project memory says automatic post-generation room compute is disabled by default behind `AIOS_AUTO_ROOM_COMPUTE`. |
| What is the canonical backend hierarchy? | Root -> room group -> room -> component group -> component. | This matches `room_tree_children_core()` virtual nodes and `room_relate` lookups. |
| What should happen when room data is missing? | Return a successful empty tree level where possible or a clear `success=false` API error; frontend should show no rows/retry without corrupting PDMS tree state. | The frontend initializes only when the room tab is active and resets state on API failure. |
| Where should validation happen? | Run service + HTTP checks for backend, then frontend smoke through `plant3d-web` or browser automation. | Repository rules prohibit `cargo test` for `web_server`; web_server behavior must be validated through HTTP/POST/GET. |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Operator Opens Room Tree In The 3D Viewer (Priority: P1)

As a model viewer user, I need a room tree tab that lists rooms and their contained model objects, so I can browse the model by physical room instead of by PDMS owner hierarchy.

**Why this priority**: This is the main display workflow requested by the user.

**Independent Test**: Start a site whose room compute has produced `room_relate`, open `../plant3d-web`, switch the model tree to `room`, and verify room groups, rooms, component groups, and components appear.

**Acceptance Scenarios**:

1. **Given** a backend with room relations, **When** the frontend requests `/api/room-tree/root` and expands the root, **Then** room groups are displayed.
2. **Given** a room group is expanded, **When** the frontend requests children for `room-group:{group}`, **Then** room nodes are displayed with `children_count`.
3. **Given** a room is expanded, **When** the frontend requests children for the room refno, **Then** component group nodes such as `BRAN`, `HANG`, `EQUI`, or `OTHER` are displayed.
4. **Given** a component group is expanded, **When** children are requested, **Then** component refnos are displayed as leaf nodes.

---

### User Story 2 - User Locates Or Isolates Models By Room (Priority: P1)

As a model reviewer, I need room tree selection, show/hide, isolate, and fly-to actions to operate on loaded room-contained components, so I can inspect a room without manually selecting every object.

**Why this priority**: A visible tree is only useful if it can drive viewer operations safely.

**Independent Test**: Select a room or component group in the room tree and verify scene selection, isolation, visibility toggles, and fly-to operate on the loaded subtree only.

**Acceptance Scenarios**:

1. **Given** a room node with loaded descendants, **When** the user toggles the eye icon, **Then** loaded room-contained component refnos become visible or hidden.
2. **Given** a room node is selected, **When** isolate/xray is requested, **Then** the viewer isolates the room-contained loaded objects.
3. **Given** a room node has loaded component descendants, **When** fly-to is requested, **Then** the viewer camera flies to their combined AABB.
4. **Given** some room descendants have not been lazy-loaded, **When** visibility state changes, **Then** later-loaded descendants inherit the current state.

---

### User Story 3 - User Jumps From A Model Object To Its Room (Priority: P2)

As a reviewer looking at a model object, I need to find the containing room and open the room tree path, so room context is available from object-centric workflows.

**Why this priority**: This connects PDMS/object workflows to the room tree without requiring manual room search.

**Independent Test**: Trigger the existing "show containing room" flow from a selected model object and verify `roomTreeGetAncestors()` returns a path that expands and selects the room.

**Acceptance Scenarios**:

1. **Given** a selected component refno exists in `room_relate`, **When** the containing-room action runs, **Then** the room tree focuses the containing room.
2. **Given** the component belongs to an owner chain inside a room relation, **When** ancestors are queried, **Then** the API returns the nearest matching room path.
3. **Given** the component is not in any room relation, **When** the action runs, **Then** the UI reports that no containing room was found.

---

### User Story 4 - Missing Room Compute Is Clear And Non-Destructive (Priority: P2)

As an operator, I need the UI and API to make missing room computation obvious without breaking the normal PDMS tree, so I know when to run room compute explicitly.

**Why this priority**: Room compute is optional by default, so missing data is an expected state.

**Independent Test**: Open the room tree against a model database without `room_relate` rows and verify the PDMS tree continues working while the room tab shows empty/error state.

**Acceptance Scenarios**:

1. **Given** no `room_relate` data exists, **When** `/api/room-tree/children/room-root` is called, **Then** the response is empty or contains a clear error message.
2. **Given** room tree initialization fails, **When** the user switches back to PDMS tree, **Then** PDMS tree selection and visibility state are not corrupted.
3. **Given** room compute is later run successfully, **When** the room tree is reopened or retried, **Then** it can load room groups without restarting the frontend.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Backend MUST expose `/api/room-tree/root`, `/api/room-tree/children/{id}`, `/api/room-tree/ancestors/{id}`, and `/api/room-tree/search`.
- **FR-002**: Backend MUST build room groups from computed room relation data and only fall back to noun hierarchy discovery when relation data is empty.
- **FR-003**: Backend MUST use stable virtual node IDs for root, room groups, and component groups: `room-root`, `room-group:{group}`, and `comp-group:{room_refno}:{group_key}`.
- **FR-004**: Backend MUST return room and component IDs in a form that `normalizeRoomTreeId()` can normalize to frontend string IDs.
- **FR-005**: Backend MUST classify room-contained components by owner noun priority `BRAN`, `HANG`, `EQUI`, otherwise `OTHER`.
- **FR-006**: Backend MUST return `children_count` for expandable nodes when it can be computed cheaply enough for interactive use.
- **FR-007**: Frontend MUST initialize room tree state only when the room tab is active and MUST isolate room-tree state from PDMS tree state.
- **FR-008**: Frontend MUST call the room tree API through `getBackendApiBaseUrl()` so deployment-specific backend routing is respected.
- **FR-009**: Frontend MUST lazily load children and preserve check/visibility state for descendants loaded later.
- **FR-010**: Frontend MUST support search and ancestor focusing for room nodes and containing-room workflows.
- **FR-011**: Room tree display MUST NOT automatically run room compute or model generation.
- **FR-012**: Missing or failed room tree data MUST NOT break PDMS tree interactions.

### Key Entities

- **Room Relation**: Persisted relation rows such as `room_relate`, produced by the room compute pipeline and used as the source of room containment.
- **Room Tree Root**: Virtual root node `room-root`.
- **Room Group**: Virtual grouping node derived from room code parsing.
- **Room Node**: Physical room refno displayed under a room group.
- **Component Group**: Virtual grouping node under a room, classified by owner noun.
- **Room Component**: Leaf model object contained by a room relation.
- **Room Tree API Client**: `../plant3d-web/src/api/genModelRoomTreeApi.ts`.
- **Room Tree View Model**: `../plant3d-web/src/composables/useRoomTree.ts`.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Against a site with room relations, expanding root -> group -> room -> component group returns non-empty children at each level through HTTP.
- **SC-002**: In `plant3d-web`, switching to the room tree tab shows the same hierarchy returned by HTTP without affecting the PDMS tree tab.
- **SC-003**: Selecting a known component can focus its containing room through `/api/room-tree/ancestors/{id}`.
- **SC-004**: Against a site without room relations, the room tree fails empty/clear while the PDMS tree remains usable.
- **SC-005**: The documented contract can be verified without `cargo test`, using running-service HTTP and frontend smoke/browser validation.

## Assumptions

- Room compute is explicit unless `AIOS_AUTO_ROOM_COMPUTE` is intentionally enabled.
- `room_relate` is the authoritative display source for room containment after compute.
- `../plant3d-web` is the active frontend for this viewer workflow.
- The backend serving the model also serves the room tree API under the same API base used by `plant3d-web`.
- This spec documents and validates the current chain; it does not require new source changes unless validation reveals gaps.
