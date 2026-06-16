# ptset Measurement Display/Capture Repair Plan

Date: 2026-06-16
Scope: `../plant3d-web` measurement + current `plant-model-gen-cata-closure` model/parquet output
Planner: Plannotator-reviewed draft

## Goal

Make ptset display and measurement capture use the same current model snapshot, with clear failures when `ptsets.parquet` or required mapping fields are missing.

## Facts

- `plant3d-web` has already moved the main viewer ptset display path to `queryPtsetByRefnoFromParquet(dbno, refno)`.
- Measurement hover fetch also calls `queryPtsetByRefnoFromParquet()` and caches candidates in `usePtsetSnap`.
- BRAN child ptset summary now uses `instances.parquet.owner_refno_str` and `ptsets.parquet` instead of backend realtime children/batch ptset APIs.
- The backend still keeps `/api/pdms/ptset/{refno}` and `/api/pdms/ptset/batch-query` as realtime/debug compatibility paths.
- The current backend parquet export writes `instances.parquet`, `transforms.parquet`, `ptsets.parquet`, and `manifest_{dbnum}.json`.
- Project rule: do not create or run cargo tests. For `web_server`, verify through running service and HTTP/POST. For aios-database, verify through CLI + JSON.

## Suspected Failure Modes

1. `instances.parquet` row lacks `cata_hash`, so frontend cannot join to `ptsets.parquet`.
2. `ptsets.parquet` is present but has zero rows for a used `cata_hash`.
3. frontend display and snap both transform points, but one path applies `globalModelMatrix` differently.
4. formal ptset display accidentally regresses to `/api/pdms/ptset`, causing snapshot drift.
5. BRAN summary lists direct children correctly but batch draw fetches stale/uncached rows after manifest refresh.
6. user receives only a generic “no ptset” message, hiding whether the missing piece is instance row, cata hash, ptset rows, transform row, or manifest table.

## Repair Strategy

### Phase 1: Lock the Contract

- Document the invariant in `plant3d-web` near the parquet loader: viewer display, BRAN ptset panel, and measurement hover capture must use the loaded model package as source of truth.
- Treat `/api/pdms/ptset` as compatibility/debug only for the viewer path.
- Add an explicit data contract table for:
  - `instances.refno_str`
  - `instances.owner_refno_str`
  - `instances.cata_hash`
  - `instances.trans_hash`
  - `transforms.*`
  - `ptsets.cata_hash`
  - `ptsets.point_number`
  - `manifest.ptset_unit`

### Phase 2: Backend Export Hardening

- In `export_dbnum_instances_parquet.rs`, audit where `ptset_export_data.refno_cata_hash` is attached to `InstanceRow`.
- Add verbose export counters already available in the manifest to distinguish:
  - total exported instances
  - instances with missing cata hash
  - used cata hashes
  - cata hashes with empty ptset
  - written ptset points
- Confirm `manifest_{dbnum}.json` always includes `tables.ptsets` even when row count is 0, so the frontend can report “no rows” rather than “missing table”.
- If real data shows `cata_hash` is missing for valid geometric instances, trace it back to `inst_relate -> inst_info.cata_hash` generation rather than patching the frontend around it.

### Phase 3: Frontend Loader Diagnostics

- In `queryPtsetByRefnoFromParquet`, split current failure messages into precise cases:
  - no `instances.parquet` row for refno
  - missing `ptsets.parquet` table/file
  - missing `cata_hash`
  - missing transform row for `trans_hash`
  - no point rows for `cata_hash`
- Keep no-fallback behavior: do not call `/api/pdms/ptset` from viewer display or measurement capture.
- Add a small debug return field only if existing `PtsetResponse` consumers can tolerate it; otherwise keep diagnostics in `error_message`.

### Phase 4: Transform Consistency

- Confirm `usePtsetVisualizationThree` and `usePtsetSnap` both route through `ptsetTransform.ts`.
- Remove or reduce duplicated transform math if any remains outside `ptsetTransform.ts`.
- Verify direction vectors do not apply translation.
- Verify label/fly-to coordinates use scene coordinates after `globalModelMatrix`, while coordinate text remains intentionally based on world/display unit policy.

### Phase 5: Measurement UX

- Make measurement miss reasons surface the exact parquet loader error for the hovered refno.
- Keep the ptset snap threshold stable at screen pixels, not world units.
- Ensure hover visualization clears when moving away or source toggle disables ptset.

### Phase 6: Real Validation

Backend validation:

- Generate/export a known dbno/refno package through the existing CLI path.
- Inspect `manifest_{dbnum}.json` for `tables.ptsets`, `ptset_unit`, and `ptset_export`.
- Query parquet with DuckDB CLI or a small local script to prove:
  - target refno exists in `instances.parquet`
  - it has a non-empty `cata_hash`
  - that `cata_hash` has rows in `ptsets.parquet`
  - its `trans_hash` exists in `transforms.parquet`

Web/server validation:

- Run `web_server` and use HTTP, not Rust tests.
- For compatibility API only, request `/api/pdms/ptset/{refno}` and confirm it remains useful for diagnostics.

Frontend validation:

- Run `plant3d-web`.
- Open the model package for the target dbno/refno.
- Right-click/show ptset and confirm no network call to `/api/pdms/ptset/{refno}`.
- Hover measurement near the same refno and confirm the green hover crosses align with formal ptset display.
- Click near a ptset point and confirm the recorded measurement source is `ptset:<refno>#<number>`.
- For BRAN, open ptset panel, confirm direct children come from snapshot, and batch draw only draws successful rows.

## Files To Inspect/Modify

Current repo:

- `src/fast_model/export_model/export_dbnum_instances_parquet.rs`
- `src/web_api/ptset_api.rs`
- `src/fast_model/gen_model/pdms_inst.rs`
- `src/fast_model/gen_model/index_tree_mode.rs`

Sibling frontend:

- `../plant3d-web/src/composables/useDbnoInstancesParquetLoader.ts`
- `../plant3d-web/src/utils/three/ptsetTransform.ts`
- `../plant3d-web/src/composables/usePtsetSnap.ts`
- `../plant3d-web/src/composables/useXeokitMeasurementTools.ts`
- `../plant3d-web/src/composables/usePtsetVisualizationThree.ts`
- `../plant3d-web/src/components/dock_panels/ViewerPanel.vue`
- `../plant3d-web/src/components/dock_panels/PtsetPanelDock.vue`

## Acceptance Criteria

- Formal ptset display and measurement hover capture use only the parquet snapshot path in `plant3d-web`.
- Displayed green crosses and measurement snap targets align for the same refno and point number.
- Missing ptset data reports the exact failing link in the parquet chain.
- BRAN child list and batch draw are derived from `instances.parquet`.
- Backend manifest exposes enough counters to diagnose missing cata hash vs empty ptset.
- Verification is recorded with concrete commands and real HTTP/CLI outputs, without cargo tests.
