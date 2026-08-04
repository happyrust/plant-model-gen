# Orchestrator Integration

## Integration points

The generation orchestrator should continue to depend on `ModelWriterBackend`, not individual storage engines.

Required integration points:

- writer construction from options
- `ModelWriterContext` propagation
- base instance batch writes
- mesh result persistence
- `inst_relate_aabb` writes
- negative relation reconciliation
- finalization summary

## Option model

Backend selection should remain explicit and conservative:

- `surreal`: current default
- `ducklake`: future canonical DuckLake writer
- `parquet`: future canonical Parquet writer
- `compare`: optional dual-write validation mode

The SurrealDB dependency source in Cargo files must remain `github.com/happyrust/surrealdb`.

## Canonical adapter placement

The canonical adapter should live below orchestration and above concrete sinks. This lets `SurrealModelWriterBackend` remain the compatibility implementation while new writers consume the same canonical records.

## Finalization

`finalize` should report:

- backend name
- batch counts
- raw rows written by table
- projection rows refreshed by table
- validation hint paths or SQL files when applicable

## Compatibility

Existing SurrealDB behavior is the compatibility baseline. Any new backend must prove parity through CLI + SQL before it can be used as a primary writer.
