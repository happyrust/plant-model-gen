use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use aios_core::options::DbOption;
use aios_core::parsed_data::TubiInfoData;
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;
use serde::Serialize;

use crate::fast_model::gen_model::boolean_task::{BooleanTask, BooleanTaskAccumulator};
use crate::fast_model::gen_model::manifold_bool::run_bool_worker_from_tasks_versioned;
use crate::fast_model::mesh_generate::MeshResult;
use crate::fast_model::pdms_inst::{
    InstRelatePrecomputed, build_inst_relate_aabb_rows, persist_negative_relations_from_artifacts,
    persist_tubi_relations_from_artifacts, save_inst_relate_aabb_rows,
    save_instance_data_with_report_versioned,
};
use crate::fast_model::utils::{save_aabb_to_surreal_checked, save_pts_to_surreal_checked};
use crate::options::{BooleanPipelineMode, ModelWriterMode};

/// 单次模型生成的内存事实源。生成、mesh、关系与 boolean 阶段只在这里交接，
/// writer 负责最终持久化，不允许为了补齐当前 run 的数据再回读模型表。
#[derive(Debug)]
pub struct GenerationArtifacts {
    authoritative_snapshot_id: u64,
    batches: Mutex<BTreeMap<u64, Arc<ShapeInstancesData>>>,
    mesh_results: Mutex<BTreeMap<u64, HashMap<u64, MeshResult>>>,
    boolean_accumulator: Mutex<BooleanTaskAccumulator>,
    missing_neg_carriers: Mutex<BTreeSet<RefnoEnum>>,
    tubi_info: DashMap<String, TubiInfoData>,
    boolean_execution: Mutex<Option<BooleanExecutionArtifact>>,
}

#[derive(Debug, Clone, Serialize)]
struct BooleanExecutionArtifact {
    task_count: usize,
    task_semantic_hash: String,
    success: usize,
    failed: usize,
    skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerationArtifactsSummary {
    pub authoritative_snapshot_id: u64,
    pub batch_count: usize,
    pub mesh_result_count: usize,
    pub missing_neg_carrier_count: usize,
    pub tubi_info_count: usize,
    pub boolean_task_count: usize,
    pub geometry_artifact_hash: String,
    pub semantic_hash: String,
    pub model_semantic_hash: String,
}

impl GenerationArtifacts {
    pub fn new(authoritative_snapshot_id: u64) -> Self {
        Self {
            authoritative_snapshot_id,
            batches: Mutex::new(BTreeMap::new()),
            mesh_results: Mutex::new(BTreeMap::new()),
            boolean_accumulator: Mutex::new(BooleanTaskAccumulator::default()),
            missing_neg_carriers: Mutex::new(BTreeSet::new()),
            tubi_info: DashMap::new(),
            boolean_execution: Mutex::new(None),
        }
    }

    pub fn record_base_batch(
        &self,
        batch_id: u64,
        shape_insts: Arc<ShapeInstancesData>,
    ) -> anyhow::Result<()> {
        let mut batches = self
            .batches
            .lock()
            .map_err(|_| anyhow::anyhow!("batch artifact mutex poisoned"))?;
        anyhow::ensure!(
            !batches.contains_key(&batch_id),
            "duplicate generation batch_id={batch_id}"
        );
        self.boolean_accumulator
            .lock()
            .map_err(|_| anyhow::anyhow!("boolean artifact mutex poisoned"))?
            .merge_batch(&shape_insts);
        batches.insert(batch_id, shape_insts);
        Ok(())
    }

    pub fn record_mesh_results(
        &self,
        batch_id: u64,
        mesh_results: &HashMap<u64, MeshResult>,
    ) -> anyhow::Result<()> {
        let mut batches = self
            .mesh_results
            .lock()
            .map_err(|_| anyhow::anyhow!("mesh artifact mutex poisoned"))?;
        anyhow::ensure!(
            !batches.contains_key(&batch_id),
            "duplicate mesh artifact batch_id={batch_id}"
        );
        batches.insert(batch_id, mesh_results.clone());
        Ok(())
    }

    pub fn record_missing_neg_carriers(
        &self,
        refnos: impl IntoIterator<Item = RefnoEnum>,
    ) -> anyhow::Result<()> {
        self.missing_neg_carriers
            .lock()
            .map_err(|_| anyhow::anyhow!("negative relation artifact mutex poisoned"))?
            .extend(refnos);
        Ok(())
    }

    pub fn take_run_outputs(&self) -> anyhow::Result<(Vec<BooleanTask>, ShapeInstancesData)> {
        let mut accumulator = self
            .boolean_accumulator
            .lock()
            .map_err(|_| anyhow::anyhow!("boolean artifact mutex poisoned"))?;
        let accumulator = std::mem::take(&mut *accumulator);
        let tasks = accumulator.build_tasks();
        Ok((tasks, accumulator.into_merged()))
    }

    pub fn missing_neg_carriers(&self) -> anyhow::Result<Vec<RefnoEnum>> {
        Ok(self
            .missing_neg_carriers
            .lock()
            .map_err(|_| anyhow::anyhow!("negative relation artifact mutex poisoned"))?
            .iter()
            .copied()
            .collect())
    }

    pub fn record_tubi_info(&self, values: &DashMap<String, TubiInfoData>) -> anyhow::Result<()> {
        for entry in values {
            match self.tubi_info.entry(entry.key().clone()) {
                dashmap::mapref::entry::Entry::Occupied(existing) => {
                    anyhow::ensure!(
                        existing.get().to_surreal_json() == entry.value().to_surreal_json(),
                        "conflicting TUBI artifact key={}",
                        entry.key()
                    );
                }
                dashmap::mapref::entry::Entry::Vacant(vacant) => {
                    vacant.insert(entry.value().clone());
                }
            }
        }
        Ok(())
    }

    pub fn tubi_info(&self) -> &DashMap<String, TubiInfoData> {
        &self.tubi_info
    }

    pub fn record_boolean_execution(
        &self,
        task_count: usize,
        task_semantic_hash: String,
        report: &BooleanBridgeReport,
    ) -> anyhow::Result<()> {
        let mut execution = self
            .boolean_execution
            .lock()
            .map_err(|_| anyhow::anyhow!("boolean execution artifact mutex poisoned"))?;
        anyhow::ensure!(
            execution.is_none(),
            "boolean execution artifact already recorded"
        );
        *execution = Some(BooleanExecutionArtifact {
            task_count,
            task_semantic_hash,
            success: report.success,
            failed: report.failed,
            skipped: report.skipped,
        });
        Ok(())
    }

    pub fn summary(&self) -> anyhow::Result<GenerationArtifactsSummary> {
        let batches = self
            .batches
            .lock()
            .map_err(|_| anyhow::anyhow!("batch artifact mutex poisoned"))?;
        let mut merged = BooleanTaskAccumulator::default();
        for shape in batches.values() {
            merged.merge_batch(shape);
        }
        let mut merged = merged.into_merged();
        for refnos in merged.neg_relate_map.values_mut() {
            refnos.sort_unstable();
        }
        for pairs in merged.ngmr_neg_relate_map.values_mut() {
            pairs.sort_unstable();
        }
        for geos in merged.inst_geos_map.values_mut() {
            for geo in &mut geos.insts {
                geo.cata_neg_refnos.sort_unstable();
                geo.cata_neg_refnos.dedup();
            }
            geos.insts
                .sort_unstable_by_key(|geo| crate::generation_read::hash_serializable(geo));
        }
        let neg_relations = merged
            .neg_relate_map
            .iter()
            .map(|(target, carriers)| (*target, carriers.clone()))
            .collect::<BTreeMap<_, _>>();
        let ngmr_relations = merged
            .ngmr_neg_relate_map
            .iter()
            .map(|(target, pairs)| (*target, pairs.clone()))
            .collect::<BTreeMap<_, _>>();
        let merged_shape = canonical_json(serde_json::to_value(merged)?);
        let geometry_artifact_hash = crate::generation_read::hash_serializable(&merged_shape);

        let mesh_results = self
            .mesh_results
            .lock()
            .map_err(|_| anyhow::anyhow!("mesh artifact mutex poisoned"))?;
        let mut mesh_digest = BTreeMap::new();
        for results in mesh_results.values() {
            for (geo_hash, result) in results {
                let mut pts = result.pts_hashes.clone();
                pts.sort_unstable();
                let digest = (result.meshed, result.bad, result.aabb_hash, pts);
                if let Some(existing) = mesh_digest.insert(*geo_hash, digest.clone()) {
                    anyhow::ensure!(
                        existing == digest,
                        "conflicting mesh artifact geo_hash={geo_hash}"
                    );
                }
            }
        }
        let mesh_result_count = mesh_digest.len();

        let missing_neg_carriers = self.missing_neg_carriers()?;
        let mut tubi_info = self
            .tubi_info
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().to_surreal_json()))
            .collect::<Vec<_>>();
        tubi_info.sort_by(|left, right| left.0.cmp(&right.0));
        let boolean_execution = self
            .boolean_execution
            .lock()
            .map_err(|_| anyhow::anyhow!("boolean execution artifact mutex poisoned"))?
            .clone();
        let model_semantic_hash = crate::generation_read::hash_serializable(&(
            &merged_shape,
            &neg_relations,
            &ngmr_relations,
            &mesh_digest,
            &tubi_info,
            &boolean_execution,
        ));
        let semantic_hash = crate::generation_read::hash_serializable(&(
            self.authoritative_snapshot_id,
            &model_semantic_hash,
            &missing_neg_carriers,
        ));
        Ok(GenerationArtifactsSummary {
            authoritative_snapshot_id: self.authoritative_snapshot_id,
            batch_count: batches.len(),
            mesh_result_count,
            missing_neg_carrier_count: missing_neg_carriers.len(),
            tubi_info_count: tubi_info.len(),
            boolean_task_count: boolean_execution
                .as_ref()
                .map(|execution| execution.task_count)
                .unwrap_or_default(),
            geometry_artifact_hash,
            semantic_hash,
            model_semantic_hash,
        })
    }
}

fn canonical_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect::<BTreeMap<_, _>>();
            serde_json::to_value(sorted).expect("canonical JSON object serialization")
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical_json).collect())
        }
        other => other,
    }
}

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
    pub fn executed(stage: &'static str, item_count: usize) -> Self {
        Self {
            stage,
            status: ModelWriterStageStatus::Executed,
            item_count,
            skipped_reason: None,
        }
    }

    pub fn implemented(stage: &'static str) -> Self {
        Self {
            stage,
            status: ModelWriterStageStatus::Implemented,
            item_count: 0,
            skipped_reason: None,
        }
    }

    pub fn skipped(stage: &'static str, reason: &'static str, item_count: usize) -> Self {
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
    /// Phase 1 raw tables intentionally NOT written by this backend.
    /// Empty for surreal/drain-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_gap_tables: Vec<&'static str>,
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
    pub scope_refnos: Vec<RefnoEnum>,
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

    /// 专门的 pe_transform 持久化阶段（按需生成方案）。
    ///
    /// 模型生成阶段已算出每个实例的 world_transform；这里把它作为一个**独立的
    /// writer 阶段**落库 `pe_transform`，使覆盖成为生成的副产品——只覆盖本次
    /// 生成的实例、零重算、无整库 BFS。非生成路径（导出 / transform API）继续
    /// 按需惰性获取。默认 no-op，仅写库后端实现。
    async fn persist_pe_transform(
        &self,
        shape_insts: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriterStageReport> {
        let _ = shape_insts;
        Ok(ModelWriterStageReport::skipped(
            "pe_transform",
            "backend default no-op",
            0,
        ))
    }

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
        artifacts: &ShapeInstancesData,
        tubi_info: &DashMap<String, TubiInfoData>,
    ) -> anyhow::Result<ModelWriterStageReport>;

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport>;

    /// spec 006 T302：收尾安全网。sink 全部完成后调用一次，把全局
    /// aabb/pts map 全量 `INSERT IGNORE` 补写，兜住增量写覆盖不到的
    /// 跨运行状态漂移（mesh 文件在、DB 行不在）。默认 no-op。
    async fn finalize_mesh_entities(
        &self,
        mesh_aabb_map: &DashMap<String, Aabb>,
        mesh_pts_map: &DashMap<u64, String>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        let _ = (mesh_aabb_map, mesh_pts_map);
        Ok(ModelWriterStageReport::skipped(
            "final_sweep",
            "backend default no-op",
            0,
        ))
    }

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
    inst_relate_precomputed: Arc<InstRelatePrecomputed>,
}

impl SurrealModelWriterBackend {
    pub fn new(
        mesh_aabb_map: Arc<DashMap<String, Aabb>>,
        missing_neg_carriers: Arc<Mutex<HashSet<RefnoEnum>>>,
        inst_relate_precomputed: Arc<InstRelatePrecomputed>,
    ) -> Self {
        Self {
            mesh_aabb_map,
            missing_neg_carriers,
            inst_relate_precomputed,
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
        let save_report = save_instance_data_with_report_versioned(
            batch,
            false,
            &HashMap::new(),
            &self.mesh_aabb_map,
            false,
            &self.inst_relate_precomputed,
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

    async fn persist_pe_transform(
        &self,
        shape_insts: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriterStageReport> {
        use aios_core::rs_surreal::pe_transform::PeTransformEntry;

        let mut entries: Vec<PeTransformEntry> =
            Vec::with_capacity(shape_insts.inst_info_map.len());
        for (refno, info) in &shape_insts.inst_info_map {
            let world = info.world_transform;
            if world.translation.is_nan() || world.rotation.is_nan() || world.scale.is_nan() {
                continue;
            }
            entries.push(PeTransformEntry {
                refno: *refno,
                local: None,
                world: Some(world),
            });
        }

        if entries.is_empty() {
            return Ok(ModelWriterStageReport::skipped(
                "pe_transform",
                "no instance world transforms in batch",
                0,
            ));
        }

        let count = entries.len();
        let db_option = crate::options::get_db_option_ext();
        crate::pe_transform_store::save_entries_with_backend(&db_option, &entries).await?;
        Ok(ModelWriterStageReport::executed("pe_transform", count))
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

        // spec 006 T301：每批只写本批 delta（由 mesh_results 的 aabb_hash/pts_hashes 还原），
        // 不再全量重写整个全局 map（O(N²) 回归根因，基线每批固定 ~5.5s）。
        // 历史行与跨运行状态漂移由收尾 finalize_mesh_entities 全量补写兜底。
        let delta_aabb: DashMap<String, Aabb> = DashMap::new();
        let delta_pts: DashMap<u64, String> = DashMap::new();
        for mesh_result in mesh_results.values() {
            if let Some(aabb_hash) = mesh_result.aabb_hash {
                let key = aabb_hash.to_string();
                if let Some(value) = mesh_aabb_map.get(&key) {
                    delta_aabb.insert(key, *value.value());
                }
            }
            for pts_hash in &mesh_result.pts_hashes {
                if let Some(value) = mesh_pts_map.get(pts_hash) {
                    delta_pts.insert(*pts_hash, value.value().clone());
                }
            }
        }

        // Preserve existing ordering: persist aabb/pts entities before inst_geo references them.
        save_pts_to_surreal_checked(&delta_pts).await?;
        save_aabb_to_surreal_checked(&delta_aabb).await?;

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

        aios_core::project_primary_db()
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

    /// spec 006 T302：全量补写 aabb/vec3（INSERT IGNORE 幂等，一次性 ~秒级）。
    async fn finalize_mesh_entities(
        &self,
        mesh_aabb_map: &DashMap<String, Aabb>,
        mesh_pts_map: &DashMap<u64, String>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        if crate::fast_model::gen_model::mesh_state::use_file_mesh_state() {
            crate::fast_model::gen_model::mesh_state::flush_aabb_cache();
            return Ok(ModelWriterStageReport::skipped(
                "final_sweep",
                "file mesh state active; flushed aabb cache",
                0,
            ));
        }

        save_pts_to_surreal_checked(mesh_pts_map).await?;
        save_aabb_to_surreal_checked(mesh_aabb_map).await?;
        Ok(ModelWriterStageReport::executed(
            "final_sweep",
            mesh_aabb_map.len() + mesh_pts_map.len(),
        ))
    }

    async fn reconcile_missing_neg_relations(
        &self,
        artifacts: &ShapeInstancesData,
        tubi_info: &DashMap<String, TubiInfoData>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        if artifacts.neg_relate_map.is_empty()
            && artifacts.ngmr_neg_relate_map.is_empty()
            && artifacts.inst_tubi_map.is_empty()
            && tubi_info.is_empty()
        {
            return Ok(ModelWriterStageReport::skipped(
                "run_relation_artifacts",
                "no negative or tubing relation artifacts",
                0,
            ));
        }

        // tubi 版本存储的唯一真相源是 persist_tubi_relations_from_artifacts 写入的
        // `tubi_relate`（按分支 refno 键、可随 regen 版本化清理、导出/历史读取都只认它）。
        // `tubi_info` 表从不被任何读取方消费，且按内容哈希键、无法按分支范围清理，
        // 保留写入只会在 versioned 库里持续膨胀，故不再写入（遗留行由 pre_cleanup 兜底清空）。
        let submitted = persist_negative_relations_from_artifacts(artifacts).await?
            + persist_tubi_relations_from_artifacts(artifacts).await?;
        Ok(ModelWriterStageReport::executed(
            "run_relation_artifacts",
            submitted,
        ))
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        match request.mode {
            BooleanPipelineMode::DbLegacy => anyhow::bail!(
                "DbLegacy boolean pipeline 不属于版本化双后端正式路径；请使用 memory_tasks"
            ),
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

                if request.enable_db_backfill {
                    anyhow::bail!(
                        "enable_db_backfill 与版本化双后端正式路径不兼容；boolean task 必须完全来自 GenerationArtifacts"
                    );
                }

                let report = run_bool_worker_from_tasks_versioned(
                    request.bool_tasks,
                    request.db_option,
                    None,
                )
                .await?;
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

    async fn persist_pe_transform(
        &self,
        shape_insts: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriterStageReport> {
        self.record_skipped(
            "pe_transform",
            "drain-only does not persist pe_transform",
            shape_insts.inst_info_map.len(),
        )
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
        artifacts: &ShapeInstancesData,
        tubi_info: &DashMap<String, TubiInfoData>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        self.record_skipped(
            "run_relation_artifacts",
            "drain-only does not persist relation artifacts",
            artifacts.neg_relate_map.len()
                + artifacts.ngmr_neg_relate_map.len()
                + artifacts.inst_tubi_map.len()
                + tubi_info.len(),
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
    inst_relate_precomputed: Option<Arc<InstRelatePrecomputed>>,
) -> Arc<dyn ModelWriterBackend> {
    match mode {
        ModelWriterMode::Surreal => Arc::new(SurrealModelWriterBackend::new(
            mesh_aabb_map,
            missing_neg_carriers,
            inst_relate_precomputed
                .expect("Surreal model writer requires versioned precomputed metadata"),
        )),
        ModelWriterMode::DrainOnly => Arc::new(DrainOnlyModelWriterBackend::new()),
    }
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
        "pe_transform",
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
        known_gap_tables: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_semantic_hash_is_snapshot_bound_and_deterministic() {
        fn build(snapshot_id: u64) -> GenerationArtifactsSummary {
            let artifacts = GenerationArtifacts::new(snapshot_id);
            artifacts
                .record_base_batch(1, Arc::new(ShapeInstancesData::default()))
                .expect("record batch");
            artifacts
                .record_mesh_results(1, &HashMap::new())
                .expect("record mesh");
            artifacts.summary().expect("summary")
        }

        let first = build(42);
        let repeated = build(42);
        let another_snapshot = build(43);
        assert_eq!(first.semantic_hash, repeated.semantic_hash);
        assert_ne!(first.semantic_hash, another_snapshot.semantic_hash);
        assert_eq!(
            first.model_semantic_hash,
            another_snapshot.model_semantic_hash
        );
    }

    #[test]
    fn duplicate_artifact_batch_fails_closed() {
        let artifacts = GenerationArtifacts::new(42);
        artifacts
            .record_base_batch(1, Arc::new(ShapeInstancesData::default()))
            .expect("first batch");
        assert!(
            artifacts
                .record_base_batch(1, Arc::new(ShapeInstancesData::default()))
                .is_err()
        );
    }

    #[test]
    fn semantic_hash_ignores_batch_ids_and_arrival_order() {
        fn neg_batch(target: RefnoEnum, negative: RefnoEnum) -> ShapeInstancesData {
            let mut batch = ShapeInstancesData::default();
            batch.neg_relate_map.insert(target, vec![negative]);
            batch
        }

        let target = RefnoEnum::from("1/1");
        let first_neg = RefnoEnum::from("1/2");
        let second_neg = RefnoEnum::from("1/3");

        let left = GenerationArtifacts::new(42);
        left.record_base_batch(1, Arc::new(neg_batch(target, first_neg)))
            .expect("left first");
        left.record_base_batch(2, Arc::new(neg_batch(target, second_neg)))
            .expect("left second");

        let right = GenerationArtifacts::new(42);
        right
            .record_base_batch(90, Arc::new(neg_batch(target, second_neg)))
            .expect("right second");
        right
            .record_base_batch(80, Arc::new(neg_batch(target, first_neg)))
            .expect("right first");

        assert_eq!(
            left.summary().expect("left summary").semantic_hash,
            right.summary().expect("right summary").semantic_hash
        );
    }

    #[test]
    fn geometry_artifact_hash_is_independent_of_downstream_execution() {
        let batch = Arc::new(ShapeInstancesData::default());
        let producer_only = GenerationArtifacts::new(42);
        producer_only
            .record_base_batch(1, Arc::clone(&batch))
            .expect("record producer batch");

        let downstream = GenerationArtifacts::new(42);
        downstream
            .record_base_batch(9, batch)
            .expect("record downstream batch");
        downstream
            .record_boolean_execution(
                1,
                "downstream-task".to_string(),
                &BooleanBridgeReport {
                    total: 1,
                    success: 1,
                    ..BooleanBridgeReport::default()
                },
            )
            .expect("record downstream execution");

        let producer_summary = producer_only.summary().expect("producer summary");
        let downstream_summary = downstream.summary().expect("downstream summary");
        assert_eq!(
            producer_summary.geometry_artifact_hash,
            downstream_summary.geometry_artifact_hash
        );
        assert_ne!(
            producer_summary.model_semantic_hash,
            downstream_summary.model_semantic_hash
        );
    }
}
