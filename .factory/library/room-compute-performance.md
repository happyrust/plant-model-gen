# Room Compute Performance Notes

## Mission Goal

- full `room compute` on `dbnum=7997` reaches <= one-third of baseline runtime
- `room compute-panel` also shows clear improvement
- zero regression remains mandatory

## Current Hotspots

1. Spatial index refresh coupling
2. Geometry detail cache behaves like read-and-evict
3. Panel preparation work is repeated in the hot path
4. Concurrency is mostly room-level while panel/candidate work remains underused
5. Dynamic detail query cost remains in the path until `inst_mesh_meta` is integrated

## Milestone Focus

### Milestone 1

- observability and JSON reporting
- compare / verify CLI
- spatial index reuse
- true reusable caching
- prepared panel cache
- panel-aware concurrency

### Milestone 2

- `inst_mesh_meta` schema
- materialization write path
- read path with safe fallback
- parity proof for normal / bool / TUBI

### Milestone 3

- prepared candidate evaluator
- batched candidate execution
- final <= one-third performance gate

## Evidence Expectations

- always keep before/after timing or report evidence
- performance claims without compare evidence are insufficient
- log whether a run reused or rebuilt the spatial index
- record fallback behavior explicitly when the materialized path degrades
