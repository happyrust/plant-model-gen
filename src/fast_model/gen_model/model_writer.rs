use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aios_core::error::init_save_database_error;
use aios_core::geometry::ShapeInstancesData;
use aios_core::{RefnoEnum, model_primary_db};
use anyhow::Context;
use async_trait::async_trait;
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;

use super::boolean_task::BooleanTask;
use super::manifold_bool::{BoolWorkerReport, run_bool_worker_from_tasks};
use super::mesh_generate::{MeshResult, run_boolean_worker};
use super::mesh_state::{flush_aabb_cache, use_file_mesh_state};
use super::pdms_inst::{self, SaveInstanceDataReport};
use super::pdms_inst_surreal;
use crate::options::{BooleanPipelineMode, DbOptionExt, ModelWriterMode};

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
    pub db_option: Arc<aios_core::options::DbOption>,
    pub bool_tasks: Vec<BooleanTask>,
    pub use_surrealdb: bool,
    pub defer_db_write: bool,
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
    fn db_legacy_executed() -> Self {
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

    fn skipped(pipeline: &'static str, total: usize, reason: &str) -> Self {
        println!(
            "[model-writer:surreal] stage=boolean_bridge skipped pipeline={} reason={} total={}",
            pipeline, reason, total
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
pub trait ModelWriteBackend: Send + Sync {
    fn name(&self) -> &'static str;

    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()>;

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()>;

    async fn write_base_batch(
        &self,
        batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<SaveInstanceDataReport>;

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()>;

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()>;

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize>;

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport>;

    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary>;
}

pub fn create_model_writer(db_option: &DbOptionExt) -> anyhow::Result<Arc<dyn ModelWriteBackend>> {
    match db_option.model_writer_mode {
        ModelWriterMode::Surreal => {
            println!("[model-writer] factory selected primary=surreal mirror=none fail_fast=true");
            Ok(Arc::new(SurrealModelWriteBackend))
        }
        ModelWriterMode::DrainOnly => {
            anyhow::bail!(
                "drain-only is an explicit non-persistent sink and does not create a persistence backend"
            )
        }
    }
}

#[derive(Debug, Default)]
pub struct SurrealModelWriteBackend;

#[async_trait]
impl ModelWriteBackend for SurrealModelWriteBackend {
    fn name(&self) -> &'static str {
        "surreal"
    }

    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()> {
        println!(
            "[model-writer:surreal] stage=init project={} use_surrealdb={} defer_db_write={} mode={}",
            context.project_name,
            context.use_surrealdb,
            context.defer_db_write,
            context.mode.as_str()
        );
        anyhow::ensure!(
            context.use_surrealdb,
            "Surreal model writer requires use_surrealdb=true for the current input/write contract"
        );
        aios_core::rs_surreal::inst::init_model_tables()
            .await
            .context("model_writer surreal init_model_tables failed")?;
        Ok(())
    }

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()> {
        println!(
            "[model-writer:surreal] stage=cleanup seed_refnos={}",
            request.seed_refnos.len()
        );
        pdms_inst::pre_cleanup_for_regen(request.seed_refnos)
            .await
            .context("model_writer surreal legacy cleanup failed")?;
        pdms_inst_surreal::pre_cleanup_for_regen_surreal(request.seed_refnos)
            .await
            .context("model_writer surreal relation cleanup failed")?;
        println!(
            "[model-writer:surreal] stage=cleanup done seed_refnos={}",
            request.seed_refnos.len()
        );
        Ok(())
    }

    async fn write_base_batch(
        &self,
        batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<SaveInstanceDataReport> {
        println!(
            "[model-writer:surreal] stage=base batch={} inst_info={} inst_tubi={} geo_keys={}",
            batch.batch_id,
            batch.shape_insts.inst_info_map.len(),
            batch.shape_insts.inst_tubi_map.len(),
            batch.shape_insts.inst_geos_map.len()
        );
        let mesh_results: HashMap<u64, MeshResult> = HashMap::new();
        let report = pdms_inst::save_instance_data_with_report(
            batch.shape_insts,
            batch.replace_exist,
            &mesh_results,
            batch.mesh_aabb_map,
            batch.write_inst_relate_aabb,
        )
        .await
        .with_context(|| format!("model_writer surreal base batch {} failed", batch.batch_id))?;
        println!(
            "[model-writer:surreal] stage=base batch={} done missing_neg_candidates={}",
            batch.batch_id,
            report.missing_neg_carriers.len()
        );
        Ok(report)
    }

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        if use_file_mesh_state() {
            flush_aabb_cache();
            println!(
                "[model-writer:surreal] stage=mesh_results batch={} file_mesh_state=true flushed_aabb_cache=true",
                batch.batch_id
            );
            return Ok(());
        }

        if batch.mesh_results.is_empty() {
            println!(
                "[model-writer:surreal] stage=mesh_results batch={} mesh_results=0",
                batch.batch_id
            );
            return Ok(());
        }

        let pts_written = save_pts_to_surreal_strict(batch.mesh_pts_map)
            .await
            .with_context(|| {
                format!(
                    "model_writer surreal mesh pts batch {} failed",
                    batch.batch_id
                )
            })?;
        let aabb_written = save_aabb_to_surreal_strict(batch.mesh_aabb_map)
            .await
            .with_context(|| {
                format!(
                    "model_writer surreal mesh aabb batch {} failed",
                    batch.batch_id
                )
            })?;

        let mut update_sql = String::new();
        for (geo_hash, mesh_result) in batch.mesh_results {
            update_sql.push_str(&mesh_result.to_update_sql(&geo_hash.to_string()));
        }

        if !update_sql.is_empty() {
            model_primary_db().query(&update_sql).await.map_err(|e| {
                let preview: String = update_sql.chars().take(500).collect();
                anyhow::anyhow!(
                    "model_writer surreal mesh result batch {} failed: error={}, sql_preview={}",
                    batch.batch_id,
                    e,
                    preview
                )
            })?;
        }

        println!(
            "[model-writer:surreal] stage=mesh_results batch={} mesh_results={} pts_rows={} aabb_rows={} update_sql_len={}",
            batch.batch_id,
            batch.mesh_results.len(),
            pts_written,
            aabb_written,
            update_sql.len()
        );
        Ok(())
    }

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()> {
        let (aabb_rows_map, inst_relate_aabb_rows, inst_relate_aabb_ids) =
            pdms_inst::build_inst_relate_aabb_rows(
                batch.shape_insts,
                batch.mesh_results,
                batch.mesh_aabb_map,
            )?;
        let aabb_count = aabb_rows_map.len();
        let rel_count = inst_relate_aabb_rows.len();
        pdms_inst::save_inst_relate_aabb_rows(
            &aabb_rows_map,
            &inst_relate_aabb_rows,
            &inst_relate_aabb_ids,
        )
        .await
        .with_context(|| {
            format!(
                "model_writer surreal inst_relate_aabb batch {} failed",
                batch.batch_id
            )
        })?;
        println!(
            "[model-writer:surreal] stage=inst_relate_aabb batch={} aabb_rows={} relation_rows={}",
            batch.batch_id, aabb_count, rel_count
        );
        Ok(())
    }

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize> {
        println!(
            "[model-writer:surreal] stage=reconcile_missing_neg all_refnos={} candidate_carriers={}",
            request.all_refnos.len(),
            request.candidate_carriers.len()
        );
        let inserted =
            pdms_inst::reconcile_missing_neg_relate(request.all_refnos, request.candidate_carriers)
                .await
                .context("model_writer surreal reconcile_missing_neg failed")?;
        println!(
            "[model-writer:surreal] stage=reconcile_missing_neg done inserted={}",
            inserted
        );
        Ok(inserted)
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        match request.mode {
            BooleanPipelineMode::DbLegacy => {
                if request.use_surrealdb && !request.defer_db_write {
                    println!("[model-writer:surreal] stage=boolean_bridge pipeline=db_legacy");
                    run_boolean_worker(request.db_option, 100)
                        .await
                        .context("model_writer surreal db_legacy boolean bridge failed")?;
                    Ok(BooleanBridgeReport::db_legacy_executed())
                } else {
                    Ok(BooleanBridgeReport::skipped(
                        "db_legacy",
                        0,
                        "use_surrealdb/defer_db_write guard",
                    ))
                }
            }
            BooleanPipelineMode::MemoryTasks => {
                if !request.use_surrealdb {
                    return Ok(BooleanBridgeReport::skipped(
                        "memory_tasks",
                        request.bool_tasks.len(),
                        "use_surrealdb=false",
                    ));
                }
                println!(
                    "[model-writer:surreal] stage=boolean_bridge pipeline=memory_tasks total_tasks={}",
                    request.bool_tasks.len()
                );
                let report =
                    run_bool_worker_from_tasks(request.bool_tasks, request.db_option, None)
                        .await
                        .context("model_writer surreal memory_tasks boolean bridge failed")?;
                Ok(report.into())
            }
        }
    }

    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary> {
        println!(
            "[model-writer:surreal] stage=finalize total_batches={} completed_batches={} mesh_cache_hits={} mesh_new_generated={} missing_neg_candidates={}",
            request.total_batches,
            request.completed_batches,
            request.mesh_cache_hits,
            request.mesh_new_generated,
            request.missing_neg_candidates
        );
        Ok(FinalizeSummary {
            backend: self.name(),
            total_batches: request.total_batches,
            completed_batches: request.completed_batches,
        })
    }
}

async fn save_aabb_to_surreal_strict(aabb_map: &DashMap<String, Aabb>) -> anyhow::Result<usize> {
    if aabb_map.is_empty() {
        return Ok(0);
    }

    let keys = aabb_map
        .iter()
        .map(|kv| kv.key().clone())
        .collect::<Vec<_>>();
    let mut written = 0usize;
    for chunk in keys.chunks(300) {
        let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
        for k in chunk {
            let Some(v) = aabb_map.get(k) else {
                continue;
            };
            let d = serde_json::to_string(v.value())?;
            let id_key = if k.starts_with("aabb:") {
                k.to_string()
            } else {
                format!("aabb:⟨{}⟩", k)
            };
            rows.push(format!("{{'id':{id_key}, 'd':{d}}}"));
        }
        if rows.is_empty() {
            continue;
        }
        let sql = format!("INSERT IGNORE INTO aabb [{}];", rows.join(","));
        if let Err(e) = model_primary_db().query(&sql).await {
            init_save_database_error(
                &format!("{sql}\n-- err: {e}"),
                &std::panic::Location::caller().to_string(),
            );
            anyhow::bail!("写入 mesh aabb 失败: {e}");
        }
        written += rows.len();
    }
    Ok(written)
}

async fn save_pts_to_surreal_strict(vec3_map: &DashMap<u64, String>) -> anyhow::Result<usize> {
    if vec3_map.is_empty() {
        return Ok(0);
    }

    let keys = vec3_map.iter().map(|kv| *kv.key()).collect::<Vec<_>>();
    let mut written = 0usize;
    for chunk in keys.chunks(100) {
        let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
        for &k in chunk {
            let Some(v) = vec3_map.get(&k) else {
                continue;
            };
            rows.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", k, v.value()));
        }
        if rows.is_empty() {
            continue;
        }
        let sql = format!("INSERT IGNORE INTO vec3 [{}];", rows.join(","));
        if let Err(e) = model_primary_db().query(&sql).await {
            init_save_database_error(
                &format!("{sql}\n-- err: {e}"),
                &std::panic::Location::caller().to_string(),
            );
            anyhow::bail!("写入 mesh pts/vec3 失败: {e}");
        }
        written += rows.len();
    }
    Ok(written)
}

#[derive(Debug, Default)]
pub struct DrainOnlyStats {
    pub batches: usize,
    pub instances: usize,
    pub inst_info: usize,
    pub inst_tubi: usize,
    pub geo_keys: usize,
    pub geo_instances: usize,
    pub neg_relations: usize,
    pub ngmr_relations: usize,
    pub elapsed: Duration,
}

impl DrainOnlyStats {
    fn add_batch(&mut self, batch: &ShapeInstancesData) {
        self.batches += 1;
        self.instances += batch.inst_cnt();
        self.inst_info += batch.inst_info_map.len();
        self.inst_tubi += batch.inst_tubi_map.len();
        self.geo_keys += batch.inst_geos_map.len();
        self.geo_instances += batch
            .inst_geos_map
            .values()
            .map(|geos| geos.insts.len())
            .sum::<usize>();
        self.neg_relations += batch.neg_relate_map.values().map(Vec::len).sum::<usize>();
        self.ngmr_relations += batch
            .ngmr_neg_relate_map
            .values()
            .map(Vec::len)
            .sum::<usize>();
    }

    pub fn print_summary(&self) {
        println!(
            "[model-writer:drain-only] summary: batches={} instances={} inst_info={} inst_tubi={} geo_keys={} geo_instances={} neg_relations={} ngmr_relations={} elapsed_ms={}",
            self.batches,
            self.instances,
            self.inst_info,
            self.inst_tubi,
            self.geo_keys,
            self.geo_instances,
            self.neg_relations,
            self.ngmr_relations,
            self.elapsed.as_millis()
        );
    }
}

pub async fn run_drain_only_sink(
    receiver: flume::Receiver<ShapeInstancesData>,
) -> anyhow::Result<DrainOnlyStats> {
    let started = Instant::now();
    let mut stats = DrainOnlyStats::default();

    while let Ok(batch) = receiver.recv_async().await {
        stats.add_batch(&batch);

        if stats.batches % 100 == 0 {
            println!(
                "[model-writer:drain-only] drained batches={} instances={} geo_instances={} elapsed_ms={}",
                stats.batches,
                stats.instances,
                stats.geo_instances,
                started.elapsed().as_millis()
            );
        }
    }

    stats.elapsed = started.elapsed();
    Ok(stats)
}
