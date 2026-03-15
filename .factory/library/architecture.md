# Architecture

Architectural notes for the room-compute 3x performance mission.

## Validation Dry-Run Findings

- CLI validation surface is reachable through `cargo run --bin aios-database -- room --help`
- Cold build cost is high; planning dry run took about 7m46s before showing room subcommands
- Validation remains CLI-first; no web or extra service surface is part of this mission

## Current Room Compute Pipeline

Primary code path: `src/fast_model/room_model.rs`

```text
build_room_relations_with_overrides
  -> ensure_spatial_index_ready
  -> build_room_panels_relate_for_query
  -> query_room_panels_with_tree_index
  -> query_candidate_rooms
  -> query_insts_for_room_calc (panel prefetch)
  -> compute_room_relations_with_cancel
     -> process_panel_for_room
        -> cal_room_refnos_with_options
           -> query panel geom insts
           -> derive/merge panel AABB
           -> load transformed panel meshes
           -> SQLite RTree coarse filter
           -> batch candidate AABB lookup
           -> 27-point vote against panel meshes
  -> save room relations
```

## Mission Architectural Boundaries

- Keep the mission CLI-only.
- Reuse the existing SQLite coarse filter:
  - `spatial_index.sqlite`
  - `inst_relate_aabb`
  - `inst_relate_booled_aabb`
- Do not introduce a new service, queue, or daemon.
- Do not add a new SQLite idx table in Milestone 1 or 2.

## High-ROI Hotspots

### 1. Spatial Index Refresh Coupling

- `ensure_spatial_index_ready()` can dominate steady-state runs if it rebuilds too eagerly.
- This must be reused or scoped before deeper hot-path work can show its full value.

### 2. Geometry Detail Cache Semantics

- `GEOM_CACHE` currently behaves like a transport buffer rather than a reusable cache because reads remove entries.
- Repeated lookups need a true shared-cache behavior.

### 3. Panel Hot Path Rebuild Cost

- `cal_room_refnos_with_options()` rebuilds panel state that is often reusable:
  - merged AABB
  - transformed meshes
  - evaluator-ready inputs

### 4. Execution Model Bottleneck

- The current orchestration is mainly room-level concurrent while panel work inside a room remains sequential.
- `candidate_concurrency` exists in options but is not yet a meaningful execution-model lever.

### 5. Dynamic Detail Query Cost

- The current detail path still relies on dynamic query composition and batching.
- `inst_mesh_meta` is intended to replace this with a materialized, cache-friendly read path.

## Milestone Intent

### Milestone 1

- establish structured observability
- add compare/verify evidence
- decouple spatial index refresh
- fix cache semantics
- add prepared panel cache
- improve panel-aware concurrency

### Milestone 2

- define `inst_mesh_meta`
- materialize writes
- integrate batched reads with safe fallback
- prove normal / bool / TUBI parity

### Milestone 3

- extract prepared candidate evaluator
- batch candidate execution
- prove final <= one-third runtime with zero regression

## Correctness Rules

- Bool and TUBI semantics are first-class correctness constraints, not optional edge cases.
- Every fast path must either preserve semantics exactly or fall back explicitly.
- Performance success is not enough without compare evidence.
