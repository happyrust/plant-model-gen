use std::collections::HashMap;
use std::sync::Arc;

use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use async_trait::async_trait;
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;

use super::boolean_task::BooleanTask;
use super::manifold_bool::BoolWorkerReport;
use super::mesh_generate::MeshResult;
use crate::options::{BooleanPipelineMode, DbOptionExt, ModelWriterMode};

// Canonical raw record types & planner; sits below trait backends (parallel
// canonical raw boundary work, see docs/development/model-writer-storage/).
pub use super::canonical_records::{
    CanonicalRawBatch, CanonicalRawPlanner, CanonicalRawRowCounts, CanonicalRawTable,
};

mod compare;
mod drain_only;
#[cfg(feature = "ducklake")]
mod ducklake;
#[cfg(feature = "model-writer-mock")]
mod mock;
mod parquet;
mod surreal;

pub use compare::CompareModelWriterBackend;
pub use drain_only::{DrainOnlyModelWriterBackend, DrainOnlyStats, run_drain_only_sink};
#[cfg(feature = "ducklake")]
pub use ducklake::DuckLakeModelWriterBackend;
#[cfg(feature = "model-writer-mock")]
pub use mock::RecordingBackend;
// Canonical raw sink scaffold + v3 Phase B ModelWriterBackend impl (file-oriented).
pub use parquet::{
    CanonicalParquetTableSummary, CanonicalParquetWriter, CanonicalParquetWriterConfig,
    ParquetModelWriterBackend,
};
pub use surreal::SurrealModelWriterBackend;

#[derive(Debug, Clone)]
pub struct ModelWriterContext {
    pub project_name: String,
    pub use_surrealdb: bool,
    pub defer_db_write: bool,
    pub mode: ModelWriterMode,
}

impl ModelWriterContext {
    pub fn from_db_option(db_option: &DbOptionExt) -> Self {
        Self {
            project_name: db_option.inner.project_name.clone(),
            use_surrealdb: db_option.use_surrealdb,
            defer_db_write: db_option.defer_db_write,
            mode: db_option.model_writer_mode,
        }
    }
}

pub struct BaseInstanceBatch<'a> {
    pub batch_id: u64,
    pub shape_insts: &'a ShapeInstancesData,
    pub mesh_aabb_map: &'a DashMap<String, Aabb>,
    pub replace_exist: bool,
    pub write_inst_relate_aabb: bool,
}

pub struct MeshResultBatch<'a> {
    pub batch_id: u64,
    pub mesh_results: &'a HashMap<u64, MeshResult>,
    pub mesh_aabb_map: &'a DashMap<String, Aabb>,
    pub mesh_pts_map: &'a DashMap<u64, String>,
    /// 来源于 `mesh_state::use_file_mesh_state()` 的进程级开关；显式塞进 batch
    /// 让 trait 行为完全由参数决定（W3 修复）。
    pub file_mesh_state: bool,
}

pub struct InstRelateAabbBatch<'a> {
    pub batch_id: u64,
    pub shape_insts: &'a ShapeInstancesData,
    pub mesh_results: &'a HashMap<u64, MeshResult>,
    pub mesh_aabb_map: &'a DashMap<String, Aabb>,
}

pub struct CleanupRequest<'a> {
    pub seed_refnos: &'a [RefnoEnum],
}

pub struct ReconcileRequest<'a> {
    pub all_refnos: &'a [RefnoEnum],
    pub candidate_carriers: &'a [RefnoEnum],
}

pub struct BooleanBridgeRequest {
    pub mode: BooleanPipelineMode,
    // TODO(P5): replace with a minimal BridgeContext to remove DbOption coupling
    // from the trait surface. See findings.md §3 (decision: keep short-term).
    pub db_option: Arc<aios_core::options::DbOption>,
    pub bool_tasks: Vec<BooleanTask>,
}

#[derive(Debug, Clone)]
pub struct BooleanBridgeReport {
    pub pipeline: &'static str,
    pub total: usize,
    pub cata_cnt: usize,
    pub inst_cnt: usize,
    pub success: usize,
    pub failed: usize,
    pub skipped: usize,
    pub deferred_mode: bool,
}

impl BooleanBridgeReport {
    pub(crate) fn db_legacy_executed() -> Self {
        Self {
            pipeline: "db_legacy",
            total: 0,
            cata_cnt: 0,
            inst_cnt: 0,
            success: 0,
            failed: 0,
            skipped: 0,
            deferred_mode: false,
        }
    }

    pub(crate) fn skipped(pipeline: &'static str, total: usize, reason: &str) -> Self {
        println!(
            "[model-writer:{}] stage=boolean_bridge skipped pipeline={} reason={} total={}",
            pipeline, pipeline, reason, total
        );
        Self {
            pipeline,
            total,
            cata_cnt: 0,
            inst_cnt: 0,
            success: 0,
            failed: 0,
            skipped: total,
            deferred_mode: false,
        }
    }
}

impl From<BoolWorkerReport> for BooleanBridgeReport {
    fn from(report: BoolWorkerReport) -> Self {
        Self {
            pipeline: "memory_tasks",
            total: report.total,
            cata_cnt: report.cata_cnt,
            inst_cnt: report.inst_cnt,
            success: report.success,
            failed: report.failed,
            skipped: report.skipped,
            deferred_mode: report.deferred_mode,
        }
    }
}

/// `write_base_batch` 的对外 report，不耦合具体 backend 内部类型。
///
/// 字段名 `missing_neg_carriers` 与历史 `pdms_inst::SaveInstanceDataReport` 保持兼容，
/// 让 orchestrator 调用代码改动量为 0。
#[derive(Debug, Clone, Default)]
pub struct WriteBaseReport {
    pub batch_id: u64,
    pub missing_neg_count: usize,
    /// 历史调用方需要的 `RefnoEnum` 集合，由 backend 在写 base batch 时收集。
    ///
    /// 这是 trait 接口里唯一仍带 `aios_core::RefnoEnum` 的字段，等同于把
    /// "missing-neg 收集" 语义抬到 trait 层。P5 backlog (`T5.1`) 计划把这个字段
    /// 拆为独立 trait 方法 `take_missing_neg_carriers(&self) -> Vec<RefnoEnum>`，
    /// 让 backend 决定是否暴露 carriers；当前先保留以减少本 PR 的接口侵入面。
    pub missing_neg_carriers: Vec<RefnoEnum>,
}

#[derive(Debug, Clone, Default)]
pub struct FinalizeRequest {
    pub total_batches: u64,
    pub completed_batches: usize,
    pub mesh_cache_hits: usize,
    pub mesh_new_generated: usize,
    pub missing_neg_candidates: usize,
}

#[derive(Debug, Clone)]
pub struct FinalizeSummary {
    pub backend: &'static str,
    pub total_batches: u64,
    pub completed_batches: usize,
}

#[async_trait]
pub trait ModelWriterBackend: Send + Sync {
    fn name(&self) -> &'static str;

    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()>;

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()>;

    async fn write_base_batch(
        &self,
        batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<WriteBaseReport>;

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()>;

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()>;

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize>;

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport>;

    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary>;
}

/// Build a single backend instance for the given mode. Used both for the
/// primary backend and (when configured) for the compare-mode candidate.
fn build_single_backend(
    db_option: &DbOptionExt,
    mode: ModelWriterMode,
) -> anyhow::Result<Arc<dyn ModelWriterBackend>> {
    let backend: Arc<dyn ModelWriterBackend> = match mode {
        ModelWriterMode::Surreal => Arc::new(SurrealModelWriterBackend::default()),
        ModelWriterMode::DrainOnly => Arc::new(DrainOnlyModelWriterBackend::default()),
        ModelWriterMode::Parquet => {
            let output_root = db_option
                .parquet_model_writer_output_root
                .as_ref()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "model_writer=parquet requires `parquet_model_writer_output_root` to be set in DbOptionExt"
                    )
                })?;
            Arc::new(ParquetModelWriterBackend::with_dbnum(
                std::path::PathBuf::from(output_root),
                db_option
                    .parquet_model_writer_dbnum
                    .unwrap_or(ParquetModelWriterBackend::DEFAULT_DBNUM),
            ))
        }
        ModelWriterMode::DuckLake => {
            #[cfg(feature = "ducklake")]
            {
                Arc::new(DuckLakeModelWriterBackend::new())
            }
            #[cfg(not(feature = "ducklake"))]
            {
                anyhow::bail!(
                    "model_writer=ducklake requires --features ducklake build (v3 Phase D skeleton, real implementation lands in v4 per mission docs/04-ducklake-writer.md)"
                )
            }
        }
    };
    Ok(backend)
}

pub fn create_model_writer(db_option: &DbOptionExt) -> anyhow::Result<Arc<dyn ModelWriterBackend>> {
    let primary = build_single_backend(db_option, db_option.model_writer_mode)?;

    // v3 Phase C: compare mode dual-write wrapper.
    if let Some(candidate_mode) = db_option.model_writer_compare_with {
        if candidate_mode == db_option.model_writer_mode {
            anyhow::bail!(
                "model_writer_compare_with={} duplicates primary model_writer_mode={}; remove compare_with or pick a different backend",
                candidate_mode.as_str(),
                db_option.model_writer_mode.as_str()
            );
        }
        if matches!(candidate_mode, ModelWriterMode::DrainOnly) {
            anyhow::bail!(
                "model_writer_compare_with=drain-only is not supported: DrainOnly is a baseline sink, not a candidate writer (mission docs/05 + v2 invariant)"
            );
        }
        let candidate = build_single_backend(db_option, candidate_mode)?;
        let wrapper = Arc::new(CompareModelWriterBackend::new(
            primary.clone(),
            candidate.clone(),
        ));
        println!(
            "[model-writer] factory selected primary={} mirror={} fail_fast=true wrapper=compare",
            primary.name(),
            candidate.name()
        );
        return Ok(wrapper);
    }

    println!(
        "[model-writer] factory selected primary={} mirror=none fail_fast=true",
        primary.name()
    );
    Ok(primary)
}
