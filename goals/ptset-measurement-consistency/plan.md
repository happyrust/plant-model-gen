# ptset Measurement Consistency Plan

## Solution Approach

Keep `ptsets.parquet` as the single source of truth for plant3d-web ptset display and measurement capture, while hardening backend export diagnostics and frontend failure messages. The compatibility `/api/pdms/ptset` endpoints stay available for diagnostics, but the viewer and measurement paths should not depend on them.

## Ordered Steps

1. Confirm the current parquet contract.

   Files: `src/fast_model/export_model/export_dbnum_instances_parquet.rs`, `../plant3d-web/src/composables/useDbnoInstancesParquetLoader.ts`

   Check that `instances.parquet` carries `refno_str`, `owner_refno_str`, `cata_hash`, and `trans_hash`; `transforms.parquet` carries the matrix rows; `ptsets.parquet` is keyed by `cata_hash` and `point_number`; and `manifest_{dbnum}.json` exposes `ptset_unit` and `ptset_export`.

2. Harden backend export diagnostics.

   File: `src/fast_model/export_model/export_dbnum_instances_parquet.rs`

   Ensure manifest counters distinguish total exported instances, missing `cata_hash` refnos, used cata hashes, empty ptset hashes, and written ptset point count. If a real target refno has geometry but no `cata_hash`, trace the generation path back to `inst_relate -> inst_info.cata_hash` instead of adding frontend fallback.

3. Improve frontend parquet loader diagnostics.

   File: `../plant3d-web/src/composables/useDbnoInstancesParquetLoader.ts`

   Split `queryPtsetByRefnoFromParquet` failures into precise messages for missing instance row, missing `ptsets.parquet`, missing `cata_hash`, missing transform row, and no point rows for `cata_hash`. Keep the response compatible with existing `PtsetResponse` consumers.

4. Lock display and capture to the same transform path.

   Files: `../plant3d-web/src/utils/three/ptsetTransform.ts`, `../plant3d-web/src/composables/usePtsetVisualizationThree.ts`, `../plant3d-web/src/composables/usePtsetSnap.ts`

   Confirm both display and snap candidates apply `pt * unitFactor -> world/refno transform -> globalModelMatrix`. Direction vectors must ignore translation. Remove duplicated transform logic if it can drift.

5. Preserve no-fallback viewer behavior.

   Files: `../plant3d-web/src/components/dock_panels/ViewerPanel.vue`, `../plant3d-web/src/composables/useXeokitMeasurementTools.ts`, `../plant3d-web/src/components/dock_panels/PtsetPanelDock.vue`

   Verify formal display, measurement hover fetch, and BRAN child summaries do not call `/api/pdms/ptset`. Keep backend ptset API for manual diagnostics only.

6. Validate with real commands.

   Backend/parquet: run the project CLI export for a known dbno/refno, then inspect `manifest_{dbnum}.json` and query parquet rows for the target refno/cata hash/transform.

   Server: run `web_server` and check compatibility API with HTTP/POST only.

   Frontend: run plant3d-web, open the model package, show ptset, hover measurement, and confirm display crosses align with snap targets and recorded measurement source is `ptset:<refno>#<number>`.

## Verification

- No cargo tests.
- Use SigMap before file exploration.
- Use CodeGraph in the current Rust repo for symbol and impact checks where available.
- Use `rg`/direct reads in `../plant3d-web` because it is not CodeGraph-indexed.
- Record exact CLI, HTTP, and browser validation commands/results in the final implementation notes.

## Risks

- Some target data may genuinely lack `cata_hash`; that is a generation/export data issue, not a frontend fallback issue.
- Transform bugs can be visually subtle because labels, fly-to boxes, cross geometry, and snap candidates use related but not identical coordinate representations.
- `plannotator annotate --gate` is an interactive browser gate; if no review is submitted, the written plan remains the authoritative draft but is not human-approved by Plannotator.
