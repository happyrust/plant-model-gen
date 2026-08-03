//! ZoneStream 初始化编排入口（spec 030 Scope §3）。
//!
//! Phase 2 只落骨架：模式判定、阶段划分与唯一入口签名。真正的 sidecar 拉起、
//! 双 slot 调度、回填与发布分别在 Phase 4/6/7/9 填入。

use anyhow::{bail, Result};

use crate::web_server::models::{InitializationPipelineMode, ManagedProjectSite};

/// 一次 ZoneStream 运行内部的阶段划分。
///
/// 前六个阶段以 ZONE 为单位循环推进（`DependencyLoad` 每个 dbnum 只做一次），
/// 后三个阶段以 dbnum 为单位收尾。指标 JSON 按同一套阶段名记录耗时（spec 030 R18）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneStreamStage {
    /// 计算本 dbnum 全部 ZONE 的依赖并集并装载 `deps`，产出不可变 deps epoch（ADR-0016 D5）。
    DependencyLoad,
    /// 把单个 ZONE 的设计数据解析进当前 slot。
    ZoneParse,
    /// 产出 `ZoneScopeSeal`：子树、祖先链、CATA 闭包、transform 完整性证明（D6）。
    ZoneSeal,
    /// 在短命子进程中生成该 ZONE 的模型产物，写库绑定当前 slot（D7）。
    ZoneGenerate,
    /// 经 `GenerationOutputBackfill` 顺序回填到目标库（D7）。
    ZoneBackfill,
    /// 目标库 read-back 校验行数与 digest，通过后写 ZONE 检查点。
    ZoneVerify,
    /// 整库层级 / reference / 模型关系审计，发布真实的 dbnum 级 Ready（D6）。
    DbnumAudit,
    /// 导出 Parquet 与 spatial index，先私有 staging 再提升到最终路径（D9 步骤 3）。
    DbnumExport,
    /// 同一 Surreal 事务写 baseline anchor 与 create-only publication（D9 步骤 5）。
    DbnumPublish,
}

impl ZoneStreamStage {
    /// 指标 JSON 与管理页共用的稳定阶段名，不随枚举顺序变化。
    pub fn as_metric_key(self) -> &'static str {
        match self {
            Self::DependencyLoad => "dependency_load",
            Self::ZoneParse => "zone_parse",
            Self::ZoneSeal => "zone_seal",
            Self::ZoneGenerate => "zone_generate",
            Self::ZoneBackfill => "zone_backfill",
            Self::ZoneVerify => "zone_verify",
            Self::DbnumAudit => "dbnum_audit",
            Self::DbnumExport => "dbnum_export",
            Self::DbnumPublish => "dbnum_publish",
        }
    }
}

/// 站点是否走 ZoneStream 初始化。
pub fn is_zone_stream(site: &ManagedProjectSite) -> bool {
    site.initialization_pipeline_mode == InitializationPipelineMode::ZoneStream
}

/// 拒绝 ZoneStream 站点走 Legacy 的解析 / 生成入口。
///
/// ADR-0016 D1 要求分流发生在编排入口而不是旧入口内部，所以这里只做守卫、不做转发：
/// ZoneStream 的 Start / Stop / Resume 由 `TaskType::ZoneStreamInitialization` 承担
/// （Phase 3），届时本函数的错误会被映射成 HTTP 409。
pub fn reject_legacy_entry_for_zone_stream(site: &ManagedProjectSite, entry: &str) -> Result<()> {
    if is_zone_stream(site) {
        bail!(
            "站点 `{}` 的初始化流水模式为 zone-stream，不能走 Legacy 的{}入口；\
             请使用 ZoneStreamInitialization 任务的 Start / Stop / Resume。",
            site.site_id,
            entry
        );
    }
    Ok(())
}

/// Start：ZoneStream 初始化的唯一入口。
///
/// 失败即失败，不回退 Legacy（ADR-0016 D1 / D11）。
pub async fn run_initialization(site_id: &str) -> Result<()> {
    bail!(
        "ZoneStream 初始化尚未实现（站点 `{site_id}`）：spec 030 已落配置分流、任务与运行记录，\
         流水调度在 Phase 4 及之后接入。当前请把站点的 initialization_pipeline_mode 设回 legacy。"
    )
}

/// Stop：在当前解析或写回批次边界停止。
///
/// 停止后 task 标记 `Cancelled`、run 标记 `Interrupted`，未完成的 ZONE 不写检查点
/// （ADR-0016 D9 恢复规则）。
pub async fn request_stop(site_id: &str) -> Result<()> {
    bail!(
        "ZoneStream Stop 尚未实现（站点 `{site_id}`）：批次边界停止与 Interrupted 落库在 Phase 10 接入。"
    )
}

/// Resume：仅在源 manifest、contract hash、ZONE plan 三者一致时继续同一个 run。
///
/// 允许在 Resume 前调大 `zone_stream_memory_budget_mib`——预算不参与判等（ADR-0016 D10）。
pub async fn resume_initialization(site_id: &str) -> Result<()> {
    bail!(
        "ZoneStream Resume 尚未实现（站点 `{site_id}`）：run 判等、槽位清空与半写行清理在 Phase 10 接入。"
    )
}
