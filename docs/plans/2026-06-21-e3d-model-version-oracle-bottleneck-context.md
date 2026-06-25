# E3D Model Version Oracle Bottleneck Context

Date: 2026-06-21

Purpose: focused context for a second-opinion Oracle architecture review. The
full source package is too large; this file records the current decision,
verified DB1112 evidence, and the exact code bottleneck that needs review.

## Current Architecture Decision

The preferred production path is:

```text
SourceObservation
  -> BaselineState
  -> IncrementEvidence
  -> GenerationJob
  -> SurrealDB generation workspace
  -> immutable ReleasePackage
  -> DuckLake catalog/index/diff/audit/read-model
  -> read-only API
  -> two-pane 3D compare
```

User-facing model versions are `release_id + package_hash`. `sesno` is a source
history anchor only. DuckLake snapshot ids are storage audit ids only.

DuckLake is allowed for rebuildable release catalog, read model, component/unit
index, diff, impact, status events, audit, and asset lineage. It is not allowed
as the generation writer, GLB/Parquet payload store, baseline restore source,
job-state truth, or user-visible version identity.

## Implemented Backend Pieces

- `POST /api/model-version/runs/prepare-history-replay`
- `POST /api/model-version/runs/execute-history-replay-plan`
- `POST /api/model-version/releases/register`
- `POST /api/model-version/releases/publish-history`
- `POST /api/model-version/incremental/handoff`
- `POST /api/model-version/releases/{release_id}/state-machine`
- Source observation manifests with before/after hash checks.
- Physical baseline snapshot evidence for DB1112 `from_sesno=791`.
- History replay prepare now separates:
  - baseline proof source: physical snapshot/replacement DB at `sesno=791`;
  - history read source: current/history DB that can read through `to_sesno=897`.

## DB1112 Current Evidence

Target site:

```text
D:\AVEVA\Projects\E3D2.1\AvevaMarineSample
```

Target file:

```text
D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
```

Current/history source SHA-256:

```text
70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
```

Physical baseline snapshot SHA-256 at `from_sesno=791`:

```text
5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385
```

Successful prepare run:

```text
prepare_run_id = codex-http-history-targetsrc-20260621062500
source_mode    = physical_snapshot_with_history_source
from_sesno     = 791
to_sesno       = 897
```

Observed generate run:

```text
run_id          = codex-http-exec-generate-observed-20260621071222
phase           = generate
argv            = incremental-sesno --file <current-history-db> --from-sesno 791 --to-sesno 897 --generate-model --json
status          = cancelled for analysis
observed stage  = incremental_sesno_collecting_file
elapsed         = about 6.4 minutes before cancellation
CPU             = increasing continuously
metrics         = task-metrics.json heartbeat refreshed every 15s
source hash     = unchanged before/after
```

This proves the child process is not silent anymore, but it spends a long time
in the first collection stage before any SurrealDB persist, model generation, or
Parquet export starts.

## Code Bottleneck

The `incremental-sesno --generate-model` path in `src/main.rs` calls
`collect_pdms_increment_for_file` first:

```rust
let file_outcome =
    aios_database::data_interface::sesno_increment::collect_pdms_increment_for_file(
        &db_option_ext.inner.project_name,
        file.clone(),
        options.from_sesno,
        options.to_sesno,
        options.verbose,
    )?;
outcome.merge(file_outcome);
```

Later, the same run persists increment rows:

```rust
let persist_stats = {
    let _heartbeat = aios_database::perf_metrics::start_generate_heartbeat(
        "incremental_sesno_persisting",
        Some(format!("files={}", outcome.files.len())),
        std::time::Duration::from_secs(15),
    );
    aios_database::data_interface::sesno_increment::persist_pdms_increment_files(
        &outcome.files,
        options.verbose,
    )
    .await?
};
```

`collect_pdms_increment_for_file` in `src/data_interface/sesno_increment.rs`
opens pdms-io, resolves actual sesno range, and calls:

```rust
let grouped = io
    .collect_increment_eles(Some(actual_start..=actual_end))
    .with_context(|| {
        format!(
            "收集 PDMS 增量失败: dbnum={} sesno={}..={} file={}",
            dbnum,
            actual_start,
            actual_end,
            file_path.display()
        )
    })?;
```

It uses that `grouped` data to build:

- `IncrGeoUpdateLog`
- `PdmsSesnoElementChange`
- per-file counts and actual sesno range

`persist_pdms_increment_file` then opens pdms-io again and repeats the same
collection:

```rust
let grouped = io
    .collect_increment_eles(Some(
        report.actual_start_sesno as i32..=report.actual_end_sesno as i32,
    ))
    .with_context(|| {
        format!(
            "收集 PDMS 增量落库数据失败: dbnum={} sesno={}..={} file={}",
            report.dbnum,
            report.actual_start_sesno,
            report.actual_end_sesno,
            report.file_path.display()
        )
    })?;
```

This means DB1112 `791 -> 897` may scan/resolve operations twice before model
generation starts.

## pdms-io Relevant Shape

In `D:\work\plant-code\pdms-io-fork\src\io.rs`, operation data is cloneable:

```rust
#[derive(Debug, Clone)]
pub enum EleOperationDetail {
    Add(EleData),
    Modified(ModifiedElement),
    Deleted,
    None,
}

#[derive(Debug, Clone)]
pub struct EleOperationData {
    pub refno: RefU64,
    pub sesno: u32,
    pub detail: EleOperationDetail,
}
```

`collect_increment_eles` returns:

```rust
pub fn collect_increment_eles(
    &mut self,
    sesno_range: Option<RangeInclusive<i32>>,
) -> anyhow::Result<BTreeMap<u32, Vec<EleOperationData>>>
```

Its high-level loop:

```rust
let session_numbers = match &sesno_range {
    Some(range) => self
        .ses_range_map
        .keys()
        .filter(|&sesno| range.contains(sesno))
        .cloned()
        .collect::<Vec<i32>>(),
    None => {
        let latest_sesno = self.get_latest_sesno()? as i32;
        vec![latest_sesno]
    }
};

for &sesno in session_numbers.iter() {
    let final_locs = self.collect_refno_locs(sesno);
    let mut operation_details_for_sesno = HashMap::with_capacity(final_locs.len());
    let mut seen_refnos = HashSet::with_capacity(final_locs.len());

    for loc in final_locs {
        let refno = RefU64::from_two_nums(loc.refno_0, loc.refno_1);
        if !seen_refnos.insert(refno) {
            continue;
        }

        let operation_details =
            self.get_refno_operation_status(refno, Some(sesno as u32))?;
        operation_details_for_sesno.extend(operation_details);
    }

    let operation_data =
        convert_to_operation_data(operation_details_for_sesno, sesno as u32);
    if !operation_data.is_empty() {
        grouped_results.insert(sesno as u32, operation_data);
    }
}
```

## Review Questions

1. Is the current architecture still the correct production path?
2. Should DuckLake be used in this version, and exactly where should the hard
   boundary be?
3. What is the precise model-data version contract: entity IDs, immutable
   facts, derived/rebuildable indexes, and release identity?
4. Should the next DB1112 slice prioritize eliminating the double
   `collect_increment_eles` pass before trying to publish/index more releases?
5. What safe Rust implementation is preferred?
   - transient grouped operations in the outcome with `#[serde(skip)]`;
   - new collector API returning `(outcome, grouped_operations)`;
   - a per-run collection artifact;
   - pdms-io progress callbacks and cancellation;
   - or another approach?
6. Which edge cases must block production readiness even if two-pane visual
   comparison appears to work?
7. What evidence would falsify the current DuckLake-as-read-model decision?
