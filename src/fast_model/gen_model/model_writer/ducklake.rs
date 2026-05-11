//! DuckLake backend skeleton — v3 Phase D placeholder for v4 real implementation.
//!
//! mission `04-ducklake-writer.md` 把 DuckLake target design 定为
//! "通过 Rust DuckDB binding 直接写 ducklake-canonical schema"。v3 范围
//! 不实装真实写入（避免引入 `duckdb` crate 依赖污染本轮 review）；本骨架
//! 只承担三件事：
//!
//! 1. 占住 `ModelWriterMode::DuckLake` 在工厂里的分支
//! 2. 给 trait 8 个方法一个具体 impl，便于 v4 直接填充
//! 3. 让 `cargo check --features ducklake` 编过，以便后续 PR 在 feature 开启
//!    的情况下迭代真实写入
//!
//! 所有 trait 方法返回 `bail!("DuckLake backend skeleton, not yet implemented
//! (mission docs/04). Use parquet/surreal in v3.")`，确保**任何**误用都会
//! 立即失败而不是静默成功。`init` 也是 `bail!`，因为没有任何后续 stage 能
//! 真正持久化数据；让生产端在 init 阶段就被打断，避免空跑生成 pipeline。

#![cfg(feature = "ducklake")]

use async_trait::async_trait;

use super::{
    BaseInstanceBatch, BooleanBridgeReport, BooleanBridgeRequest, CleanupRequest, FinalizeRequest,
    FinalizeSummary, InstRelateAabbBatch, MeshResultBatch, ModelWriterBackend, ModelWriterContext,
    ReconcileRequest, WriteBaseReport,
};

/// DuckLake backend skeleton. **All trait methods bail.** v4 will replace these
/// with real implementations backed by the Rust DuckDB binding.
#[derive(Debug, Default)]
pub struct DuckLakeModelWriterBackend {
    /// v4 will hold the duckdb connection + ducklake metadata path here.
    /// Kept as ZST during the skeleton phase to avoid unused-field churn.
    _phantom: (),
}

impl DuckLakeModelWriterBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

fn not_implemented(stage: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "[model-writer:ducklake] stage={} DuckLake backend skeleton, not yet implemented (mission docs/04-ducklake-writer.md). Use --model-writer parquet|surreal in v3; real DuckLake support lands in v4 with the duckdb crate.",
        stage
    )
}

#[async_trait]
impl ModelWriterBackend for DuckLakeModelWriterBackend {
    fn name(&self) -> &'static str {
        "ducklake"
    }

    async fn init(&self, _context: &ModelWriterContext) -> anyhow::Result<()> {
        Err(not_implemented("init"))
    }

    async fn cleanup(&self, _request: CleanupRequest<'_>) -> anyhow::Result<()> {
        Err(not_implemented("cleanup"))
    }

    async fn write_base_batch(
        &self,
        _batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<WriteBaseReport> {
        Err(not_implemented("write_base_batch"))
    }

    async fn persist_mesh_results(&self, _batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        Err(not_implemented("persist_mesh_results"))
    }

    async fn write_inst_relate_aabb(
        &self,
        _batch: InstRelateAabbBatch<'_>,
    ) -> anyhow::Result<()> {
        Err(not_implemented("write_inst_relate_aabb"))
    }

    async fn reconcile_missing_neg(
        &self,
        _request: ReconcileRequest<'_>,
    ) -> anyhow::Result<usize> {
        Err(not_implemented("reconcile_missing_neg"))
    }

    async fn run_boolean_bridge(
        &self,
        _request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        Err(not_implemented("run_boolean_bridge"))
    }

    async fn finalize(&self, _request: FinalizeRequest) -> anyhow::Result<FinalizeSummary> {
        Err(not_implemented("finalize"))
    }
}
