# P3 实施手册 — 接口纯化

> **状态**：P1 已完成；P2 实施手册已写。本手册细化 `task_plan.md` §5 Phase 3 的 4 个 Task。
> **核心目标**：让 trait 接口不再泄漏 `pdms_inst::SaveInstanceDataReport` / `Arc<aios_core::DbOption>` 等 surreal 内部类型，并移除接口冗余。
> **读者**：执行 P3 的 agent / 工程师；假设已读过 task_plan.md / findings.md / p2-implementation-guide.md。

---

## 0. P3 上下文

P1 完成后 trait 接口签名（`model_writer/mod.rs`）：

| 方法 | 当前签名（关键部分） | 要解决的问题 |
|---|---|---|
| `write_base_batch` | `-> Result<SaveInstanceDataReport>` | 返回 `pdms_inst::SaveInstanceDataReport` — W1 接口耦合泄漏 |
| `run_boolean_bridge` | `BooleanBridgeRequest { db_option: Arc<aios_core::options::DbOption>, use_surrealdb, defer_db_write, ... }` | W1 + W2 接口耦合 + 字段冗余 |
| `persist_mesh_results` | 内部读全局 `use_file_mesh_state()` | W3 隐式全局状态 |
| `init` | `ensure!(context.use_surrealdb, ...)` 时机偏晚 | W4 |

P3 不动：
- `BaseInstanceBatch` / `MeshResultBatch` / `InstRelateAabbBatch` / `CleanupRequest` / `ReconcileRequest` 的 lifetime + DashMap 字段（这些是 hot path，零拷贝借用，不可改）
- `BooleanPipelineMode` 枚举（语义已稳定）
- DrainOnly / Surreal backend 的对外接口（仅改内部读取来源）

P3 完成后 mock backend 与 verify binary 都需同步改（按 `p2-implementation-guide.md §7` 约定）。

---

## 1. 设计原则

### 1.1 抽象边界

| 类型 | 应在 trait 接口出现 | 应留在 backend 内部 |
|---|---|---|
| `WriteBaseReport`（新建） | ✓ | × |
| `pdms_inst::SaveInstanceDataReport` | × | ✓ |
| `BooleanBridgeRequest`（精简后） | ✓ | × |
| `Arc<aios_core::options::DbOption>` | ⚠️ 短期保留（boolean worker 需要） | — |
| `RefnoEnum` | ✓（管线公共类型） | — |
| `ModelWriterContext` | ✓（init 时传一次） | — |

### 1.2 调用契约

- **init 一次，缓存 context**：backend 内部用 `OnceCell<ModelWriterContext>` 或 `RwLock<Option<ModelWriterContext>>` 缓存 init 时传入的 context；后续方法（如 `run_boolean_bridge`）从中读 `use_surrealdb` / `defer_db_write`，不再从 request 字段读。
- **mock backend 等价处理**：mock 也缓存 context，但不强制断言 init 在前（保持调用顺序的灵活性）；contract 测试由 `verify_model_writer_trait` binary 显式断言"先调 init"。
- **fallback 安全**：若 backend 在 init 之前被调 `run_boolean_bridge`，应以 `BooleanBridgeReport::skipped(..., "context not initialized")` 优雅回退，不 panic。

---

## 2. T3.1 — 引入 `WriteBaseReport`

### 2.1 现状

`model_writer/mod.rs` 导入 `pub use super::pdms_inst::SaveInstanceDataReport;`（实际是 `use ... pdms_inst::SaveInstanceDataReport`），在 trait 签名直接返回。

`SaveInstanceDataReport` 定义见 `pdms_inst.rs:123-125`：

```rust
pub struct SaveInstanceDataReport {
    pub missing_neg_carriers: Vec<RefnoEnum>,
}
```

只有一个字段。Trait 接口可以用更精简的本地类型。

### 2.2 目标

在 `model_writer/mod.rs` 新增：

```rust
/// `write_base_batch` 的对外 report，不耦合具体 backend 内部类型。
#[derive(Debug, Clone, Default)]
pub struct WriteBaseReport {
    pub batch_id: u64,
    pub missing_neg_count: usize,
    pub missing_neg_carriers: Vec<RefnoEnum>,
}
```

trait 签名改为：

```rust
async fn write_base_batch(&self, batch: BaseInstanceBatch<'_>) -> anyhow::Result<WriteBaseReport>;
```

### 2.3 实现差量

#### 2.3.1 `model_writer/mod.rs`

- 删除 `use super::pdms_inst::SaveInstanceDataReport;` 这一行（该类型不再在 trait 接口暴露）
- 新增 `WriteBaseReport` 定义（如上）
- trait 签名改返回值

#### 2.3.2 `model_writer/surreal.rs`

```rust
async fn write_base_batch(
    &self,
    batch: BaseInstanceBatch<'_>,
) -> anyhow::Result<WriteBaseReport> {
    println!("[model-writer:surreal] stage=base batch={} ...", batch.batch_id);
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
    let missing_neg_count = report.missing_neg_carriers.len();
    println!(
        "[model-writer:surreal] stage=base batch={} done missing_neg_candidates={}",
        batch.batch_id, missing_neg_count
    );
    Ok(WriteBaseReport {
        batch_id: batch.batch_id,
        missing_neg_count,
        missing_neg_carriers: report.missing_neg_carriers,
    })
}
```

注意：`use super::super::pdms_inst::{self};` 仍保留（surreal.rs 内部需要调 pdms_inst 各函数）。

#### 2.3.3 `model_writer/drain_only.rs`

```rust
async fn write_base_batch(
    &self,
    batch: BaseInstanceBatch<'_>,
) -> anyhow::Result<WriteBaseReport> {
    {
        let mut stats = self.stats.lock().expect("drain-only stats lock");
        stats.add_batch(batch.shape_insts);
    }
    println!("[model-writer:drain-only] stage=base batch={} ...", batch.batch_id);
    Ok(WriteBaseReport {
        batch_id: batch.batch_id,
        missing_neg_count: 0,
        missing_neg_carriers: Vec::new(),
    })
}
```

drain_only.rs 顶部删除 `use super::super::pdms_inst::SaveInstanceDataReport;`（不再需要）。

#### 2.3.4 `orchestrator.rs`

`run_base_writer` 内部消费 report 的代码：

```rust
// 旧
if !save_report.missing_neg_carriers.is_empty() {
    let mut guard = missing_neg_carriers.lock().unwrap();
    guard.extend(save_report.missing_neg_carriers.iter().copied());
}

// 新（字段名一致，无需改；只是类型从 SaveInstanceDataReport 变成 WriteBaseReport）
if !save_report.missing_neg_carriers.is_empty() {
    let mut guard = missing_neg_carriers.lock().unwrap();
    guard.extend(save_report.missing_neg_carriers.iter().copied());
}
```

字段名保持 `missing_neg_carriers`，调用代码改动量为 0。

`println!` 中 `save_report.missing_neg_carriers.len()` 改为 `save_report.missing_neg_count`（数据相同，更明确）。

#### 2.3.5 mock.rs（同步改）

```rust
async fn write_base_batch(
    &self,
    batch: BaseInstanceBatch<'_>,
) -> anyhow::Result<WriteBaseReport> {
    self.record(format!("write_base_batch:batch={},...", batch.batch_id));
    let missing_neg_carriers = self
        .injected_missing_neg
        .lock()
        .expect("recording lock")
        .clone();
    let missing_neg_count = missing_neg_carriers.len();
    Ok(WriteBaseReport {
        batch_id: batch.batch_id,
        missing_neg_count,
        missing_neg_carriers,
    })
}
```

### 2.4 完成判定

- [ ] `model_writer/mod.rs` 不再 `use ... SaveInstanceDataReport`
- [ ] `WriteBaseReport` 已定义并 `pub`
- [ ] 三个 backend impl 都返回 `WriteBaseReport`
- [ ] `orchestrator.rs::run_base_writer` 编译通过
- [ ] `verify_model_writer_trait` binary 通过

### 2.5 风险

| 风险 | 处置 |
|---|---|
| `WriteBaseReport` 未来字段膨胀（新 backend 想塞 metric） | 用 `#[non_exhaustive]` 标注 + builder pattern；本 Phase 不用，但留口 |
| `pdms_inst` 其他调用方仍在用 `SaveInstanceDataReport` | 保留 `pdms_inst::SaveInstanceDataReport` 类型本身；只是 trait 不再暴露它 |

---

## 3. T3.2 — 去除 `BooleanBridgeRequest` 冗余字段

### 3.1 现状

```rust
pub struct BooleanBridgeRequest {
    pub mode: BooleanPipelineMode,
    pub db_option: Arc<aios_core::options::DbOption>,
    pub bool_tasks: Vec<BooleanTask>,
    pub use_surrealdb: bool,        // ← 冗余（init context 已带）
    pub defer_db_write: bool,       // ← 冗余
}
```

调用方（`orchestrator.rs:1424` / `1475`）每次都重复传：

```rust
let report = model_writer
    .run_boolean_bridge(BooleanBridgeRequest {
        mode: BooleanPipelineMode::DbLegacy,
        db_option: Arc::new(db_option.inner.clone()),
        bool_tasks: Vec::new(),
        use_surrealdb,           // ← 已经在 ModelWriterContext 里
        defer_db_write,          // ← 已经在 ModelWriterContext 里
    })
    .await?;
```

### 3.2 目标

精简为：

```rust
pub struct BooleanBridgeRequest {
    pub mode: BooleanPipelineMode,
    pub db_option: Arc<aios_core::options::DbOption>,  // §4.4 短期保留
    pub bool_tasks: Vec<BooleanTask>,
}
```

backend 内部用 init 时缓存的 context 决定行为。

### 3.3 实现差量

#### 3.3.1 `model_writer/mod.rs`

`BooleanBridgeRequest` 删除 `use_surrealdb` / `defer_db_write` 两个字段。

#### 3.3.2 `model_writer/surreal.rs`

新增 context 缓存：

```rust
use std::sync::OnceLock;

#[derive(Debug)]
pub struct SurrealModelWriteBackend {
    context: OnceLock<ModelWriterContext>,
}

impl Default for SurrealModelWriteBackend {
    fn default() -> Self {
        Self {
            context: OnceLock::new(),
        }
    }
}
```

`init` 内：

```rust
async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()> {
    println!("[model-writer:surreal] stage=init ...");
    anyhow::ensure!(
        context.use_surrealdb,
        "Surreal model writer requires use_surrealdb=true ..."
    );
    aios_core::rs_surreal::inst::init_model_tables()
        .await
        .context("model_writer surreal init_model_tables failed")?;
    let _ = self.context.set(context.clone());
    Ok(())
}
```

`run_boolean_bridge` 改为读 context：

```rust
async fn run_boolean_bridge(
    &self,
    request: BooleanBridgeRequest,
) -> anyhow::Result<BooleanBridgeReport> {
    let Some(ctx) = self.context.get() else {
        return Ok(BooleanBridgeReport::skipped(
            "uninitialized",
            request.bool_tasks.len(),
            "init not called",
        ));
    };
    match request.mode {
        BooleanPipelineMode::DbLegacy => {
            if ctx.use_surrealdb && !ctx.defer_db_write {
                run_boolean_worker(request.db_option, 100).await?;
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
            if !ctx.use_surrealdb {
                return Ok(BooleanBridgeReport::skipped(
                    "memory_tasks",
                    request.bool_tasks.len(),
                    "use_surrealdb=false",
                ));
            }
            let report = run_bool_worker_from_tasks(request.bool_tasks, request.db_option, None)
                .await?;
            Ok(report.into())
        }
    }
}
```

#### 3.3.3 `model_writer/drain_only.rs`

`DrainOnlyModelWriteBackend` 已有 `started: Mutex<Option<Instant>>`，加一个 `context: Mutex<Option<ModelWriterContext>>` 字段。`init` 时填入。`run_boolean_bridge` 不需要读 context（直接 NoOp，已无关 use_surrealdb）。

#### 3.3.4 `orchestrator.rs`

调用方 `BooleanBridgeRequest { ... }` 中删除 `use_surrealdb` / `defer_db_write` 两行赋值。

#### 3.3.5 `mock.rs`

`record` 中去掉 `use_surrealdb` / `defer_db_write` 字段；`RecordingBackend` 也加 `context: Mutex<Option<ModelWriterContext>>`，`init` 时填入（用于将来"未 init 时调其他方法应 fail-safe"的契约测试）。

### 3.4 风险

| 风险 | 处置 |
|---|---|
| backend 实例被多次 init（同一 SurrealBackend 复用） | `OnceLock` 第二次 set 会失败但不 panic；本 Phase 接受现状（生产路径每次 process_index_tree_generation 新建 backend）；如未来要复用，改 `RwLock<Option<...>>` |
| 调用方某些路径未调 init 直接调 `run_boolean_bridge` | 已加 `Some(ctx) else { skipped }` 优雅回退 |
| context 字段后续扩张 | `ModelWriterContext` 已是 owned `String` + `bool` * 2 + enum，clone 成本可忽略 |

---

## 4. T3.3 — `persist_mesh_results` 移除全局依赖

### 4.1 现状

`SurrealModelWriteBackend::persist_mesh_results` 内部：

```rust
if use_file_mesh_state() {
    flush_aabb_cache();
    println!("... file_mesh_state=true ...");
    return Ok(());
}
```

`use_file_mesh_state()` / `flush_aabb_cache()` 都来自 `super::super::mesh_state`，是进程级全局状态。

### 4.2 目标

把开关塞进 batch 字段：

```rust
pub struct MeshResultBatch<'a> {
    pub batch_id: u64,
    pub mesh_results: &'a HashMap<u64, MeshResult>,
    pub mesh_aabb_map: &'a DashMap<String, Aabb>,
    pub mesh_pts_map: &'a DashMap<u64, String>,
    pub file_mesh_state: bool,
}
```

backend 不再调全局函数。

### 4.3 实现差量

#### 4.3.1 `model_writer/mod.rs`

`MeshResultBatch` 加 `pub file_mesh_state: bool` 字段。

#### 4.3.2 `model_writer/surreal.rs`

```rust
async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
    if batch.file_mesh_state {
        // 文件 mesh 模式仍需 flush_aabb_cache：这是 surreal backend 的实现细节，保留
        flush_aabb_cache();
        println!(
            "[model-writer:surreal] stage=mesh_results batch={} file_mesh_state=true flushed_aabb_cache=true",
            batch.batch_id
        );
        return Ok(());
    }
    // ... 其余不变
}
```

注意：`flush_aabb_cache` 仍然是全局函数，但**调用条件**完全由 batch 字段控制；mock backend 不需要 flush（不调全局）。

#### 4.3.3 `orchestrator.rs`

构造 `MeshResultBatch` 的两个调用点（行 ~655 和 ~739）增加：

```rust
.persist_mesh_results(MeshResultBatch {
    batch_id: batch.batch_id,
    mesh_results: &batch.mesh_results,
    mesh_aabb_map: &mesh_aabb_map,
    mesh_pts_map: &mesh_pts_map,
    file_mesh_state: use_file_mesh_state(),
})
```

`use_file_mesh_state` 在 orchestrator.rs 顶部 use 进来。

#### 4.3.4 `mock.rs`

```rust
async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
    self.record(format!(
        "persist_mesh_results:batch={},mesh_results={},file_mesh_state={}",
        batch.batch_id,
        batch.mesh_results.len(),
        batch.file_mesh_state
    ));
    Ok(())
}
```

### 4.4 风险

| 风险 | 处置 |
|---|---|
| `use_file_mesh_state()` 在调用 `persist_mesh_results` 之间发生变化（被其他线程改） | 实践中该状态在 process 级是固定的（启动时定）；改为 batch 字段后语义反而更稳 |
| 增加字段破坏 `MeshResultBatch` 字段顺序敏感的代码 | 字段顺序无敏感性（结构体）；构造点全部明确指定字段名 |

---

## 5. T3.4 — `Surreal + use_surrealdb=false` 早期拒绝

### 5.1 现状

`options.rs::validate_model_writer_features` 只校验 feature flag：

```rust
pub fn validate_model_writer_features(&self) -> anyhow::Result<()> {
    match self.model_writer_mode {
        ModelWriterMode::Surreal
            if !cfg!(any(
                feature = "write-to-surrealdb",
                feature = "surreal-save"
            )) =>
        {
            anyhow::bail!("model_writer=surreal 需要编译 feature ...")
        }
        ModelWriterMode::DrainOnly => Ok(()),
        _ => Ok(()),
    }
}
```

`Surreal + use_surrealdb=false` 这种**配置非法组合**要等到 `SurrealBackend::init` 才报错。

### 5.2 目标

在 `validate_model_writer_features` 增加运行时配置守卫：

```rust
pub fn validate_model_writer_features(&self) -> anyhow::Result<()> {
    // 1) feature flag 守卫
    match self.model_writer_mode {
        ModelWriterMode::Surreal
            if !cfg!(any(
                feature = "write-to-surrealdb",
                feature = "surreal-save"
            )) =>
        {
            anyhow::bail!(
                "model_writer=surreal 需要编译 feature `surreal-save` 或 `write-to-surrealdb`"
            );
        }
        _ => {}
    }
    // 2) 运行时配置组合守卫
    if matches!(self.model_writer_mode, ModelWriterMode::Surreal) && !self.use_surrealdb {
        anyhow::bail!(
            "model_writer=surreal 需要 use_surrealdb=true；当前 use_surrealdb=false 导致非法组合"
        );
    }
    Ok(())
}
```

### 5.3 实现差量

#### 5.3.1 `options.rs`

按 §5.2 修改 `validate_model_writer_features`。

#### 5.3.2 `surreal.rs`（保留 init 兜底）

```rust
async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()> {
    println!("[model-writer:surreal] stage=init ...");
    // 兜底守卫：理论上 options.validate_model_writer_features 已拒绝，
    // 这里二次校验防止绕开 validate 的代码路径。
    anyhow::ensure!(
        context.use_surrealdb,
        "Surreal model writer requires use_surrealdb=true (defense-in-depth)"
    );
    // ...
}
```

### 5.4 风险

| 风险 | 处置 |
|---|---|
| 现有 toml 配置含 `model_writer = "surreal"` + `use_surrealdb = false` 的"用例" | 极不可能（这种配置当前就跑不通）；如有，CHANGELOG 标 breaking note |
| `cli_modes.rs::run_regen_model` 可能在 validate 之前已开始操作 | 检查：`run_regen_model` 在创建 model_writer 前应先 `validate_model_writer_features`；已有则保留，没有则补 |

#### 5.4.1 验证命令

```powershell
rg -n "validate_model_writer_features" .worktrees/model-persistence-trait/src/
```

确认 `cli_modes.rs::run_regen_model` 与 `orchestrator.rs::process_index_tree_generation` 都调过。

---

## 6. P3 整体验证策略

按 `task_plan.md §6`，每个 Task 完成后跑：

```powershell
# 1) IDE lint
ReadLints on changed files
# 2) grep 验证 trait 接口纯度
rg -n "pdms_inst::SaveInstanceDataReport" .worktrees/model-persistence-trait/src/fast_model/gen_model/model_writer/
# 期望：只在 surreal.rs / drain_only.rs / mock.rs 内部出现，mod.rs 不出现

rg -n "use_surrealdb|defer_db_write" .worktrees/model-persistence-trait/src/fast_model/gen_model/model_writer/mod.rs
# 期望：只在 ModelWriterContext 内出现，BooleanBridgeRequest 已无

rg -n "use_file_mesh_state" .worktrees/model-persistence-trait/src/fast_model/gen_model/model_writer/
# 期望：surreal.rs 不再调用；orchestrator.rs 在构造 MeshResultBatch 时调用一次

# 3) 完整 build（P4 末尾统一跑）
pwsh docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1
```

---

## 7. P3 完成判定

- [ ] T3.1 完成：trait 接口不再返回 `SaveInstanceDataReport`，外部代码无引用
- [ ] T3.2 完成：`BooleanBridgeRequest` 字段精简为 3 个；backend 缓存 init context；调用方不再传 `use_surrealdb` / `defer_db_write`
- [ ] T3.3 完成：`MeshResultBatch` 加 `file_mesh_state` 字段；surreal backend 不再调全局 `use_file_mesh_state()`
- [ ] T3.4 完成：`validate_model_writer_features` 拒绝 `Surreal + use_surrealdb=false`；surreal backend 保留 ensure! 兜底
- [ ] mock.rs / verify binary / verify-mock.ps1 全部同步改完
- [ ] `progress.md` 4 行 status 全 complete

---

## 8. 与 P4 / P5 的依赖

P3 完成是 P4 推 PR 的前置条件（接口纯化是 review 阻塞项 W1/W2/W3/W4 的解决）。

P5 长期改进项（async fn in trait / const NAME / 清理空 mesh_results 包袱）独立立项，**不依赖 P3**；如果 P3 出现意外阻塞，P4 可以在仅完成 P1 + P2 + 部分 P3（至少 T3.1）的状态下推 PR，剩余 P3 Task 在后续 PR 续做。

---

## 9. 错误升级协议

按 `task_plan.md §7` 3-strike：

- 同一 Task 第 1 次失败：写入 `progress.md` 错误表，分析根因
- 第 2 次失败：换实现路径（如 OnceLock → RwLock 或反之）
- 第 3 次失败：升级用户决策；可能需要重新评估 trait 接口设计

---

## 10. 与 mock 同步的强约定（重申）

**每完成 P3 任一 Task，必须**：

1. 同步改 `mock.rs`（如方法签名变了）
2. 同步改 `verify_model_writer_trait` binary（如 fixture 构造或 snapshot 断言变了）
3. 跑 `pwsh verify-mock.ps1`（环境允许时）或在 `progress.md` 标"verify 留待 P4 末尾"

不允许只改 trait 接口不改 mock。这条违反 == P3 Task 未完成。
