# Environment

Environment notes for the room-compute 3x mission.

**What belongs here:** platform constraints, validation readiness, DB/data assumptions, and machine-specific quirks.
**What does NOT belong here:** service ports/commands (use `.factory/services.yaml`).

---

## Machine Snapshot

- OS: Windows 10 (`win32 10.0.26200`)
- CPU: AMD Ryzen 9 7950X, 16 cores / 32 logical processors
- Memory observed during planning:
  - total visible: ~63.1 GB
  - free physical: ~24.8 GB

## Database / Data Assumptions

- Existing local DB configuration is reused; do not start a second DB instance from mission automation.
- The room-compute validation target remains centered on `dbnum=7997`.
- The mission relies on the existing coarse-filter chain:
  - `spatial_index.sqlite`
  - `inst_relate_aabb`
  - `inst_relate_booled_aabb`

## Validation Readiness

- `cargo --version` succeeded during planning.
- `cargo run --bin aios-database -- room --help` succeeded during planning after a heavy cold build.
- Cold build cost is a known limitation; the initial dry run took about 7m46s.

## Mission-specific Constraints

- This is a CLI-only mission.
- Validator concurrency is capped at `1` because full and panel runs share persisted room relation state and SQLite/Surreal inputs.
- Avoid unnecessary full rebuilds during validation; reuse existing artifacts where safe and observable.

## Useful Notes

- Check mission logs/JSON reports rather than intuition when claiming performance gains.
- If `output/spatial_index.sqlite` is missing, validators may need an explicit rebuild step, but that should remain observable and intentional.
- Bool and TUBI semantics are part of correctness, not edge cases.
