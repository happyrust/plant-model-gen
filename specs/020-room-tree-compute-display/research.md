# Research: Room Tree Compute And Display

## Question: Is the room tree API mounted by the backend?

**Decision**: Yes. The stateless web API assembly includes `create_room_tree_routes()`.

**Evidence**:

- `src/web_api/mod.rs` declares and exports `room_tree_api`.
- `assemble_stateless_web_api_routes()` merges `create_room_tree_routes()`.
- `stateless_web_api_route_paths()` lists all four `/api/room-tree/*` routes.

**Risk**: A packaged or older runtime binary may not include the current route assembly. Runtime route logs or HTTP checks are still required.

## Question: Can `../plant3d-web` display a room tree?

**Decision**: Yes, conditionally. The frontend display chain is present.

**Evidence**:

- `src/components/model-tree/ModelTreePanel.vue` has `activeTree: 'pdms' | 'room'`.
- The room tab uses `useRoomTree(roomViewerRef, computed(() => activeTree.value === 'room'))`.
- `src/api/genModelRoomTreeApi.ts` calls `/api/room-tree/root`, `/children`, `/ancestors`, and `/search`.
- `src/composables/useRoomTree.ts` initializes the tree, lazy-loads children, and exposes selection, visibility, isolate, fly-to, search, and ancestor focus.

**Condition**: The backend must be reachable through `getBackendApiBaseUrl()`, and the model database must already contain room relation data.

## Question: What data does room tree display require?

**Decision**: `room_relate` is the primary display source.

**Evidence**:

- `query_arch_room_groups()` starts from `aios_core::room::algorithm::query_rooms_from_room_relate()`.
- Component queries read `FROM room_relate WHERE room_num = ...`.
- Ancestor resolution for components queries `room_relate` by `out` and owner chain.

**Risk**: If room compute has not run, the tree may be empty or rely on fallback room discovery that cannot populate component containment.

## Question: Does room tree display trigger room compute?

**Decision**: No. It is read-only over existing data.

**Evidence**:

- Frontend room tree code only calls `/api/room-tree/*`.
- Backend `room_tree_api.rs` only performs queries and DTO assembly.
- Project memory records that automatic room compute is disabled by default unless `AIOS_AUTO_ROOM_COMPUTE` is enabled.

## Question: Are backend IDs compatible with frontend normalization?

**Decision**: Likely yes, but runtime JSON should be sampled.

**Evidence**:

- Backend uses untagged `RoomTreeNodeId` with `RefnoEnum` or `String`.
- Frontend `normalizeRoomTreeId()` handles arrays, nested object forms, Surreal-style wrappers, comma/slash refnos, and plain strings.

**Risk**: The exact serialized JSON for `RefnoEnum` should be captured in quickstart validation to confirm no frontend-only edge case remains.

## Question: What is the main unverified gap?

**Decision**: Runtime validation, not static wiring.

**Rationale**: Static code shows the chain exists, but the user asked whether it can display "current" room tree. That requires a running backend with room compute data and a frontend pointed at the same backend. This spec marks HTTP/browser validation as pending.
