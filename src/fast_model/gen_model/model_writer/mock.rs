#![cfg(feature = "model-writer-mock")]

use std::sync::Mutex;

use aios_core::RefnoEnum;
use async_trait::async_trait;

use super::{
    BaseInstanceBatch, BooleanBridgeReport, BooleanBridgeRequest, CleanupRequest, FinalizeRequest,
    FinalizeSummary, InstRelateAabbBatch, MeshResultBatch, ModelWriterBackend, ModelWriterContext,
    ReconcileRequest, WriteBaseReport,
};

/// 记录每次 trait 方法调用的 fixture backend，仅用于契约验证。
///
/// **不要**用于 release 构建：本类型仅在 feature `model-writer-mock` 下编译。
#[derive(Debug, Default)]
pub struct RecordingBackend {
    calls: Mutex<Vec<String>>,
    pub injected_reconcile_inserted: Mutex<usize>,
    pub injected_missing_neg: Mutex<Vec<RefnoEnum>>,
}

impl RecordingBackend {
    pub fn snapshot(&self) -> Vec<String> {
        self.calls.lock().expect("recording lock").clone()
    }

    fn record(&self, line: impl Into<String>) {
        self.calls.lock().expect("recording lock").push(line.into());
    }
}

#[async_trait]
impl ModelWriterBackend for RecordingBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()> {
        self.record(format!(
            "init:project={},use_surrealdb={},defer_db_write={},mode={}",
            context.project_name,
            context.use_surrealdb,
            context.defer_db_write,
            context.mode.as_str()
        ));
        Ok(())
    }

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()> {
        self.record(format!("cleanup:seed_refnos={}", request.seed_refnos.len()));
        Ok(())
    }

    async fn write_base_batch(
        &self,
        batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<WriteBaseReport> {
        self.record(format!(
            "write_base_batch:batch={},inst_info={},inst_tubi={},replace_exist={},write_inst_relate_aabb={}",
            batch.batch_id,
            batch.shape_insts.inst_info_map.len(),
            batch.shape_insts.inst_tubi_map.len(),
            batch.replace_exist,
            batch.write_inst_relate_aabb
        ));
        // v3 Phase F.1: injected carriers are no longer returned per-batch.
        // The verify binary now drains them via `take_missing_neg_carriers`.
        let missing_neg_count = self
            .injected_missing_neg
            .lock()
            .expect("recording lock")
            .len();
        Ok(WriteBaseReport {
            batch_id: batch.batch_id,
            missing_neg_count,
        })
    }

    async fn take_missing_neg_carriers(&self) -> anyhow::Result<Vec<RefnoEnum>> {
        self.record("take_missing_neg_carriers");
        let mut guard = self
            .injected_missing_neg
            .lock()
            .expect("recording lock");
        let drained = std::mem::take(&mut *guard);
        Ok(drained)
    }

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        self.record(format!(
            "persist_mesh_results:batch={},mesh_results={},file_mesh_state={}",
            batch.batch_id,
            batch.mesh_results.len(),
            batch.file_mesh_state
        ));
        Ok(())
    }

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()> {
        self.record(format!(
            "write_inst_relate_aabb:batch={},mesh_results={},aabb_keys={}",
            batch.batch_id,
            batch.mesh_results.len(),
            batch.mesh_aabb_map.len()
        ));
        Ok(())
    }

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize> {
        self.record(format!(
            "reconcile_missing_neg:all={},candidates={}",
            request.all_refnos.len(),
            request.candidate_carriers.len()
        ));
        Ok(*self
            .injected_reconcile_inserted
            .lock()
            .expect("recording lock"))
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        self.record(format!(
            "run_boolean_bridge:mode={:?},bool_tasks={}",
            request.mode,
            request.bool_tasks.len()
        ));
        Ok(BooleanBridgeReport::skipped(
            "mock",
            request.bool_tasks.len(),
            "mock backend",
        ))
    }

    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary> {
        self.record(format!(
            "finalize:total_batches={},completed_batches={},mesh_cache_hits={},mesh_new_generated={},missing_neg_candidates={}",
            request.total_batches,
            request.completed_batches,
            request.mesh_cache_hits,
            request.mesh_new_generated,
            request.missing_neg_candidates
        ));
        Ok(FinalizeSummary {
            backend: self.name(),
            total_batches: request.total_batches,
            completed_batches: request.completed_batches,
        })
    }
}
