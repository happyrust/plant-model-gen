# Data Model: Room Tree Compute And Display

## Room Relation

Computed room containment relation consumed by the room tree API.

**Source**: room compute pipeline in `src/fast_model/room_model.rs`.

**Storage**: Model database relation table, especially `room_relate`.

**Important Fields**:

- `room_num`: Room display code used to group component relations under a room.
- `out`: Related model object refno.
- `out.noun`: Leaf object noun.
- `out.owner...noun`: Owner chain used for component group classification.

**Notes**:

- This spec treats room relation data as precomputed input.
- Missing room relation rows are a valid state and should not imply frontend failure outside the room tree tab.

## RoomTreeNodeDto

Backend DTO returned to the frontend.

**Fields**:

- `id`: `RoomTreeNodeId`; either a model refno or a virtual node string.
- `name`: Display label.
- `noun`: Type label such as `ROOM_ROOT`, `ROOM_GROUP`, `ROOM`, `COMP_GROUP`, or a component noun.
- `owner`: Parent node ID when applicable.
- `children_count`: Number of children when known.

**ID Forms**:

- Root: `room-root`
- Room group: `room-group:{group}`
- Room: physical room `RefnoEnum`
- Component group: `comp-group:{room_refno}:{group_key}`
- Component: physical component `RefnoEnum`

## RoomTreeNodeId

Serializable backend ID type.

**Variants**:

- `Refno(RefnoEnum)`: Physical model object.
- `Str(String)`: Virtual grouping object.

**Frontend Normalization**:

`../plant3d-web/src/api/genModelRoomTreeApi.ts` normalizes IDs to strings with `normalizeRoomTreeId()` before inserting nodes into the local tree.

## ChildrenResponse

Response for `GET /api/room-tree/children/{id}`.

**Fields**:

- `success`: Whether the query succeeded.
- `parent_id`: ID of expanded node.
- `children`: Child `RoomTreeNodeDto` list.
- `truncated`: Whether `limit` cut the response.
- `error_message`: Human-readable failure message.

## AncestorsResponse

Response for `GET /api/room-tree/ancestors/{id}`.

**Fields**:

- `success`: Whether a path was found.
- `ids`: Path from target node up toward root. Frontend reverses this path during focus.
- `error_message`: Human-readable failure message.

**Expected Paths**:

- Room: `[room_refno, room-group:{group}, room-root]`
- Direct component: `[component_refno, comp-group:{room_refno}:{group}, room_refno, room-group:{group}, room-root]`
- Owner-chain component: `[comp-group:{room_refno}:{group}, room_refno, room-group:{group}, room-root]`

## SearchResponse

Response for `POST /api/room-tree/search`.

**Fields**:

- `success`: Whether search succeeded.
- `items`: Matching room nodes.
- `error_message`: Human-readable failure message.

**Current Scope**:

Search returns room nodes, not every component leaf.

## Frontend TreeNode

Local view model in `../plant3d-web/src/composables/useRoomTree.ts`.

**Fields**:

- `id`: Normalized string ID.
- `name`: Display label.
- `type`: Node noun/type.
- `parentId`: Parent ID or null.
- `childrenIds`: Loaded child IDs.

**State Maps**:

- `nodesById`: Loaded nodes.
- `rootIds`: Root IDs, normally `[room-root]`.
- `expandedIds`: Expanded nodes.
- `selectedIds`: Selected nodes.
- `checkStateById`: Visibility/check state.
- `childrenCountById`: Known child counts.
- `childrenLoadedById`: Loaded parent set.

## Component Group Classification

Component group key is derived from the owner noun chain.

**Priority**:

1. `BRAN`
2. `HANG`
3. `EQUI`
4. `OTHER`

The nearest owner noun in this list wins.
