use std::sync::Mutex;
use std::time::{Duration, Instant};

use aios_core::geometry::ShapeInstancesData;
use async_trait::async_trait;

use super::{
    BaseInstanceBatch, BooleanBridgeReport, BooleanBridgeRequest, CleanupRequest, FinalizeRequest,
    FinalizeSummary, InstRelateAabbBatch, MeshResultBatch, ModelWriterBackend, ModelWriterContext,
    ReconcileRequest, WriteBaseReport,
};

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
    pub mesh_result_batches: usize,
    pub mesh_results: usize,
    pub inst_relate_aabb_batches: usize,
    pub elapsed: Duration,
}

impl DrainOnlyStats {
    pub(crate) fn add_batch(&mut self, batch: &ShapeInstancesData) {
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
            "[model-writer:drain-only] summary: batches={} instances={} inst_info={} inst_tubi={} geo_keys={} geo_instances={} neg_relations={} ngmr_relations={} mesh_batches={} mesh_results={} inst_relate_aabb_batches={} elapsed_ms={}",
            self.batches,
            self.instances,
            self.inst_info,
            self.inst_tubi,
            self.geo_keys,
            self.geo_instances,
            self.neg_relations,
            self.ngmr_relations,
            self.mesh_result_batches,
            self.mesh_results,
            self.inst_relate_aabb_batches,
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

#[derive(Debug)]
pub struct DrainOnlyModelWriterBackend {
    stats: Mutex<DrainOnlyStats>,
    started: Mutex<Option<Instant>>,
}

impl Default for DrainOnlyModelWriterBackend {
    fn default() -> Self {
        Self {
            stats: Mutex::new(DrainOnlyStats::default()),
            started: Mutex::new(None),
        }
    }
}

#[async_trait]
impl ModelWriterBackend for DrainOnlyModelWriterBackend {
    fn name(&self) -> &'static str {
        "drain-only"
    }

    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()> {
        println!(
            "[model-writer:drain-only] stage=init project={} use_surrealdb={} defer_db_write={} mode={}",
            context.project_name,
            context.use_surrealdb,
            context.defer_db_write,
            context.mode.as_str()
        );
        *self.started.lock().expect("drain-only started lock") = Some(Instant::now());
        Ok(())
    }

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()> {
        // Drain-only 是非持久化压测 sink，禁止删除任何现有数据；此处仅记录意图。
        println!(
            "[model-writer:drain-only] stage=cleanup noop seed_refnos={}",
            request.seed_refnos.len()
        );
        Ok(())
    }

    async fn write_base_batch(
        &self,
        batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<WriteBaseReport> {
        {
            let mut stats = self.stats.lock().expect("drain-only stats lock");
            stats.add_batch(batch.shape_insts);
        }
        println!(
            "[model-writer:drain-only] stage=base batch={} inst_info={} inst_tubi={} geo_keys={}",
            batch.batch_id,
            batch.shape_insts.inst_info_map.len(),
            batch.shape_insts.inst_tubi_map.len(),
            batch.shape_insts.inst_geos_map.len()
        );
        Ok(WriteBaseReport {
            batch_id: batch.batch_id,
            missing_neg_count: 0,
            missing_neg_carriers: Vec::new(),
        })
    }

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        let count = batch.mesh_results.len();
        {
            let mut stats = self.stats.lock().expect("drain-only stats lock");
            stats.mesh_result_batches += 1;
            stats.mesh_results += count;
        }
        println!(
            "[model-writer:drain-only] stage=mesh_results batch={} mesh_results={}",
            batch.batch_id, count
        );
        Ok(())
    }

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()> {
        {
            let mut stats = self.stats.lock().expect("drain-only stats lock");
            stats.inst_relate_aabb_batches += 1;
        }
        println!(
            "[model-writer:drain-only] stage=inst_relate_aabb batch={} mesh_results={} aabb_keys={}",
            batch.batch_id,
            batch.mesh_results.len(),
            batch.mesh_aabb_map.len()
        );
        Ok(())
    }

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize> {
        println!(
            "[model-writer:drain-only] stage=reconcile_missing_neg noop all_refnos={} candidate_carriers={}",
            request.all_refnos.len(),
            request.candidate_carriers.len()
        );
        Ok(0)
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        println!(
            "[model-writer:drain-only] stage=boolean_bridge noop mode={:?} bool_tasks={}",
            request.mode,
            request.bool_tasks.len()
        );
        Ok(BooleanBridgeReport::skipped(
            "drain_only",
            request.bool_tasks.len(),
            "drain-only is non-persistent",
        ))
    }

    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary> {
        let elapsed = self
            .started
            .lock()
            .expect("drain-only started lock")
            .map(|t| t.elapsed())
            .unwrap_or_default();
        {
            let mut stats = self.stats.lock().expect("drain-only stats lock");
            stats.elapsed = elapsed;
            stats.print_summary();
        }
        println!(
            "[model-writer:drain-only] stage=finalize total_batches={} completed_batches={} mesh_cache_hits={} mesh_new_generated={} missing_neg_candidates={}",
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
