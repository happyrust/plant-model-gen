# Model Write Trait — Review Findings

> 来源：2026-05-09 对 `feat/model-persistence-trait` (HEAD `b060860`) 的代码 review。
> 用途：作为 `task_plan.md` 的依据库；新发现的问题持续追加到本文件。

## 1. 当前实现摘要

### Trait 定义

位置：`src/fast_model/gen_model/model_writer.rs:154-179`

```rust
#[async_trait]
pub trait ModelWriteBackend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()>;
    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()>;
    async fn write_base_batch(&self, batch: BaseInstanceBatch<'_>) -> anyhow::Result<SaveInstanceDataReport>;
    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()>;
    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()>;
    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize>;
    async fn run_boolean_bridge(&self, request: BooleanBridgeRequest) -> anyhow::Result<BooleanBridgeReport>;
    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary>;
}
```

### 调用点矩阵

| 方法 | 调用位置 |
|---|---|
| `init` | `orchestrator.rs:958`（`process_index_tree_generation` 入口） |
| `cleanup` | `cli_modes.rs:1686`（`run_regen_model` 内 `if writes_to_surreal()` 守卫下） |
| `write_base_batch` | `orchestrator.rs:488`（`run_base_writer`） |
| `persist_mesh_results` | `orchestrator.rs:655` / `739`（`run_inst_aabb_writer` 双 select 分支） |
| `write_inst_relate_aabb` | `orchestrator.rs:669` / `752` |
| `reconcile_missing_neg` | `orchestrator.rs:1391` |
| `run_boolean_bridge` | `orchestrator.rs:1424`（DbLegacy）+ `1475`（MemoryTasks） |
| `finalize` | `orchestrator.rs:1720` |

### Backend 实现

- 唯一实现：`SurrealModelWriteBackend`（ZST，model_writer.rs:196）
- 工厂：`create_model_writer(db_option) -> anyhow::Result<Arc<dyn ModelWriteBackend>>`
  - `ModelWriterMode::Surreal` → `Arc::new(SurrealModelWriteBackend)`
  - `ModelWriterMode::DrainOnly` → `bail!("drain-only is an explicit non-persistent sink ...")`

### Regression 检查（已通过）

`pdms_inst::pre_cleanup_for_regen` / `pdms_inst_surreal::pre_cleanup_for_regen_surreal` / `pdms_inst::save_instance_data_with_report` / `pdms_inst::reconcile_missing_neg_relate` / `pdms_inst::build_inst_relate_aabb_rows` / `pdms_inst::save_inst_relate_aabb_rows` / `run_boolean_worker` / `run_bool_worker_from_tasks` 在 worktree 中**只在 `model_writer.rs` trait 实现内出现**，其他代码路径未绕过 trait 直调。

## 2. 问题清单

### Critical（阻碍发布 / 抽象未闭环）

#### C1. DrainOnly 二分法绕过 trait

**证据**：

```rust
// orchestrator.rs:952-962
let model_writer = if db_option.model_writer_mode == ModelWriterMode::DrainOnly {
    None
} else {
    let writer = create_model_writer(db_option)?;
    let writer_context = ModelWriterContext::from_db_option(db_option);
    writer.init(&writer_context).await?;
    Some(writer)
};
```

```rust
// model_writer.rs:187-191
ModelWriterMode::DrainOnly => {
    anyhow::bail!("drain-only is an explicit non-persistent sink ...")
}
```

**影响**：

- `Option<Arc<dyn ModelWriteBackend>>` 在 orchestrator / cli_modes 都要解包，或加 `if writes_to_surreal()` 守卫。
- 新 backend 加入需同时改工厂分支与所有调用方分支。
- factory 把"是否持久化"硬编码，trait 扩展性受限。

**对应 Task**：T1.2 + T1.3 + T1.4 + T1.5。

---

#### C2. 唯一 backend，trait 化收益未兑现

**证据**：worktree 内只有 `SurrealModelWriteBackend` 一个 impl，无 mock / no-op / mirror。

**影响**：

- trait 抽象等同于"过程式 → trait 包装"一次切片，多 backend 价值未兑现。
- 没有 mock，无法做契约测试，trait 演进时易破坏隐式约定。

**对应 Task**：T2.1 + T2.2 + T2.3。

---

### Warning（设计风险 / 长期债）

#### W1. trait 接口耦合泄漏

**证据**：

- `write_base_batch -> SaveInstanceDataReport`（来自 `pdms_inst::`）
- `BooleanBridgeRequest::db_option: Arc<aios_core::options::DbOption>`

**影响**：trait 与具体后端的内部数据强耦合，新 backend 必须兼容这些类型。

**对应 Task**：T3.1。

---

#### W2. `BooleanBridgeRequest` 字段冗余

**证据**：`use_surrealdb` / `defer_db_write` 已在 `ModelWriterContext`（init 时传入）；同一 backend 实例不应有两套语义，但接口允许。

**影响**：未来易出现 init context 与 request 字段不一致的 bug。

**对应 Task**：T3.2。

---

#### W3. `persist_mesh_results` 依赖全局状态

**证据**：

```rust
// model_writer.rs:269
async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
    if use_file_mesh_state() {
        flush_aabb_cache();
        ...
        return Ok(());
    }
    ...
}
```

**影响**：trait 行为依赖进程级全局开关，签名外不可见；mock backend 无法模拟 file mesh 模式。

**对应 Task**：T3.3。

---

#### W4. `init` 守卫时机偏晚

**证据**：

```rust
// model_writer.rs:212
anyhow::ensure!(
    context.use_surrealdb,
    "Surreal model writer requires use_surrealdb=true ..."
);
```

**影响**：非法组合（`Surreal + use_surrealdb=false`）等到 perf init / pre_check 之后才报错，浪费启动开销。

**对应 Task**：T3.4。

---

#### W5. 缺 mock / 契约测试

**证据**：worktree 无 `tests/`、无 mock backend、无契约断言代码。

**影响**：trait 抽象的最大好处之一未被利用；接口演进易破坏隐式约定。

**对应 Task**：T2.1 + T2.2。

---

#### W6. 分支未推送 + base 同改 `model_writer.rs`

**证据**：

- `git branch -a --contains b060860` 仅本地。
- 主仓库 `git status` 显示 `feat/collab-api-consolidation` 工作区有 `modified: src/fast_model/gen_model/model_writer.rs` 未提交。

**影响**：rebase / merge 必冲突；越拖越大。

**对应 Task**：T4.1 + T4.4。

---

### Note（小问题 / 改进建议）

#### N1. 命名不一致

`ModelWrite**Backend**` vs `ModelWriter**Mode**` / `model_writer_mode` / `ModelWriter**Context**`。**对应 Task**：T4.2。

#### N2. 文件过长（578 行）

`model_writer.rs` 集 trait + factory + Surreal 实现 + 私有 SQL helper + DrainOnlyStats + drain sink 于一身。**对应 Task**：T1.1。

#### N3. `async_trait` boxing 开销

hot path 上每个 batch 都过 `Pin<Box<dyn Future>>`，nightly 已支持原生 `async fn in trait`。**对应 Task**：T5.1（独立立项）。

#### N4. `name()` 仅在 `finalize` 用一次

可改为 `const NAME: &'static str`，让实现自存常量。**对应 Task**：T5.2。

#### N5. SQL 拼接缺显式 sanitize 层

`save_aabb_to_surreal_strict` / `save_pts_to_surreal_strict` 直接 `format!("aabb:⟨{}⟩", k)`。当前 key 是内部 hash，安全；但缺类型层守卫。**对应 Task**：T4.3。

#### N6. `write_base_batch` 内空 mesh_results 包袱

```rust
let mesh_results: HashMap<u64, MeshResult> = HashMap::new();
let report = pdms_inst::save_instance_data_with_report(..., &mesh_results, ...).await?;
```

为兼容旧 API 强行造空 map，trait 化未清理。**对应 Task**：T5.3（独立立项）。

#### N7. 日志风格

全 `println!("[model-writer:surreal] stage=...")`，无结构化 trace。**对应 Task**：未单独立项，可在 P4 顺手做。

## 3. 待回答的设计问题（开干前需确认）

| 问题 | 备选 | 暂定 |
|---|---|---|
| `RecordingBackend` 是否进 release 编译？ | (a) `#[cfg(test)]`<br>(b) feature flag `model-writer-mock` | **b**：方便用 binary 验证而不需 cargo test |
| trait 命名最终值 | (a) `ModelWriterBackend`<br>(b) `ModelWriter`<br>(c) 保持 `ModelWriteBackend` | **a**：与 `ModelWriterMode` 对齐 |
| DrainOnly 是否完全删除独立的 `run_drain_only_sink`？ | (a) 删，逻辑搬进 backend<br>(b) 保留，作为 backend 内部子流程 | **a**：彻底统一 |
| `BooleanBridgeRequest::db_option` 是否也精简掉？ | (a) 保留，因为 boolean worker 真要用 DbOption<br>(b) 改为 trait method 注入更小的 BridgeContext | T3.2 配套讨论，倾向 **a**（短期保留） |

## 4. 新增发现（开干后追加）

> 此节随实施进度追加；每条注明发现日期 + 关联 Task。

（暂无）
