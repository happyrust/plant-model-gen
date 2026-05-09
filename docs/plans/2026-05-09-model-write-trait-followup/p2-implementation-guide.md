# P2 实施手册 — Mock 与契约验证

> **状态**：P1 已完成；本文件是 P2 三个 Task 的细颗粒度执行手册，对 `task_plan.md` §5 Phase 2 的展开。
> **读者**：执行 P2 的 agent / 工程师；本文件假设读者已读过 `task_plan.md` 与 `findings.md`。

---

## 0. P2 上下文（基于 P1 完成后的 worktree）

```
.worktrees/model-persistence-trait/
├── src/fast_model/gen_model/model_writer/
│   ├── mod.rs          # trait + 类型 + factory
│   ├── surreal.rs      # SurrealModelWriteBackend
│   └── drain_only.rs   # DrainOnlyModelWriteBackend + DrainOnlyStats + run_drain_only_sink
└── Cargo.toml          # 现有 features: surreal-save / write-to-surrealdb / model-writer-drain
```

P2 目标：让 trait 可被一个**完全脱离 SurrealDB 与真实 mesh 计算**的后端实现，并在独立 binary 中跑通完整调用链断言。

---

## 1. 设计前置：Mock backend 的范围与不范围

### 1.1 范围（必须做）

- 实现 `ModelWriteBackend` 全部 8 个方法。
- 每个方法把 `(method_name, key_args)` 记录到顺序日志 `Vec<String>`（用 `Mutex` 包，不引入 async 锁）。
- `write_base_batch` 返回**可控**的 `SaveInstanceDataReport`（默认空，但允许通过 builder 注入 `missing_neg_carriers` 模拟"有 reconcile 候选"分支）。
- `reconcile_missing_neg` 返回**可控** `usize`（默认 0，可注入）。
- `run_boolean_bridge` 返回**可控** `BooleanBridgeReport`（默认 `BooleanBridgeReport::skipped(...)`，可注入）。
- `finalize` 返回 `FinalizeSummary { backend: "mock", ... }`，并把传入的 request 内容也记入日志便于断言。

### 1.2 不范围（不要做）

- 不调用 `aios_core::rs_surreal::*`、`pdms_inst::*`、`pdms_inst_surreal::*`、`run_boolean_worker`、`run_bool_worker_from_tasks` 中任何一个真实底层。
- 不引入新的依赖（不要加 `mockall` / `wiremock` 等）。`Mutex<Vec<String>>` 已经够用。
- 不模拟 `model_primary_db()`：mock 只测调用契约，不测 SQL 语义。

### 1.3 Feature flag 设计

在 `Cargo.toml` 新增：

```toml
# Mock 后端：只用于 trait 契约验证 binary，不进 release
model-writer-mock = []
```

`mock.rs` 顶部加：

```rust
#![cfg(feature = "model-writer-mock")]
```

`model_writer/mod.rs` 加：

```rust
#[cfg(feature = "model-writer-mock")]
mod mock;

#[cfg(feature = "model-writer-mock")]
pub use mock::RecordingBackend;
```

**注意**：feature 仅启用 mock 模块本身，**不**改变 `create_model_writer` 工厂的行为——mock backend 由 verify binary 直接 `Arc::new(RecordingBackend::default())` 构造，不进生产路径。

---

## 2. T2.1 — 完整 `mock.rs` 骨架

**文件**：`.worktrees/model-persistence-trait/src/fast_model/gen_model/model_writer/mock.rs`

```rust
#![cfg(feature = "model-writer-mock")]

use std::sync::Mutex;

use aios_core::RefnoEnum;
use async_trait::async_trait;

use super::super::pdms_inst::SaveInstanceDataReport;
use super::{
    BaseInstanceBatch, BooleanBridgeReport, BooleanBridgeRequest, CleanupRequest, FinalizeRequest,
    FinalizeSummary, InstRelateAabbBatch, MeshResultBatch, ModelWriteBackend, ModelWriterContext,
    ReconcileRequest,
};

/// 记录每次 trait 方法调用的 fixture backend，仅用于契约验证。
///
/// **不要**用于 release 构建：本类型仅在 feature `model-writer-mock` 下编译。
#[derive(Debug, Default)]
pub struct RecordingBackend {
    calls: Mutex<Vec<String>>,
    /// 注入：`reconcile_missing_neg` 应返回的值（默认 0）。
    pub injected_reconcile_inserted: Mutex<usize>,
    /// 注入：`write_base_batch` 应返回的 `missing_neg_carriers`（默认空）。
    pub injected_missing_neg: Mutex<Vec<RefnoEnum>>,
}

impl RecordingBackend {
    pub fn snapshot(&self) -> Vec<String> {
        self.calls.lock().expect("recording lock").clone()
    }

    pub fn record(&self, line: impl Into<String>) {
        self.calls.lock().expect("recording lock").push(line.into());
    }
}

#[async_trait]
impl ModelWriteBackend for RecordingBackend {
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
    ) -> anyhow::Result<SaveInstanceDataReport> {
        self.record(format!(
            "write_base_batch:batch={},inst_info={},inst_tubi={},replace_exist={},write_inst_relate_aabb={}",
            batch.batch_id,
            batch.shape_insts.inst_info_map.len(),
            batch.shape_insts.inst_tubi_map.len(),
            batch.replace_exist,
            batch.write_inst_relate_aabb
        ));
        let missing_neg_carriers = self
            .injected_missing_neg
            .lock()
            .expect("recording lock")
            .clone();
        Ok(SaveInstanceDataReport {
            missing_neg_carriers,
        })
    }

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        self.record(format!(
            "persist_mesh_results:batch={},mesh_results={}",
            batch.batch_id,
            batch.mesh_results.len()
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
        Ok(*self.injected_reconcile_inserted.lock().expect("recording lock"))
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        self.record(format!(
            "run_boolean_bridge:mode={:?},bool_tasks={},use_surrealdb={},defer_db_write={}",
            request.mode,
            request.bool_tasks.len(),
            request.use_surrealdb,
            request.defer_db_write
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
```

**审查 checklist**：

- [ ] `#![cfg(feature = "model-writer-mock")]` 在文件顶部
- [ ] 所有 8 个 trait 方法都 `record` 一行
- [ ] 不引用 `pdms_inst::save_instance_data_with_report` 等真实底层
- [ ] `injected_*` 字段都用 `Mutex` 而非 `RefCell`（要 Send + Sync）
- [ ] 顶部 `use super::super::pdms_inst::SaveInstanceDataReport`（接口纯化在 P3 才动）

---

## 3. T2.2 — `verify_model_writer_trait` binary

### 3.1 Cargo.toml 改动

在 `[[bin]]` 段后追加：

```toml
[[bin]]
name = "verify_model_writer_trait"
path = "src/bin/verify_model_writer_trait.rs"
required-features = ["model-writer-mock"]
```

### 3.2 Fixture 构造说明

`BaseInstanceBatch` / `MeshResultBatch` / `InstRelateAabbBatch` 都借用 `&'a ShapeInstancesData` / `&'a HashMap<u64, MeshResult>` / `&'a DashMap<...>`。手工构造时：

- `ShapeInstancesData` 来自 `aios_core::geometry`，需要看其默认构造或字段全 `Default`。
  **预检**：先 `rg "impl Default for ShapeInstancesData"` 确认存在；若无，需 `rg "pub struct ShapeInstancesData"` 看字段，按字段全部填默认值或加 `#[derive(Default)]` 后向兼容。
- `MeshResult` 同样 `rg "impl Default for MeshResult"` 预检。
- `DashMap::new()` 即可。

如果 `ShapeInstancesData` 没有 `Default`，**回退方案**：mock 不做真实 batch 构造，改用 `MaybeUninit::zeroed().assume_init()` 或者把验证简化为只测**没有 batch 阶段**的 7 个方法（init / cleanup / reconcile_missing_neg / run_boolean_bridge / finalize）+ 1 个手工 mock batch（只测 `write_base_batch` / `persist_mesh_results` / `write_inst_relate_aabb` 是否都被调，不验内容）。

### 3.3 完整 binary 骨架

**文件**：`.worktrees/model-persistence-trait/src/bin/verify_model_writer_trait.rs`

```rust
#![cfg(feature = "model-writer-mock")]

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;

use aios_core::geometry::ShapeInstancesData;
use aios_database::fast_model::gen_model::model_writer::{
    BaseInstanceBatch, BooleanBridgeRequest, CleanupRequest, FinalizeRequest, InstRelateAabbBatch,
    MeshResultBatch, ModelWriteBackend, ModelWriterContext, ReconcileRequest, RecordingBackend,
};
use aios_database::options::{BooleanPipelineMode, ModelWriterMode};
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let backend = Arc::new(RecordingBackend::default());

    let context = ModelWriterContext {
        project_name: "verify-fixture".to_string(),
        use_surrealdb: true,
        defer_db_write: false,
        mode: ModelWriterMode::Surreal,
    };

    let shape_insts = ShapeInstancesData::default(); // 若无 Default 见 §3.2 回退
    let mesh_aabb_map: DashMap<String, Aabb> = DashMap::new();
    let mesh_pts_map: DashMap<u64, String> = DashMap::new();
    let mesh_results: HashMap<u64, super::MeshResult> = HashMap::new();

    macro_rules! check {
        ($e:expr, $msg:literal) => {
            match $e.await {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[verify] FAIL: {}: {}", $msg, e);
                    return ExitCode::from(1);
                }
            }
        };
    }

    check!(backend.init(&context), "init");

    check!(
        backend.cleanup(CleanupRequest { seed_refnos: &[] }),
        "cleanup"
    );

    check!(
        backend.write_base_batch(BaseInstanceBatch {
            batch_id: 1,
            shape_insts: &shape_insts,
            mesh_aabb_map: &mesh_aabb_map,
            replace_exist: false,
            write_inst_relate_aabb: false,
        }),
        "write_base_batch"
    );

    check!(
        backend.persist_mesh_results(MeshResultBatch {
            batch_id: 1,
            mesh_results: &mesh_results,
            mesh_aabb_map: &mesh_aabb_map,
            mesh_pts_map: &mesh_pts_map,
        }),
        "persist_mesh_results"
    );

    check!(
        backend.write_inst_relate_aabb(InstRelateAabbBatch {
            batch_id: 1,
            shape_insts: &shape_insts,
            mesh_results: &mesh_results,
            mesh_aabb_map: &mesh_aabb_map,
        }),
        "write_inst_relate_aabb"
    );

    check!(
        backend.reconcile_missing_neg(ReconcileRequest {
            all_refnos: &[],
            candidate_carriers: &[],
        }),
        "reconcile_missing_neg"
    );

    check!(
        backend.run_boolean_bridge(BooleanBridgeRequest {
            mode: BooleanPipelineMode::DbLegacy,
            db_option: Arc::new(aios_core::options::DbOption::default()),
            bool_tasks: Vec::new(),
            use_surrealdb: true,
            defer_db_write: false,
        }),
        "run_boolean_bridge"
    );

    check!(
        backend.finalize(FinalizeRequest::default()),
        "finalize"
    );

    let snapshot = backend.snapshot();
    let expected_prefixes = [
        "init:",
        "cleanup:",
        "write_base_batch:",
        "persist_mesh_results:",
        "write_inst_relate_aabb:",
        "reconcile_missing_neg:",
        "run_boolean_bridge:",
        "finalize:",
    ];

    if snapshot.len() != expected_prefixes.len() {
        eprintln!(
            "[verify] FAIL: 调用计数不符 expected={} got={}",
            expected_prefixes.len(),
            snapshot.len()
        );
        for line in &snapshot {
            eprintln!("  - {}", line);
        }
        return ExitCode::from(2);
    }

    for (idx, prefix) in expected_prefixes.iter().enumerate() {
        if !snapshot[idx].starts_with(prefix) {
            eprintln!(
                "[verify] FAIL: 第 {} 步预期前缀 `{}`, 实得 `{}`",
                idx, prefix, snapshot[idx]
            );
            return ExitCode::from(3);
        }
    }

    println!("[verify] PASS — 8 个 trait 方法按预期顺序被调用");
    for (idx, line) in snapshot.iter().enumerate() {
        println!("  [{}] {}", idx + 1, line);
    }
    ExitCode::SUCCESS
}
```

**审查 checklist**：

- [ ] `#![cfg(feature = "model-writer-mock")]` 在 binary 顶部
- [ ] `Cargo.toml` 用 `required-features = ["model-writer-mock"]` 守门
- [ ] exit code: 0 / 1 / 2 / 3 各对应不同失败原因，便于脚本区分
- [ ] 输出格式 `[verify] PASS` / `[verify] FAIL: <reason>` 利于 grep
- [ ] `ShapeInstancesData::default()` 若不存在按 §3.2 回退方案

### 3.4 预检命令

启动 T2.2 前先跑：

```powershell
rg -n "impl Default for ShapeInstancesData" rs-core/ src/
rg -n "impl Default for MeshResult" rs-core/ src/
rg -n "impl Default for DbOption" rs-core/ src/
```

如果三者全有 `Default`，按 §3.3 直接做；任一缺失，先解决再写 binary。

---

## 4. T2.3 — PowerShell 验证脚本

**文件**：`docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1`

```powershell
#!/usr/bin/env pwsh
# Verify ModelWriterBackend trait 契约（基于 RecordingBackend mock 后端）。
#
# 前置：
#   - 已完成 P2 的 T2.1 / T2.2
#   - cargo + nightly toolchain + NASM 在 PATH
#
# 用法：
#   pwsh -NoProfile -File docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1
#
# 退出码：
#   0   — 通过
#   1   — binary build 失败
#   2-9 — verify_model_writer_trait 自身的 FAIL 退出码（透传）

param(
    [string]$WorkdirPath = "$PSScriptRoot/../../../.worktrees/model-persistence-trait",
    [switch]$VerboseRun
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $WorkdirPath)) {
    Write-Host "[verify-mock] FAIL: worktree 不存在: $WorkdirPath"
    exit 1
}

Push-Location $WorkdirPath
try {
    $started = Get-Date
    Write-Host "[verify-mock] 编译 + 运行 verify_model_writer_trait..."

    if ($VerboseRun) {
        cargo run --bin verify_model_writer_trait --features model-writer-mock 2>&1 | Tee-Object -Variable runOutput
    } else {
        $runOutput = cargo run --bin verify_model_writer_trait --features model-writer-mock 2>&1
    }
    $exit = $LASTEXITCODE
    $elapsed = [int]((Get-Date) - $started).TotalSeconds

    if ($exit -ne 0) {
        Write-Host "[verify-mock] FAIL: exit=$exit, elapsed=${elapsed}s"
        $runOutput | Select-Object -Last 30 | ForEach-Object { Write-Host "  $_" }
        exit $exit
    }

    Write-Host "[verify-mock] PASS — elapsed=${elapsed}s"
    exit 0
} finally {
    Pop-Location
}
```

**审查 checklist**：

- [ ] 用 `Push-Location` / `Pop-Location` 隔离工作目录改动
- [ ] 退出码透传（不要硬编码）
- [ ] 不依赖 `cargo` 在 worktree PATH 里——靠用户预设
- [ ] 默认不冗余 cargo 输出，加 `-VerboseRun` 才展开

---

## 5. P2 风险与回退

| 风险 | 触发条件 | 回退方案 |
|---|---|---|
| `ShapeInstancesData` / `MeshResult` / `DbOption` 任一无 `Default` impl | binary 编译失败 | 在 verify binary 内手工构造 zeroed 实例（不安全），或简化断言为"7 个非 batch 方法 + 1 个 batch 方法"；同时把"加 `Default`" 立项到独立 PR（不在本 worktree 改 rs-core） |
| `model-writer-mock` feature 与 `default = ["review"]` 互斥（如有 `not(feature = "x")` 守卫） | `cargo run --bin verify_model_writer_trait --features model-writer-mock` build 失败 | 显式加 `--no-default-features --features model-writer-mock,gen_model,manifold,project_hd,write-to-surrealdb`，最小集编 mock |
| `aios_core::options::DbOption` 字段过多导致 fixture 构造繁琐 | binary 写不动 | 在 verify binary 内 `Arc::new(...)` 时用 `DbOption::default()`（前提 §3.4 预检通过）；若 default 不存在，加 helper `fn fixture_db_option() -> DbOption` 集中维护 |
| `cargo run` 在本机受 NASM/PATH 阻塞 | T2.2/T2.3 跑不动 | 与 P1 验证策略一致：模块编译 + IDE lint 视为"代码层面正确"，完整 build 移到 P4 推 PR 前一并跑通 |
| RecordingBackend 与真实 SurrealBackend 的语义偏差被忽略 | 长期债 | P3 接口纯化后，把真实 backend 也跑一次 verify（顶替 fixture），对比 snapshot 偏差 |

---

## 6. P2 完成判定

- [ ] `mock.rs` 文件已创建，`#![cfg(feature = "model-writer-mock")]` 守门
- [ ] `Cargo.toml` 加 `model-writer-mock = []` feature + `verify_model_writer_trait` bin 段（`required-features` 标对）
- [ ] `model_writer/mod.rs` 加 `#[cfg(feature = "model-writer-mock")] mod mock; pub use mock::RecordingBackend;`
- [ ] `src/bin/verify_model_writer_trait.rs` 已创建，覆盖 8 个 trait 方法 + snapshot 断言
- [ ] `docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1` 可独立运行
- [ ] `ReadLints` 全绿
- [ ] `progress.md` 更新：T2.1 / T2.2 / T2.3 全部 complete

完整 cargo build 验证移到 P4 末尾。

---

## 7. 与下游 Phase 的接口

P3 的 T3.1 会改 trait 返回类型（移除 `SaveInstanceDataReport`），届时 `mock.rs` 需要同步更新；
P3 的 T3.2 会去掉 `BooleanBridgeRequest` 冗余字段，verify binary 也要改。

**约定**：每次改 trait 接口，必须**同时**改 mock + verify binary，并跑一遍 verify-mock.ps1，确保契约同步。

---

## 8. 出问题怎么升级

按 `task_plan.md §7` 的 3-strike 协议：3 次失败后写入 `progress.md` 错误表，调 best-mcp-sqlite-5 `check_messages` 把症状递交给用户决策。
