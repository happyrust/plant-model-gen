//! Compare mode dual-write wrapper around two `ModelWriterBackend` impls.
//!
//! v3 Phase C：把 "candidate backend" 作为旁路写入器挂在主 backend 上，
//! 让 orchestrator 在不修改调用点的情况下完成双写。语义遵循 mission
//! `03-writer-architecture.md` §Error handling：**fail fast, no silent
//! fallback** —— candidate 写失败立即 bail，绝不静默继续。
//!
//! 典型用法：`model_writer_mode = "surreal"` + `model_writer_compare_with = "parquet"`，
//! 工厂会构造 `CompareModelWriterBackend::new(SurrealBackend, ParquetBackend)`。
//!
//! 调用顺序约定：
//!
//! 1. 先 primary，再 candidate。primary 失败 → 立即 bail（不动 candidate）；
//!    primary 成功 → 调 candidate；candidate 失败 → 立即 bail（已写入 primary
//!    无回滚，运维 / SQL parity 在 v3 Phase E 做事后比对）。
//! 2. 报告（`WriteBaseReport`、`reconcile_missing_neg.inserted`、
//!    `BooleanBridgeReport`、`FinalizeSummary`）以 **primary 为准** 返回；
//!    candidate 的差异在 `[model-writer:compare]` 日志里逐条暴露，
//!    供 Phase E `validate-compare` CLI 抓取。

use async_trait::async_trait;

use aios_core::RefnoEnum;

use super::{
    BaseInstanceBatch, BooleanBridgeReport, BooleanBridgeRequest, CleanupRequest, FinalizeRequest,
    FinalizeSummary, InstRelateAabbBatch, MeshResultBatch, ModelWriterBackend, ModelWriterContext,
    ReconcileRequest, WriteBaseReport,
};

use std::sync::Arc;

/// Dual-write wrapper. See module doc for ordering + failure semantics.
pub struct CompareModelWriterBackend {
    primary: Arc<dyn ModelWriterBackend>,
    candidate: Arc<dyn ModelWriterBackend>,
}

impl std::fmt::Debug for CompareModelWriterBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompareModelWriterBackend")
            .field("primary", &self.primary.name())
            .field("candidate", &self.candidate.name())
            .finish()
    }
}

impl CompareModelWriterBackend {
    pub fn new(
        primary: Arc<dyn ModelWriterBackend>,
        candidate: Arc<dyn ModelWriterBackend>,
    ) -> Self {
        println!(
            "[model-writer:compare] init wrapper primary={} candidate={} fail_fast=true",
            primary.name(),
            candidate.name()
        );
        Self { primary, candidate }
    }

    fn log(&self, stage: &str, key: &str, value: &str) {
        println!(
            "[model-writer:compare] stage={} primary={} candidate={} {}={}",
            stage,
            self.primary.name(),
            self.candidate.name(),
            key,
            value
        );
    }
}

#[async_trait]
impl ModelWriterBackend for CompareModelWriterBackend {
    fn name(&self) -> &'static str {
        // Static label; primary/candidate identities surface in every log line.
        "compare"
    }

    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()> {
        self.log("init", "step", "primary");
        self.primary.init(context).await.map_err(|e| {
            anyhow::anyhow!(
                "compare primary({}) init failed: {}",
                self.primary.name(),
                e
            )
        })?;
        self.log("init", "step", "candidate");
        self.candidate.init(context).await.map_err(|e| {
            anyhow::anyhow!(
                "compare candidate({}) init failed (fail-fast, no rollback on primary): {}",
                self.candidate.name(),
                e
            )
        })?;
        Ok(())
    }

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()> {
        self.log(
            "cleanup",
            "seed_refnos",
            &request.seed_refnos.len().to_string(),
        );
        let seeds = request.seed_refnos;
        self.primary
            .cleanup(CleanupRequest { seed_refnos: seeds })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare primary({}) cleanup failed: {}",
                    self.primary.name(),
                    e
                )
            })?;
        self.candidate
            .cleanup(CleanupRequest { seed_refnos: seeds })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare candidate({}) cleanup failed: {}",
                    self.candidate.name(),
                    e
                )
            })?;
        Ok(())
    }

    async fn write_base_batch(
        &self,
        batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<WriteBaseReport> {
        let batch_id = batch.batch_id;
        let primary_batch = BaseInstanceBatch {
            batch_id,
            shape_insts: batch.shape_insts,
            mesh_aabb_map: batch.mesh_aabb_map,
            replace_exist: batch.replace_exist,
            write_inst_relate_aabb: batch.write_inst_relate_aabb,
        };
        let candidate_batch = BaseInstanceBatch {
            batch_id,
            shape_insts: batch.shape_insts,
            mesh_aabb_map: batch.mesh_aabb_map,
            replace_exist: batch.replace_exist,
            write_inst_relate_aabb: batch.write_inst_relate_aabb,
        };

        let primary = self
            .primary
            .write_base_batch(primary_batch)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare primary({}) write_base_batch({}) failed: {}",
                    self.primary.name(),
                    batch_id,
                    e
                )
            })?;
        let candidate = self
            .candidate
            .write_base_batch(candidate_batch)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare candidate({}) write_base_batch({}) failed: {}",
                    self.candidate.name(),
                    batch_id,
                    e
                )
            })?;

        if primary.missing_neg_count != candidate.missing_neg_count {
            self.log(
                "base_diff",
                "missing_neg_count",
                &format!(
                    "primary={} candidate={} batch={}",
                    primary.missing_neg_count, candidate.missing_neg_count, batch_id
                ),
            );
        } else {
            self.log("base", "batch_done", &batch_id.to_string());
        }
        Ok(primary)
    }

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        let batch_id = batch.batch_id;
        let mesh_count = batch.mesh_results.len();
        let primary_batch = MeshResultBatch {
            batch_id,
            mesh_results: batch.mesh_results,
            mesh_aabb_map: batch.mesh_aabb_map,
            mesh_pts_map: batch.mesh_pts_map,
            file_mesh_state: batch.file_mesh_state,
        };
        let candidate_batch = MeshResultBatch {
            batch_id,
            mesh_results: batch.mesh_results,
            mesh_aabb_map: batch.mesh_aabb_map,
            mesh_pts_map: batch.mesh_pts_map,
            file_mesh_state: batch.file_mesh_state,
        };

        self.primary
            .persist_mesh_results(primary_batch)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare primary({}) persist_mesh_results({}) failed: {}",
                    self.primary.name(),
                    batch_id,
                    e
                )
            })?;
        self.candidate
            .persist_mesh_results(candidate_batch)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare candidate({}) persist_mesh_results({}) failed: {}",
                    self.candidate.name(),
                    batch_id,
                    e
                )
            })?;
        self.log(
            "mesh_results",
            "batch_done",
            &format!("batch={} mesh_results={}", batch_id, mesh_count),
        );
        Ok(())
    }

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()> {
        let batch_id = batch.batch_id;
        let primary_batch = InstRelateAabbBatch {
            batch_id,
            shape_insts: batch.shape_insts,
            mesh_results: batch.mesh_results,
            mesh_aabb_map: batch.mesh_aabb_map,
        };
        let candidate_batch = InstRelateAabbBatch {
            batch_id,
            shape_insts: batch.shape_insts,
            mesh_results: batch.mesh_results,
            mesh_aabb_map: batch.mesh_aabb_map,
        };
        self.primary
            .write_inst_relate_aabb(primary_batch)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare primary({}) write_inst_relate_aabb({}) failed: {}",
                    self.primary.name(),
                    batch_id,
                    e
                )
            })?;
        self.candidate
            .write_inst_relate_aabb(candidate_batch)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare candidate({}) write_inst_relate_aabb({}) failed: {}",
                    self.candidate.name(),
                    batch_id,
                    e
                )
            })?;
        self.log("inst_relate_aabb", "batch_done", &batch_id.to_string());
        Ok(())
    }

    async fn take_missing_neg_carriers(&self) -> anyhow::Result<Vec<RefnoEnum>> {
        // primary 为准（与所有其他报告字段保持一致）。candidate 也 drain 一次以
        // 保证后续生命周期对称；差异落到 diff log（candidate 通常为空/近似）。
        let primary = self
            .primary
            .take_missing_neg_carriers()
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare primary({}) take_missing_neg_carriers failed: {}",
                    self.primary.name(),
                    e
                )
            })?;
        let candidate = self
            .candidate
            .take_missing_neg_carriers()
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare candidate({}) take_missing_neg_carriers failed: {}",
                    self.candidate.name(),
                    e
                )
            })?;
        if primary.len() != candidate.len() {
            self.log(
                "take_missing_neg_diff",
                "len",
                &format!("primary={} candidate={}", primary.len(), candidate.len()),
            );
        }
        Ok(primary)
    }

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize> {
        let primary_req = ReconcileRequest {
            all_refnos: request.all_refnos,
            candidate_carriers: request.candidate_carriers,
        };
        let candidate_req = ReconcileRequest {
            all_refnos: request.all_refnos,
            candidate_carriers: request.candidate_carriers,
        };

        let primary_inserted =
            self.primary
                .reconcile_missing_neg(primary_req)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "compare primary({}) reconcile_missing_neg failed: {}",
                        self.primary.name(),
                        e
                    )
                })?;
        let candidate_inserted = self
            .candidate
            .reconcile_missing_neg(candidate_req)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "compare candidate({}) reconcile_missing_neg failed: {}",
                    self.candidate.name(),
                    e
                )
            })?;

        if primary_inserted != candidate_inserted {
            self.log(
                "reconcile_diff",
                "inserted",
                &format!(
                    "primary={} candidate={} (candidate may be approximate, e.g. parquet)",
                    primary_inserted, candidate_inserted
                ),
            );
        }
        Ok(primary_inserted)
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        // BooleanBridgeRequest 不是 Copy；按 mission 03 仅做单写到 primary，
        // candidate 按约定走 skipped pipeline（Parquet/DuckLake 当前都跳过 boolean）。
        // 这与 mission 05 §Phase boundary 一致：boolean 是 Phase 2 范围。
        let primary_report = self.primary.run_boolean_bridge(request).await.map_err(|e| {
            anyhow::anyhow!(
                "compare primary({}) run_boolean_bridge failed: {}",
                self.primary.name(),
                e
            )
        })?;
        self.log(
            "boolean_bridge",
            "primary_pipeline",
            primary_report.pipeline,
        );
        // candidate 不带 db_option 时如何 fan-out 是 v4 BridgeContext 的工作。
        // 当前 candidate boolean 跳过，避免误传 DbOption 给 file backend。
        println!(
            "[model-writer:compare] stage=boolean_bridge candidate({}) skipped reason=mission_03_phase2_boolean_only_routed_to_primary",
            self.candidate.name()
        );
        Ok(primary_report)
    }

    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary> {
        let primary_req = FinalizeRequest {
            total_batches: request.total_batches,
            completed_batches: request.completed_batches,
            mesh_cache_hits: request.mesh_cache_hits,
            mesh_new_generated: request.mesh_new_generated,
            missing_neg_candidates: request.missing_neg_candidates,
        };
        let candidate_req = FinalizeRequest {
            total_batches: request.total_batches,
            completed_batches: request.completed_batches,
            mesh_cache_hits: request.mesh_cache_hits,
            mesh_new_generated: request.mesh_new_generated,
            missing_neg_candidates: request.missing_neg_candidates,
        };

        let primary_summary = self.primary.finalize(primary_req).await.map_err(|e| {
            anyhow::anyhow!(
                "compare primary({}) finalize failed: {}",
                self.primary.name(),
                e
            )
        })?;
        let candidate_summary = self.candidate.finalize(candidate_req).await.map_err(|e| {
            anyhow::anyhow!(
                "compare candidate({}) finalize failed: {}",
                self.candidate.name(),
                e
            )
        })?;
        println!(
            "[model-writer:compare] stage=finalize primary={} primary_completed={} candidate={} candidate_completed={} total_batches={}",
            primary_summary.backend,
            primary_summary.completed_batches,
            candidate_summary.backend,
            candidate_summary.completed_batches,
            request.total_batches
        );
        Ok(primary_summary)
    }
}
