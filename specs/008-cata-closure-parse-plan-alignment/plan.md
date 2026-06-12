# Implementation Plan: CATA closure 与解析计划口径对齐

## Goal

让管理站点的解析目标列表与 CATA refno closure 结果对齐：

- 最终 parse config 不再把所有 CATA 库都标成待解析目标。
- 站点级 `db_index.sqlite` 能在单库目标场景记录真实依赖边。
- 运行时在 closure manifest 生成后，用 manifest 收窄最终 parse config。

## Current Flow

```text
admin site / quick deploy
  -> load_parse_plan_from_sidecar()
  -> write_site_files_with_parse_plan()
       DbOption-parse.toml: included_db_files = manual DESI + DICT + all CATA
  -> gen-cata-closure --rescan-index
       cata_closure.json: by_dbnum = precise CATA refno closure
  -> parse with AIOS_CATA_CLOSURE_MODE=manifest
       each CATA file not in by_dbnum => skipped
```

Observed mismatch:

```text
parse plan: aps250166_0001 source=auto_related
manifest: by_dbnum only contains 250193
parse metrics: 250166 mode=skipped total_in_file=3
```

## Target Flow

```text
admin site / quick deploy
  -> rebuild db_index
       records target DESI dependency edges even when manual_db_nums is non-empty
  -> preview parse plan
       may be conservative until precise db_index is available
  -> write initial parse config
  -> gen-cata-closure --rescan-index
       produces authoritative CATA by_dbnum
  -> rewrite parse config from manifest
       keep non-CATA files
       keep manual DESI
       keep only CATA dbnums present in manifest
  -> parse with AIOS_CATA_CLOSURE_MODE=manifest
       no manifest-outside CATA files are visited
```

## Design

### 1. Fix db-index rebuild for manual target sites

Current code in `src/parse_sidecar.rs::rebuild_db_index_request()` skips dependency collection whenever `manual_db_nums` is non-empty.

Replace it with targeted collection:

```rust
let all_outbound = collect_design_outbound(&roots).await;
let target_set = payload.manual_db_nums.iter().copied().collect::<HashSet<_>>();
let outbound = if target_set.is_empty() {
    all_outbound
} else {
    all_outbound
        .into_iter()
        .filter(|(src, _)| target_set.contains(src))
        .collect()
};
```

Then record dependencies as today. This keeps the work bounded to target DESI sources while still populating `db_dependency`.

### 2. Stop using "all CATA" as the final parse target

Current code in `src/parse_sidecar.rs::resolve_included_db_files()` adds every CATA file for `auto_parse_related_dbnums=true`.

Refactor target selection in two stages:

1. Runtime finalization must be manifest-driven:
   - `manual_db_nums` always add their own files.
   - mandatory preparse DB types stay unchanged.
   - `parse_db_types` explicit selections stay unchanged.
   - CATA files are rewritten from manifest coverage after `gen-cata-closure` succeeds.
2. Preview plan can remain conservative until it can read the same precise dependency source.

Because `/parse/preview-plan` currently has no `index_path`, avoid a half-fix that simply removes all CATA from preview. That would make it impossible for a pure "filter existing entries" helper to keep manifest-covered `250193`.

Recommended sequence:

1. Short term: keep preview conservative if needed, but make runtime manifest alignment authoritative before parse starts.
2. Medium term: extend preview request with an optional `db_index_path` and use `DbIndexStore::resolve_related_closure(manual_db_nums)` to display precise related files.

### 3. Runtime manifest alignment after closure

Add a helper in `src/web_server/managed_project_sites.rs`:

```rust
fn align_parse_plan_cata_with_manifest(
    site: &ManagedProjectSite,
    plan: &ManagedSiteParsePlan,
    manifest: &CataClosureManifest,
) -> ManagedSiteParsePlan
```

Behavior:

- Build `covered_dbnums = manifest.by_dbnum.keys()`.
- Keep every non-CATA entry.
- Keep CATA entries only if `entry.dbnum` is in `covered_dbnums`.
- Ensure each manifest-covered CATA dbnum is present in `included_db_files`; if it is missing from the original plan, resolve dbnum -> file name from the same project roots/db index source used by the sidecar, not by ad hoc web_server file scanning.
- Recompute `included_db_files` and `auto_related_db_files`.
- Preserve `warnings`; append a warning only when a CATA entry lacks dbnum/db_type metadata and therefore cannot be safely classified.

Call sequence in `spawn_parse_process()`:

1. Load and write initial parse plan.
2. If `cata_partial_enabled`, run closure job.
3. If closure job succeeded:
   - load `default_manifest_path(&site.project_name)`;
   - align CATA entries with manifest coverage;
   - call `write_site_files_with_parse_plan(..., Some(&aligned_plan))` again;
   - append one log line summarizing `before_cata_files -> after_cata_files`.
4. Start parse job.

Fail-open rule:

- If manifest missing/parse error, append a warning and do not realign.
- If closure job failed, keep existing failure behavior.

### 4. Logging policy

`src/data_interface/cata_closure.rs::apply_sync_filter()` currently logs manifest-outside CATA as `warn!` and also `println!`.

After runtime alignment, this branch becomes exceptional/noisy only when stale config or race exists. Change policy:

- normal skipped mode should be `log::info!`;
- avoid duplicate `println!`;
- metrics remains the structured source of truth.

### 5. Tests and validation

Unit-level checks:

- `align_parse_plan_cata_with_manifest` keeps non-CATA and manual DESI entries.
- It removes CATA entries absent from manifest.
- It keeps CATA entries present in manifest.
- It can add a manifest-covered CATA entry that was absent from the conservative preview, using a sidecar/db_index-sourced dbnum -> file mapping.
- Empty manifest removes all CATA only when closure succeeded and manifest loaded.

Integration/manual validation:

```powershell
cargo check -q --features web_server
```

Then run the packaged scenario or source dev equivalent:

```powershell
# Start admin site, trigger quick deploy / parse for manual_db_nums=[250160]
# Verify files under runtime/admin_sites/quicktest-250160-8080/
```

Expected artifact checks:

```text
output/AvevaPlantSample/scene_tree/cata_closure.json
  by_dbnum keys: [250193]

DbOption-parse.toml
  included_db_files contains aps250193_0001
  included_db_files does not contain aps250166_0001

metrics/parse-*.json
  parse.dbs has 250193 mode=partial elements=16
  parse.dbs has no 250166 skipped row
```

## Risks

### R1: Preview UI may temporarily over-report related CATA

Until preview reads the same precise db_index/manifest source, it may remain conservative and show more auto-related CATA files than the final parse config. This is acceptable if runtime parse config is corrected after closure and UI text explains that CATA is manifest-driven.

### R2: Manifest path must match parse config project name

Use `cata_closure::default_manifest_path(&site.project_name)` after `gen-cata-closure` because this is the same path `load_sync_filter()` reads. Do not derive from cwd manually.

### R3: Non-CATA dependencies must not be removed

DICT and system/reuse DB files are not governed by CATA manifest and must remain in `included_db_files`.

### R4: Missing manifest must not silently remove CATA

Only align CATA files after a successful closure job and successful manifest load. Otherwise preserve existing config and let current fallback semantics handle it.

## Rollout

1. Implement runtime manifest alignment and unit tests.
2. Fix `/db-index/rebuild` dependency collection for manual targets.
3. Adjust preview plan CATA inclusion behavior.
4. Downgrade normal skip logging.
5. Rebuild package and rerun `quicktest-250160-8080`.
