use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use aios_core::options::DbOption;
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;
use serde::Serialize;

use crate::fast_model::gen_model::boolean_task::BooleanTask;
use crate::fast_model::gen_model::manifold_bool::run_bool_worker_from_tasks;
use crate::fast_model::mesh_generate::{MeshResult, run_boolean_worker};
use crate::fast_model::pdms_inst::save_instance_data_with_report;
use crate::fast_model::pdms_inst::{
    build_inst_relate_aabb_rows, reconcile_missing_neg_relate, save_inst_relate_aabb_rows,
};
use crate::fast_model::utils::{save_aabb_to_surreal, save_pts_to_surreal};
use crate::options::{BooleanPipelineMode, ModelWriterMode};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelWriterStageStatus {
    Implemented,
    Executed,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelWriterStageReport {
    pub stage: &'static str,
    pub status: ModelWriterStageStatus,
    pub item_count: usize,
    pub skipped_reason: Option<&'static str>,
}

impl ModelWriterStageReport {
    fn executed(stage: &'static str, item_count: usize) -> Self {
        Self {
            stage,
            status: ModelWriterStageStatus::Executed,
            item_count,
            skipped_reason: None,
        }
    }

    fn implemented(stage: &'static str) -> Self {
        Self {
            stage,
            status: ModelWriterStageStatus::Implemented,
            item_count: 0,
            skipped_reason: None,
        }
    }

    fn skipped(stage: &'static str, reason: &'static str, item_count: usize) -> Self {
        Self {
            stage,
            status: ModelWriterStageStatus::Skipped,
            item_count,
            skipped_reason: Some(reason),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelWriterContractEvidence {
    pub backend: &'static str,
    pub writes_to_surreal: bool,
    pub runs_downstream_pipeline: bool,
    pub stages: Vec<ModelWriterStageReport>,
}

#[derive(Debug, Default, Clone)]
pub struct DrainOnlyStats {
    pub batches: usize,
    pub instances: usize,
    pub inst_info: usize,
    pub inst_tubi: usize,
    pub geo_keys: usize,
    pub geo_instances: usize,
    pub neg_relations: usize,
    pub ngmr_relations: usize,
    pub skipped_stages: usize,
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

#[derive(Debug, Default, Clone)]
pub struct ModelWriteBatchReport {
    pub missing_neg_carriers: Vec<RefnoEnum>,
}

#[derive(Debug, Default, Clone)]
pub struct ModelWriterFinishReport {
    pub writer_name: &'static str,
    pub drain_only_stats: Option<DrainOnlyStats>,
    pub stage_reports: Vec<ModelWriterStageReport>,
}

pub struct BooleanBridgeRequest {
    pub mode: BooleanPipelineMode,
    pub db_option: Arc<DbOption>,
    pub use_surrealdb: bool,
    pub defer_db_write: bool,
    pub enable_db_backfill: bool,
    pub bool_tasks: Vec<BooleanTask>,
}

#[derive(Debug, Default, Clone)]
pub struct BooleanBridgeReport {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub skipped: usize,
    pub skipped_reason: Option<&'static str>,
}

#[async_trait::async_trait]
pub trait ModelWriterBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn writes_to_surreal(&self) -> bool;

    fn runs_downstream_pipeline(&self) -> bool;

    /// Called once before any writer work. Kept non-destructive for Surreal compatibility.
    async fn init(&self) -> anyhow::Result<ModelWriterStageReport> {
        Ok(ModelWriterStageReport::executed("init", 0))
    }

    /// Called before generation when a backend needs cleanup. Default is a safe no-op.
    async fn cleanup(&self) -> anyhow::Result<ModelWriterStageReport> {
        Ok(ModelWriterStageReport::skipped(
            "cleanup",
            "no backend cleanup configured",
            0,
        ))
    }

    /// May be called concurrently by multiple base-writer workers.
    async fn write_base_batch(
        &self,
        batch: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriteBatchReport>;

    async fn persist_mesh_results(
        &self,
        mesh_results: &HashMap<u64, MeshResult>,
        mesh_aabb_map: &DashMap<String, Aabb>,
        mesh_pts_map: &DashMap<u64, String>,
    ) -> anyhow::Result<ModelWriterStageReport>;

    async fn persist_inst_relate_aabb(
        &self,
        shape_insts: &ShapeInstancesData,
        mesh_results: &HashMap<u64, MeshResult>,
        mesh_aabb_map: &DashMap<String, Aabb>,
        skip_inst_relate_aabb: bool,
    ) -> anyhow::Result<ModelWriterStageReport>;

    async fn reconcile_missing_neg_relations(
        &self,
        all_refnos: &[RefnoEnum],
        missing_neg_carriers: &[RefnoEnum],
    ) -> anyhow::Result<ModelWriterStageReport>;

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport>;

    /// Called once after all writer stages finish.
    async fn finalize(&self) -> anyhow::Result<ModelWriterFinishReport> {
        Ok(ModelWriterFinishReport {
            writer_name: self.name(),
            drain_only_stats: None,
            stage_reports: Vec::new(),
        })
    }
}

pub type ModelWriter = dyn ModelWriterBackend;

pub struct SurrealModelWriterBackend {
    mesh_aabb_map: Arc<DashMap<String, Aabb>>,
    missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,
}

impl SurrealModelWriterBackend {
    pub fn new(
        mesh_aabb_map: Arc<DashMap<String, Aabb>>,
        missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,
    ) -> Self {
        Self {
            mesh_aabb_map,
            missing_neg_carriers,
        }
    }
}

#[async_trait::async_trait]
impl ModelWriterBackend for SurrealModelWriterBackend {
    fn name(&self) -> &'static str {
        "surreal"
    }

    fn writes_to_surreal(&self) -> bool {
        true
    }

    fn runs_downstream_pipeline(&self) -> bool {
        true
    }

    async fn write_base_batch(
        &self,
        batch: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriteBatchReport> {
        let save_report = save_instance_data_with_report(
            batch,
            false,
            &HashMap::new(),
            &self.mesh_aabb_map,
            false,
        )
        .await?;
        if !save_report.missing_neg_carriers.is_empty() {
            let mut guard = self
                .missing_neg_carriers
                .lock()
                .map_err(|_| anyhow::anyhow!("missing_neg_carriers mutex poisoned"))?;
            guard.extend(save_report.missing_neg_carriers.iter().copied());
        }
        Ok(ModelWriteBatchReport {
            missing_neg_carriers: save_report.missing_neg_carriers,
        })
    }

    async fn persist_mesh_results(
        &self,
        mesh_results: &HashMap<u64, MeshResult>,
        mesh_aabb_map: &DashMap<String, Aabb>,
        mesh_pts_map: &DashMap<u64, String>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        if crate::fast_model::gen_model::mesh_state::use_file_mesh_state() {
            crate::fast_model::gen_model::mesh_state::flush_aabb_cache();
            return Ok(ModelWriterStageReport::skipped(
                "mesh_persist",
                "file mesh state active; flushed aabb cache",
                mesh_results.len(),
            ));
        }

        if mesh_results.is_empty() {
            return Ok(ModelWriterStageReport::skipped(
                "mesh_persist",
                "no mesh results",
                0,
            ));
        }

        // Preserve existing ordering: persist aabb/pts entities before inst_geo references them.
        save_pts_to_surreal(mesh_pts_map).await;
        save_aabb_to_surreal(mesh_aabb_map).await;

        let mut update_sql = String::new();
        for (geo_hash, mesh_result) in mesh_results {
            update_sql.push_str(&mesh_result.to_update_sql(&geo_hash.to_string()));
        }

        if update_sql.is_empty() {
            return Ok(ModelWriterStageReport::skipped(
                "mesh_persist",
                "mesh results produced no update sql",
                mesh_results.len(),
            ));
        }

        aios_core::model_primary_db()
            .query(&update_sql)
            .await
            .map_err(|e| {
                let preview: String = update_sql.chars().take(500).collect();
                anyhow::anyhow!(
                    "回写 inst_geo mesh 结果失败: error={}, sql_preview={}",
                    e,
                    preview
                )
            })?;

        Ok(ModelWriterStageReport::executed(
            "mesh_persist",
            mesh_results.len(),
        ))
    }

    async fn persist_inst_relate_aabb(
        &self,
        shape_insts: &ShapeInstancesData,
        mesh_results: &HashMap<u64, MeshResult>,
        mesh_aabb_map: &DashMap<String, Aabb>,
        skip_inst_relate_aabb: bool,
    ) -> anyhow::Result<ModelWriterStageReport> {
        if skip_inst_relate_aabb {
            return Ok(ModelWriterStageReport::skipped(
                "inst_relate_aabb",
                "AIOS_SKIP_INST_RELATE_AABB",
                shape_insts.inst_cnt(),
            ));
        }

        let (aabb_rows_map, inst_relate_aabb_rows, inst_relate_aabb_ids) =
            build_inst_relate_aabb_rows(shape_insts, mesh_results, mesh_aabb_map)?;
        let row_count = inst_relate_aabb_rows.len();
        save_inst_relate_aabb_rows(
            &aabb_rows_map,
            &inst_relate_aabb_rows,
            &inst_relate_aabb_ids,
        )
        .await?;

        Ok(ModelWriterStageReport::executed(
            "inst_relate_aabb",
            row_count,
        ))
    }

    async fn reconcile_missing_neg_relations(
        &self,
        all_refnos: &[RefnoEnum],
        missing_neg_carriers: &[RefnoEnum],
    ) -> anyhow::Result<ModelWriterStageReport> {
        if missing_neg_carriers.is_empty() {
            return Ok(ModelWriterStageReport::skipped(
                "missing_neg_reconcile",
                "no missing negative relation carriers",
                0,
            ));
        }

        reconcile_missing_neg_relate(all_refnos, missing_neg_carriers).await?;
        Ok(ModelWriterStageReport::executed(
            "missing_neg_reconcile",
            missing_neg_carriers.len(),
        ))
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        match request.mode {
            BooleanPipelineMode::DbLegacy => {
                if request.use_surrealdb && !request.defer_db_write {
                    run_boolean_worker(request.db_option, 100).await?;
                    Ok(BooleanBridgeReport {
                        total: 0,
                        success: 0,
                        failed: 0,
                        skipped: 0,
                        skipped_reason: None,
                    })
                } else {
                    Ok(BooleanBridgeReport {
                        skipped: request.bool_tasks.len(),
                        skipped_reason: Some("db_legacy conditions not met"),
                        ..BooleanBridgeReport::default()
                    })
                }
            }
            BooleanPipelineMode::MemoryTasks => {
                if !request.use_surrealdb {
                    return Ok(BooleanBridgeReport {
                        skipped: request.bool_tasks.len(),
                        skipped_reason: Some("use_surrealdb=false"),
                        ..BooleanBridgeReport::default()
                    });
                }
                if request.bool_tasks.is_empty() {
                    return Ok(BooleanBridgeReport {
                        skipped_reason: Some("no boolean tasks"),
                        ..BooleanBridgeReport::default()
                    });
                }

                let mut bool_tasks = request.bool_tasks;
                if request.enable_db_backfill {
                    match super::boolean_backfill::backfill_cata_tasks_from_db(
                        &mut bool_tasks,
                        request.use_surrealdb,
                    )
                    .await
                    {
                        Ok(count) if count > 0 => {
                            println!(
                                "[gen_model] DB backfill 补齐了 {} 个 cata 布尔任务，当前总数 {}",
                                count,
                                bool_tasks.len()
                            );
                        }
                        Err(e) => {
                            eprintln!("[gen_model] DB backfill 失败（非致命，继续执行）: {}", e);
                        }
                        _ => {}
                    }
                }

                let report =
                    run_bool_worker_from_tasks(bool_tasks, request.db_option, None).await?;
                Ok(BooleanBridgeReport {
                    total: report.total,
                    success: report.success,
                    failed: report.failed,
                    skipped: report.skipped,
                    skipped_reason: None,
                })
            }
        }
    }
}

pub struct DrainOnlyModelWriterBackend {
    started: Instant,
    stats: Mutex<DrainOnlyStats>,
    stage_reports: Mutex<Vec<ModelWriterStageReport>>,
}

impl DrainOnlyModelWriterBackend {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            stats: Mutex::new(DrainOnlyStats::default()),
            stage_reports: Mutex::new(Vec::new()),
        }
    }

    fn record_skipped(
        &self,
        stage: &'static str,
        reason: &'static str,
        item_count: usize,
    ) -> anyhow::Result<ModelWriterStageReport> {
        let report = ModelWriterStageReport::skipped(stage, reason, item_count);
        {
            let mut stats = self
                .stats
                .lock()
                .map_err(|_| anyhow::anyhow!("drain-only stats mutex poisoned"))?;
            stats.skipped_stages += 1;
        }
        self.stage_reports
            .lock()
            .map_err(|_| anyhow::anyhow!("drain-only stage_reports mutex poisoned"))?
            .push(report.clone());
        println!(
            "[model-writer:drain-only] skipped stage={} reason={} item_count={}",
            stage, reason, item_count
        );
        Ok(report)
    }
}

impl Default for DrainOnlyModelWriterBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ModelWriterBackend for DrainOnlyModelWriterBackend {
    fn name(&self) -> &'static str {
        "drain-only"
    }

    fn writes_to_surreal(&self) -> bool {
        false
    }

    fn runs_downstream_pipeline(&self) -> bool {
        false
    }

    async fn init(&self) -> anyhow::Result<ModelWriterStageReport> {
        Ok(ModelWriterStageReport::executed("init", 0))
    }

    async fn cleanup(&self) -> anyhow::Result<ModelWriterStageReport> {
        self.record_skipped(
            "cleanup",
            "drain-only never deletes or rewrites SurrealDB data",
            0,
        )
    }

    async fn write_base_batch(
        &self,
        batch: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriteBatchReport> {
        let progress = {
            let mut stats = self
                .stats
                .lock()
                .map_err(|_| anyhow::anyhow!("drain-only stats mutex poisoned"))?;
            stats.add_batch(batch);
            if stats.batches % 100 == 0 {
                Some((stats.batches, stats.instances, stats.geo_instances))
            } else {
                None
            }
        };

        if let Some((batches, instances, geo_instances)) = progress {
            println!(
                "[model-writer:drain-only] drained batches={} instances={} geo_instances={} elapsed_ms={}",
                batches,
                instances,
                geo_instances,
                self.started.elapsed().as_millis()
            );
        }

        Ok(ModelWriteBatchReport::default())
    }

    async fn persist_mesh_results(
        &self,
        mesh_results: &HashMap<u64, MeshResult>,
        _mesh_aabb_map: &DashMap<String, Aabb>,
        _mesh_pts_map: &DashMap<u64, String>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        self.record_skipped(
            "mesh_persist",
            "drain-only does not persist mesh/aabb/pts data",
            mesh_results.len(),
        )
    }

    async fn persist_inst_relate_aabb(
        &self,
        shape_insts: &ShapeInstancesData,
        _mesh_results: &HashMap<u64, MeshResult>,
        _mesh_aabb_map: &DashMap<String, Aabb>,
        _skip_inst_relate_aabb: bool,
    ) -> anyhow::Result<ModelWriterStageReport> {
        self.record_skipped(
            "inst_relate_aabb",
            "drain-only does not persist inst_relate_aabb rows",
            shape_insts.inst_cnt(),
        )
    }

    async fn reconcile_missing_neg_relations(
        &self,
        _all_refnos: &[RefnoEnum],
        missing_neg_carriers: &[RefnoEnum],
    ) -> anyhow::Result<ModelWriterStageReport> {
        self.record_skipped(
            "missing_neg_reconcile",
            "drain-only does not reconcile SurrealDB relations",
            missing_neg_carriers.len(),
        )
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        self.record_skipped(
            "boolean_bridge",
            "drain-only does not run boolean workers or write SurrealDB",
            request.bool_tasks.len(),
        )?;
        Ok(BooleanBridgeReport {
            total: request.bool_tasks.len(),
            skipped: request.bool_tasks.len(),
            skipped_reason: Some("drain-only does not run boolean workers or write SurrealDB"),
            ..BooleanBridgeReport::default()
        })
    }

    async fn finalize(&self) -> anyhow::Result<ModelWriterFinishReport> {
        let mut stats = self
            .stats
            .lock()
            .map_err(|_| anyhow::anyhow!("drain-only stats mutex poisoned"))?
            .clone();
        stats.elapsed = self.started.elapsed();
        let stage_reports = self
            .stage_reports
            .lock()
            .map_err(|_| anyhow::anyhow!("drain-only stage_reports mutex poisoned"))?
            .clone();
        Ok(ModelWriterFinishReport {
            writer_name: self.name(),
            drain_only_stats: Some(stats),
            stage_reports,
        })
    }
}

pub fn create_model_writer(
    mode: ModelWriterMode,
    mesh_aabb_map: Arc<DashMap<String, Aabb>>,
    missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,
) -> Arc<dyn ModelWriterBackend> {
    match mode {
        ModelWriterMode::Surreal => Arc::new(SurrealModelWriterBackend::new(
            mesh_aabb_map,
            missing_neg_carriers,
        )),
        ModelWriterMode::DrainOnly => Arc::new(DrainOnlyModelWriterBackend::new()),
    }
}

pub async fn run_model_writer_sink(
    receiver: flume::Receiver<ShapeInstancesData>,
    writer: Arc<dyn ModelWriterBackend>,
) -> anyhow::Result<ModelWriterFinishReport> {
    writer.cleanup().await?;
    writer.init().await?;
    while let Ok(batch) = receiver.recv_async().await {
        writer.write_base_batch(&batch).await?;
    }
    writer.finalize().await
}

pub async fn run_drain_only_sink(
    receiver: flume::Receiver<ShapeInstancesData>,
) -> anyhow::Result<DrainOnlyStats> {
    let report =
        run_model_writer_sink(receiver, Arc::new(DrainOnlyModelWriterBackend::new())).await?;
    Ok(report.drain_only_stats.unwrap_or_default())
}

pub fn model_writer_contract_evidence(mode: ModelWriterMode) -> ModelWriterContractEvidence {
    let (backend, writes_to_surreal, runs_downstream_pipeline, drain_only_reason) = match mode {
        ModelWriterMode::Surreal => ("surreal", true, true, None),
        ModelWriterMode::DrainOnly => (
            "drain-only",
            false,
            false,
            Some("drain-only safely skips persistence and destructive stages"),
        ),
    };

    let lifecycle = [
        "init",
        "cleanup",
        "base_batch",
        "mesh_persist",
        "inst_relate_aabb",
        "missing_neg_reconcile",
        "boolean_bridge",
        "finalize",
    ];
    let stages = lifecycle
        .into_iter()
        .map(|stage| match drain_only_reason {
            Some(reason) if !matches!(stage, "init" | "base_batch" | "finalize") => {
                ModelWriterStageReport::skipped(stage, reason, 0)
            }
            _ => ModelWriterStageReport::implemented(stage),
        })
        .collect();

    ModelWriterContractEvidence {
        backend,
        writes_to_surreal,
        runs_downstream_pipeline,
        stages,
    }
}
