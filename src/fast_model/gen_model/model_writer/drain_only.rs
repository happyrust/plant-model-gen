//! DrainOnly backend = baseline mode.
//!
//! Architecture invariant: when `ModelWriterMode::DrainOnly` is selected, the
//! orchestrator takes the fast path (`run_drain_only_sink`) and only routes
//! `init` + `finalize` through this trait impl. The middle six methods —
//! `cleanup` / `write_base_batch` / `persist_mesh_results` /
//! `write_inst_relate_aabb` / `reconcile_missing_neg` / `run_boolean_bridge` —
//! exist as defensive scaffolding for the mock/verify binary and any future
//! callsite that wants to drive the trait directly. They are intentionally
//! **not** consumed by the production pipeline so that DrainOnly stays a clean
//! "skip all persistence" baseline for measuring write-backend timing across
//! Surreal / Parquet / DuckLake. Do not refactor the orchestrator to route
//! these six methods without re-validating that the baseline IO-skip semantic
//! is preserved (see `orchestrator.rs` `DrainOnly` fast-path comment).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Lock-free DrainOnly stats accumulator for the trait-routed path
/// (mock/verify binary, NOT the production fast-path).
///
/// v4 §2.3: replaces the previous `Mutex<DrainOnlyStats>` with independent
/// `AtomicU64` counters. The architectural invariant is unchanged
/// (production orchestrator skips the middle 6 trait methods entirely;
/// `run_drain_only_sink` still owns a stack-allocated `DrainOnlyStats`
/// with no atomic overhead). This struct only matters when something
/// _does_ drive the trait — primarily the verify binary asserting that
/// `add_batch` counters fire as expected.
#[derive(Debug, Default)]
struct DrainOnlyAtomicStats {
    batches: AtomicU64,
    instances: AtomicU64,
    inst_info: AtomicU64,
    inst_tubi: AtomicU64,
    geo_keys: AtomicU64,
    geo_instances: AtomicU64,
    neg_relations: AtomicU64,
    ngmr_relations: AtomicU64,
    mesh_result_batches: AtomicU64,
    mesh_results: AtomicU64,
    inst_relate_aabb_batches: AtomicU64,
}

impl DrainOnlyAtomicStats {
    fn add_batch(&self, batch: &ShapeInstancesData) {
        self.batches.fetch_add(1, Ordering::Relaxed);
        self.instances
            .fetch_add(batch.inst_cnt() as u64, Ordering::Relaxed);
        self.inst_info
            .fetch_add(batch.inst_info_map.len() as u64, Ordering::Relaxed);
        self.inst_tubi
            .fetch_add(batch.inst_tubi_map.len() as u64, Ordering::Relaxed);
        self.geo_keys
            .fetch_add(batch.inst_geos_map.len() as u64, Ordering::Relaxed);
        let geo_instances: usize = batch
            .inst_geos_map
            .values()
            .map(|geos| geos.insts.len())
            .sum();
        self.geo_instances
            .fetch_add(geo_instances as u64, Ordering::Relaxed);
        let neg: usize = batch.neg_relate_map.values().map(Vec::len).sum();
        self.neg_relations
            .fetch_add(neg as u64, Ordering::Relaxed);
        let ngmr: usize = batch.ngmr_neg_relate_map.values().map(Vec::len).sum();
        self.ngmr_relations
            .fetch_add(ngmr as u64, Ordering::Relaxed);
    }

    /// Atomic snapshot into a regular `DrainOnlyStats` so we can reuse the
    /// canonical `print_summary` format.
    fn snapshot(&self, elapsed: Duration) -> DrainOnlyStats {
        DrainOnlyStats {
            batches: self.batches.load(Ordering::Relaxed) as usize,
            instances: self.instances.load(Ordering::Relaxed) as usize,
            inst_info: self.inst_info.load(Ordering::Relaxed) as usize,
            inst_tubi: self.inst_tubi.load(Ordering::Relaxed) as usize,
            geo_keys: self.geo_keys.load(Ordering::Relaxed) as usize,
            geo_instances: self.geo_instances.load(Ordering::Relaxed) as usize,
            neg_relations: self.neg_relations.load(Ordering::Relaxed) as usize,
            ngmr_relations: self.ngmr_relations.load(Ordering::Relaxed) as usize,
            mesh_result_batches: self.mesh_result_batches.load(Ordering::Relaxed) as usize,
            mesh_results: self.mesh_results.load(Ordering::Relaxed) as usize,
            inst_relate_aabb_batches: self
                .inst_relate_aabb_batches
                .load(Ordering::Relaxed) as usize,
            elapsed,
        }
    }
}

#[derive(Debug, Default)]
pub struct DrainOnlyModelWriterBackend {
    stats: DrainOnlyAtomicStats,
    started: OnceLock<Instant>,
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
        // Second init keeps the original Instant (OnceLock is set-once); this
        // preserves the v3 verify-binary "second init is safe" assertion.
        let _ = self.started.set(Instant::now());
        Ok(())
    }

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()> {
        // architectural-invariant: not called by orchestrator main pipeline
        // when ModelWriterMode::DrainOnly is selected (baseline fast path).
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
        // architectural-invariant: not called by orchestrator main pipeline
        // when ModelWriterMode::DrainOnly is selected (baseline fast path).
        self.stats.add_batch(batch.shape_insts);
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
        })
    }

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        // architectural-invariant: not called by orchestrator main pipeline
        // when ModelWriterMode::DrainOnly is selected (baseline fast path).
        let count = batch.mesh_results.len();
        self.stats
            .mesh_result_batches
            .fetch_add(1, Ordering::Relaxed);
        self.stats
            .mesh_results
            .fetch_add(count as u64, Ordering::Relaxed);
        println!(
            "[model-writer:drain-only] stage=mesh_results batch={} mesh_results={}",
            batch.batch_id, count
        );
        Ok(())
    }

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()> {
        // architectural-invariant: not called by orchestrator main pipeline
        // when ModelWriterMode::DrainOnly is selected (baseline fast path).
        self.stats
            .inst_relate_aabb_batches
            .fetch_add(1, Ordering::Relaxed);
        println!(
            "[model-writer:drain-only] stage=inst_relate_aabb batch={} mesh_results={} aabb_keys={}",
            batch.batch_id,
            batch.mesh_results.len(),
            batch.mesh_aabb_map.len()
        );
        Ok(())
    }

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize> {
        // architectural-invariant: not called by orchestrator main pipeline
        // when ModelWriterMode::DrainOnly is selected (baseline fast path).
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
        // architectural-invariant: not called by orchestrator main pipeline
        // when ModelWriterMode::DrainOnly is selected (baseline fast path).
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
            .get()
            .map(|t| t.elapsed())
            .unwrap_or_default();
        let snapshot = self.stats.snapshot(elapsed);
        snapshot.print_summary();
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
