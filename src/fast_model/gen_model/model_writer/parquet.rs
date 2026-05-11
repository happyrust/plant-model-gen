//! Phase 1 canonical raw sink + Phase B `ModelWriterBackend` impl.
//!
//! 模块包含两层产物：
//!
//! 1. `CanonicalParquetWriter`：file-oriented canonical raw sink（JSONL fallback），
//!    与 `canonical_records.rs` 共同支撑 `docs/development/model-writer-storage/`
//!    的 canonical raw boundary。当前输出 JSON Lines 以保留 layout，typed Parquet
//!    物化推迟到 v4（mission 05-parquet-writer.md §Phase boundary）。
//! 2. `ParquetModelWriterBackend`：v3 Phase B 新增的 `ModelWriterBackend` 实现，
//!    通过 `CanonicalRawPlanner` 把生成端 batch 转成 canonical raw records 落盘。
//!    boolean bridge 跳过（Phase 2 范围），mesh_results 在 `file_mesh_state=true`
//!    时跳过、其余情况下不写入 canonical 表（canonical raw boundary 当前仅覆盖
//!    Phase 1 13 张 raw 表，mesh 持久化是另一条工作流）。

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use async_trait::async_trait;
use serde::Serialize;

use super::{
    BaseInstanceBatch, BooleanBridgeReport, BooleanBridgeRequest, CanonicalRawBatch,
    CleanupRequest, FinalizeRequest, FinalizeSummary, InstRelateAabbBatch, MeshResultBatch,
    ModelWriterBackend, ModelWriterContext, ReconcileRequest, WriteBaseReport,
};
use crate::fast_model::gen_model::canonical_records::{CanonicalRawPlanner, CanonicalRawTable};

/// File-oriented canonical writer scaffold for the future Parquet backend.
///
/// This phase keeps the boundary isolated from production `ModelWriterBackend`
/// selection and emits JSON Lines files with the same table layout the Parquet
/// writer will own. Typed Parquet materialization remains gated to the next
/// phase so the current Surreal/default path is not affected by heavy optional
/// Parquet/Polars dependencies.
#[derive(Debug, Clone)]
pub struct CanonicalParquetWriter {
    config: CanonicalParquetWriterConfig,
}

#[derive(Debug, Clone)]
pub struct CanonicalParquetWriterConfig {
    pub output_dir: PathBuf,
    pub project_name: String,
    pub dbnum: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CanonicalParquetBatchSummary {
    pub backend: &'static str,
    pub format: &'static str,
    pub limitation: &'static str,
    pub summary_path: String,
    pub project_name: String,
    pub dbnum: u32,
    pub batch_id: u64,
    pub raw_root: String,
    pub total_rows: usize,
    pub tables: Vec<CanonicalParquetTableSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CanonicalParquetTableSummary {
    pub table: &'static str,
    pub rows: usize,
    pub path: String,
    pub limitation: Option<&'static str>,
}

impl CanonicalParquetWriter {
    pub fn new(config: CanonicalParquetWriterConfig) -> Self {
        Self { config }
    }

    pub fn write_raw_batch(
        &self,
        batch_id: u64,
        batch: &CanonicalRawBatch,
    ) -> anyhow::Result<CanonicalParquetBatchSummary> {
        let raw_root = self
            .config
            .output_dir
            .join("model_writer_storage")
            .join("raw");
        fs::create_dir_all(&raw_root).with_context(|| {
            format!(
                "failed to create canonical parquet raw root {}",
                raw_root.display()
            )
        })?;

        let mut tables = Vec::new();
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawInstInfo,
            batch_id,
            &batch.inst_info,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawInstRelate,
            batch_id,
            &batch.inst_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawInstGeo,
            batch_id,
            &batch.inst_geo,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawGeoRelate,
            batch_id,
            &batch.geo_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawTubiInfo,
            batch_id,
            &batch.tubi_info,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawTubiRelate,
            batch_id,
            &batch.tubi_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawNegRelate,
            batch_id,
            &batch.neg_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawNgmrRelate,
            batch_id,
            &batch.ngmr_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawAabb,
            batch_id,
            &batch.aabb,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawTrans,
            batch_id,
            &batch.trans,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawVec3,
            batch_id,
            &batch.vec3,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawInstRelateAabb,
            batch_id,
            &batch.inst_relate_aabb,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawRefnoAssocIndex,
            batch_id,
            &batch.refno_assoc_index,
        )?);

        let summary_path = self.summary_file_path(batch_id);
        let total_rows = tables.iter().map(|table| table.rows).sum();
        let summary = CanonicalParquetBatchSummary {
            backend: "parquet-canonical",
            format: "jsonl-fallback",
            limitation: "typed parquet materialization is intentionally deferred; this scaffold preserves canonical table boundaries and row counts",
            summary_path: stable_path_string(&summary_path),
            project_name: self.config.project_name.clone(),
            dbnum: self.config.dbnum,
            batch_id,
            raw_root: stable_path_string(&raw_root),
            total_rows,
            tables,
        };
        self.write_summary(&summary_path, &summary)?;
        Ok(summary)
    }

    fn write_table<T: Serialize>(
        &self,
        raw_root: &Path,
        table: CanonicalRawTable,
        batch_id: u64,
        rows: &[T],
    ) -> anyhow::Result<CanonicalParquetTableSummary> {
        let path = self.table_file_path(raw_root, table, batch_id);
        write_json_lines(&path, rows)?;
        Ok(CanonicalParquetTableSummary {
            table: table.as_str(),
            rows: rows.len(),
            path: stable_path_string(&path),
            limitation: table.phase1_limitation(),
        })
    }

    fn table_file_path(&self, raw_root: &Path, table: CanonicalRawTable, batch_id: u64) -> PathBuf {
        raw_root
            .join(table.as_str())
            .join(format!("project_name={}", self.config.project_name))
            .join(format!("dbnum={}", self.config.dbnum))
            .join(format!("batch_{batch_id}.jsonl"))
    }

    pub fn summary_file_path(&self, batch_id: u64) -> PathBuf {
        self.config
            .output_dir
            .join("model_writer_storage")
            .join("summary")
            .join(format!("project_name={}", self.config.project_name))
            .join(format!("dbnum={}", self.config.dbnum))
            .join(format!("batch_{batch_id}.json"))
    }

    fn write_summary(
        &self,
        path: &Path,
        summary: &CanonicalParquetBatchSummary,
    ) -> anyhow::Result<()> {
        let summary_dir = path.parent().with_context(|| {
            format!(
                "failed to determine canonical parquet summary dir for {}",
                path.display()
            )
        })?;
        fs::create_dir_all(summary_dir).with_context(|| {
            format!(
                "failed to create canonical parquet summary dir {}",
                summary_dir.display()
            )
        })?;
        let file = File::create(path)
            .with_context(|| format!("failed to create summary file {}", path.display()))?;
        serde_json::to_writer_pretty(BufWriter::new(file), summary)
            .with_context(|| format!("failed to write summary file {}", path.display()))
    }
}

fn stable_path_string(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn write_json_lines<T: Serialize>(path: &Path, rows: &[T]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create table dir {}", parent.display()))?;
    }
    let file = File::create(path)
        .with_context(|| format!("failed to create table file {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    for row in rows {
        serde_json::to_writer(&mut writer, row)
            .with_context(|| format!("failed to serialize row for {}", path.display()))?;
        writer
            .write_all(b"\n")
            .with_context(|| format!("failed to write row for {}", path.display()))?;
    }
    writer
        .flush()
        .with_context(|| format!("failed to flush table file {}", path.display()))
}

// =====================================================================
//   ParquetModelWriterBackend — v3 Phase B ModelWriterBackend impl
// =====================================================================

/// File-oriented `ModelWriterBackend` writing canonical raw records to JSONL
/// (Phase 1 fallback) under the mission 05 directory layout.
///
/// Architectural notes:
///
/// - Boolean bridge (`run_boolean_bridge`) is intentionally skipped: boolean
///   tables are Phase 2 (`docs/development/model-writer-storage/00`).
/// - `reconcile_missing_neg` returns 0 with an `approximate=true` log: the file
///   sink has no cross-batch view to perform Surreal-style reconciliation.
/// - `persist_mesh_results` returns Ok without writing canonical rows: Phase 1
///   canonical scope does not include mesh persistence (mission 05 §Layout).
/// - `write_inst_relate_aabb` is a NoOp because `write_base_batch` already
///   produced the `raw_inst_relate_aabb` rows via the canonical planner.
pub struct ParquetModelWriterBackend {
    output_root: PathBuf,
    dbnum: u32,
    context: OnceLock<ModelWriterContext>,
    writer: OnceLock<CanonicalParquetWriter>,
    batches_completed: AtomicU64,
    total_rows_written: AtomicU64,
}

impl std::fmt::Debug for ParquetModelWriterBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParquetModelWriterBackend")
            .field("output_root", &self.output_root)
            .field("dbnum", &self.dbnum)
            .field("initialized", &self.writer.get().is_some())
            .finish()
    }
}

impl ParquetModelWriterBackend {
    /// 默认 dbnum 占位，等 v4 把 dbnum 拉进 `ModelWriterContext` 时替换。
    pub const DEFAULT_DBNUM: u32 = 0;

    pub fn new(output_root: impl Into<PathBuf>) -> Self {
        Self::with_dbnum(output_root, Self::DEFAULT_DBNUM)
    }

    pub fn with_dbnum(output_root: impl Into<PathBuf>, dbnum: u32) -> Self {
        Self {
            output_root: output_root.into(),
            dbnum,
            context: OnceLock::new(),
            writer: OnceLock::new(),
            batches_completed: AtomicU64::new(0),
            total_rows_written: AtomicU64::new(0),
        }
    }

    fn writer(&self) -> anyhow::Result<&CanonicalParquetWriter> {
        self.writer
            .get()
            .context("ParquetModelWriterBackend used before init (lifecycle contract violated)")
    }

    fn context(&self) -> anyhow::Result<&ModelWriterContext> {
        self.context
            .get()
            .context("ParquetModelWriterBackend used before init (lifecycle contract violated)")
    }

    fn raw_root(&self) -> PathBuf {
        self.output_root.join("model_writer_storage").join("raw")
    }
}

#[async_trait]
impl ModelWriterBackend for ParquetModelWriterBackend {
    fn name(&self) -> &'static str {
        "parquet"
    }

    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()> {
        println!(
            "[model-writer:parquet] stage=init project={} output_root={} dbnum={} mode={}",
            context.project_name,
            self.output_root.display(),
            self.dbnum,
            context.mode.as_str()
        );
        let _ = self.context.set(context.clone());
        let writer = CanonicalParquetWriter::new(CanonicalParquetWriterConfig {
            output_dir: self.output_root.clone(),
            project_name: context.project_name.clone(),
            dbnum: self.dbnum,
        });
        let _ = self.writer.set(writer);
        let raw_root = self.raw_root();
        fs::create_dir_all(&raw_root).with_context(|| {
            format!(
                "failed to ensure parquet raw_root {} during init",
                raw_root.display()
            )
        })?;
        println!(
            "[model-writer:parquet] stage=init done raw_root={}",
            stable_path_string(&raw_root)
        );
        Ok(())
    }

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()> {
        let ctx = self.context()?;
        let raw_root = self.raw_root();
        println!(
            "[model-writer:parquet] stage=cleanup seed_refnos={} raw_root={} project={} dbnum={}",
            request.seed_refnos.len(),
            stable_path_string(&raw_root),
            ctx.project_name,
            self.dbnum
        );

        if !raw_root.exists() {
            println!("[model-writer:parquet] stage=cleanup raw_root absent, nothing to remove");
            return Ok(());
        }

        let mut removed = 0usize;
        for table in CanonicalRawTable::all_phase1() {
            let table_root = raw_root
                .join(table.as_str())
                .join(format!("project_name={}", ctx.project_name))
                .join(format!("dbnum={}", self.dbnum));
            if table_root.exists() {
                fs::remove_dir_all(&table_root).with_context(|| {
                    format!(
                        "failed to remove canonical raw dir {}",
                        table_root.display()
                    )
                })?;
                removed += 1;
            }
        }
        println!(
            "[model-writer:parquet] stage=cleanup done tables_removed={}",
            removed
        );
        Ok(())
    }

    async fn write_base_batch(
        &self,
        batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<WriteBaseReport> {
        let writer = self.writer()?;
        let planner = CanonicalRawPlanner;
        let mut canonical = planner.plan_shape_instances(batch.shape_insts);
        canonical.refresh_row_counts();
        println!(
            "[model-writer:parquet] stage=base batch={} inst_info={} inst_geo={} aabb={}",
            batch.batch_id,
            canonical.inst_info.len(),
            canonical.inst_geo.len(),
            canonical.aabb.len()
        );
        let summary = writer.write_raw_batch(batch.batch_id, &canonical)?;
        self.total_rows_written
            .fetch_add(summary.total_rows as u64, Ordering::Relaxed);
        self.batches_completed.fetch_add(1, Ordering::Relaxed);
        println!(
            "[model-writer:parquet] stage=base batch={} done total_rows={} summary={}",
            batch.batch_id, summary.total_rows, summary.summary_path
        );
        Ok(WriteBaseReport {
            batch_id: batch.batch_id,
            missing_neg_count: 0,
            missing_neg_carriers: Vec::new(),
        })
    }

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        // Mesh persistence falls outside Phase 1 canonical raw scope (mission 05
        // §Layout: raw tables only). Log and skip; mesh canonical work tracked
        // separately in mission docs Phase 2+.
        println!(
            "[model-writer:parquet] stage=mesh_results batch={} mesh_results={} file_mesh_state={} action=skipped reason=phase1_canonical_raw_only",
            batch.batch_id,
            batch.mesh_results.len(),
            batch.file_mesh_state
        );
        Ok(())
    }

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()> {
        // `raw_inst_relate_aabb` already lands during `write_base_batch` via the
        // canonical planner (`plan_shape_instances` emits inst_relate_aabb rows
        // for instances with `aabb`), so this call is a NoOp on the parquet
        // sink. Surreal backend, in contrast, separates this stage because it
        // also persists the mesh-derived aabb keys here.
        println!(
            "[model-writer:parquet] stage=inst_relate_aabb batch={} action=noop reason=already_emitted_in_base_batch",
            batch.batch_id
        );
        Ok(())
    }

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize> {
        // File sink has no cross-batch DB view; reconcile semantics are
        // approximated by reporting 0 inserts and surfacing the inputs in the
        // log so downstream validation can flag the difference.
        println!(
            "[model-writer:parquet] stage=reconcile_missing_neg all_refnos={} candidate_carriers={} inserted=0 approximate=true",
            request.all_refnos.len(),
            request.candidate_carriers.len()
        );
        Ok(0)
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        // Boolean tables are Phase 2 (mission 00 / 08).
        Ok(BooleanBridgeReport::skipped(
            "parquet",
            request.bool_tasks.len(),
            "phase2_boolean_not_supported",
        ))
    }

    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary> {
        let batches = self.batches_completed.load(Ordering::Relaxed);
        let rows = self.total_rows_written.load(Ordering::Relaxed);
        println!(
            "[model-writer:parquet] stage=finalize total_batches={} completed_batches={} parquet_batches_written={} parquet_total_rows={}",
            request.total_batches, request.completed_batches, batches, rows
        );
        Ok(FinalizeSummary {
            backend: self.name(),
            total_batches: request.total_batches,
            completed_batches: request.completed_batches,
        })
    }
}
