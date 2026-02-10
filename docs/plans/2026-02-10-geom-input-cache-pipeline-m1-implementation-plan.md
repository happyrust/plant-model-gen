# Geom Input Cache Pipeline (M1) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在 Full Noun 模式下，为 LOOP/PRIM 引入“写入 Foyer geom_input_cache 后再发 Key”的流水线，使预取（IO）与几何生成（CPU）重叠执行。

**Architecture:** 在 `full_noun_mode` 中，若启用 pipeline 开关，则对 LOOP/PRIM 分别启动：prefetcher（按 dbnum+chunk 获取输入 -> 异步写 cache -> 发送 key）与 consumer（按 key 从 cache 读取 batch -> 调用 `gen_*_geos_from_cache` 生成）。保留旧行为作为 fallback。

**Tech Stack:** Rust (tokio, flume), Foyer HybridCache, SurrealDB queries (现阶段仍复用逐 refno 查询), aios_core types.

---

## Preconditions / Notes
- 本计划只实现 **M1 骨架**：不强行改批量 SurrealQL（M2 再做）。
- Pipeline 仅在 `AIOS_GEN_INPUT_CACHE=1` 且 `AIOS_GEN_INPUT_CACHE_ONLY!=1` 且 `AIOS_GEN_INPUT_CACHE_PIPELINE=1` 时启用；否则走旧路径。
- `geom_input_cache` 全局初始化已在 `src/fast_model/gen_model/orchestrator.rs` 执行（按环境变量启用）。

---

### Task 1: 新增 Pipeline 开关函数（env var）

**Files:**
- Modify: `src/fast_model/foyer_cache/geom_input_cache.rs`
- Test: `src/fast_model/foyer_cache/geom_input_cache.rs`（同文件 tokio test）

**Step 1: Write the failing test**

```rust
#[test]
fn test_is_geom_input_cache_pipeline_enabled() {
    std::env::remove_var("AIOS_GEN_INPUT_CACHE_PIPELINE");
    assert!(!crate::fast_model::foyer_cache::geom_input_cache::is_geom_input_cache_pipeline_enabled());

    std::env::set_var("AIOS_GEN_INPUT_CACHE_PIPELINE", "1");
    assert!(crate::fast_model::foyer_cache::geom_input_cache::is_geom_input_cache_pipeline_enabled());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q test_is_geom_input_cache_pipeline_enabled`
Expected: FAIL (function not found)

**Step 3: Write minimal implementation**

在 `src/fast_model/foyer_cache/geom_input_cache.rs` 增加：

```rust
pub fn is_geom_input_cache_pipeline_enabled() -> bool {
    std::env::var("AIOS_GEN_INPUT_CACHE_PIPELINE")
        .ok()
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q test_is_geom_input_cache_pipeline_enabled`
Expected: PASS

**Step 5: Commit**

```bash
git add src/fast_model/foyer_cache/geom_input_cache.rs
git commit -m "feat: add geom_input_cache pipeline toggle"
```

---

### Task 2: 将 prefetch_* 重构为“fetch inputs”与“write batch”两段（为并行铺路）

**Files:**
- Modify: `src/fast_model/foyer_cache/geom_input_cache.rs`
- Test: `src/fast_model/foyer_cache/geom_input_cache.rs`

**Step 1: Write the failing test**

目标：可以在不触发 SurrealDB 查询的情况下，直接写入一个 loop/prim batch 并按 batch_id 读回。

```rust
#[tokio::test]
async fn test_insert_and_get_single_batch_roundtrip() {
    use crate::fast_model::foyer_cache::geom_input_cache::*;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let mgr = GeomInputCacheManager::new(dir.path()).await.unwrap();

    let refno: aios_core::RefnoEnum = "24381/36716".into();

    // NamedAttrMap / Transform / PdmsGenericType 均应有 Default（若编译失败，改为项目已有的最小构造方式）。
    let loop_input = LoopInput {
        refno,
        attmap: aios_core::NamedAttrMap::default(),
        world_transform: bevy_transform::prelude::Transform::IDENTITY,
        loops: Vec::new(),
        height: 0.0,
        owner_refno: refno,
        owner_type: "TEST".to_string(),
        visible: true,
        generic_type: aios_core::pdms_types::PdmsGenericType::default(),
        neg_refnos: Vec::new(),
        cmpf_neg_refnos: Vec::new(),
    };

    let mut loop_inputs = std::collections::HashMap::new();
    loop_inputs.insert(refno, loop_input);

    let batch_id = mgr.next_batch_id(24381);
    mgr.insert_batch(GeomInputBatch {
        dbnum: 24381,
        batch_id: batch_id.clone(),
        created_at: chrono::Utc::now().timestamp_millis(),
        loop_inputs,
        prim_inputs: std::collections::HashMap::new(),
    });

    let got = mgr.get(24381, &batch_id).await.expect("batch must exist");
    assert!(got.loop_inputs.contains_key(&refno));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q test_insert_and_get_single_batch_roundtrip`
Expected: FAIL（通常因为某些类型的 Default/构造不匹配）

**Step 3: Write minimal implementation / adjustments**

- 若 `PdmsGenericType::default()` 不存在，则改为该枚举的“未知/默认”变体（以编译通过为准）。
- 若 `NamedAttrMap::default()` 不在当前 crate re-export 下，改为正确路径（以编译错误提示修正）。

**Step 4: Run test to verify it passes**

Run: `cargo test -q test_insert_and_get_single_batch_roundtrip`
Expected: PASS

**Step 5: Commit**

```bash
git add src/fast_model/foyer_cache/geom_input_cache.rs
git commit -m "test: add geom_input_cache batch roundtrip test"
```

---

### Task 3: 新增 LOOP/PRIM 流水线 runner（prefetch -> write -> key -> consume）

**Files:**
- Create: `src/fast_model/gen_model/input_cache_pipeline.rs`
- Modify: `src/fast_model/gen_model/mod.rs`（若需要挂载模块）
- Modify: `src/fast_model/gen_model/full_noun_mode.rs`
- Test: `src/fast_model/gen_model/input_cache_pipeline.rs`

**Step 1: Write the failing test**

写一个不依赖 SurrealDB 的“伪 prefetch”测试：给定两个 batch payload，writer 写入 cache 并发 key，consumer 能逐 key 读出并统计处理数量。

```rust
#[tokio::test]
async fn test_pipeline_key_driven_consume_smoke() {
    // NOTE: 这里不调用真实 fetch_*；只验证“写入->发 key->按 key 读回”的链路。
    // 期望：consumer 收到 2 个 key，并能从 cache get 到对应 batch。
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q test_pipeline_key_driven_consume_smoke`
Expected: FAIL（模块/函数不存在）

**Step 3: Write minimal implementation**

在 `src/fast_model/gen_model/input_cache_pipeline.rs` 实现：

- `struct ReadyBatchKey { dbnum: u32, batch_id: String }`
- `async fn run_loop_pipeline_from_refnos(...) -> anyhow::Result<()>`
- `async fn run_prim_pipeline_from_refnos(...) -> anyhow::Result<()>`

prefetcher 逻辑（M1 仍复用逐 refno 查询的 fetch helper）：
- 按 dbnum 分组（复用 `db_meta().get_dbnum_by_refno`）
- 按 `chunk_size = config.batch_size.get()` 切块
- 使用 `Semaphore` 限制 in-flight 写入任务（建议默认 4）
- 每 chunk：fetch inputs map（await）后 `tokio::spawn` 写入 cache（可把 serde 放到 spawn_blocking），写完后 `tx.send(key)`

consumer 逻辑：
- `while let Ok(key) = rx.recv_async().await { let batch = cache.get(key.dbnum, &key.batch_id).await; gen_*_geos_from_cache(...) }`

full_noun_mode 集成点（替换原先“先 prefetch_all 再正常生成”的路径）：
- 若 `AIOS_GEN_INPUT_CACHE_PIPELINE=1` 且 cache enabled 且非 cache-only：
  - LOOP 阶段：启动 loop pipeline（prefetch+consume），等待完成
  - PRIM 阶段：启动 prim pipeline（prefetch+consume），等待完成
- 若 pipeline 失败：打印错误并回退到旧 `process_*_refno_page` 路径

**Step 4: Run test to verify it passes**

Run: `cargo test -q test_pipeline_key_driven_consume_smoke`
Expected: PASS

**Step 5: Commit**

```bash
git add src/fast_model/gen_model/input_cache_pipeline.rs src/fast_model/gen_model/full_noun_mode.rs
git commit -m "feat: add key-driven geom input pipeline for loop/prim"
```

---

### Task 4: 在 Full Noun 中启用 pipeline（保持兼容回退）

**Files:**
- Modify: `src/fast_model/gen_model/full_noun_mode.rs`

**Step 1: Write a small regression test (optional)**

若该模块已存在大量集成测试困难，可跳过自动化测试，改为手工验证脚本（见 Step 3）。

**Step 2: Implement guarded integration**

- 在 LOOP/PRIM 阶段进入点增加：
  - `if geom_input_cache::is_geom_input_cache_enabled() && geom_input_cache::is_geom_input_cache_pipeline_enabled() && !geom_input_cache::is_geom_input_cache_only()`
- pipeline 成功：不再调用 `process_loop_refno_page/process_prim_refno_page`（避免重复生成）
- pipeline 失败：回退原有路径

**Step 3: Manual verification**

在 PowerShell 中跑一次小范围 Full Noun：

```powershell
$env:DB_OPTION_FILE = 'db_options/DbOption-tmpcache'
$env:AIOS_GEN_INPUT_CACHE = '1'
$env:AIOS_GEN_INPUT_CACHE_PIPELINE = '1'
Remove-Item Env:AIOS_GEN_INPUT_CACHE_ONLY -ErrorAction SilentlyContinue

cargo run --bin aios-database -- --gen-model
```

预期现象：
- 日志出现 pipeline 启用提示（新增日志）
- LOOP/PRIM 在 pipeline 模式下运行
- 生成阶段不再等待“全量 prefetch 完成”才开始（可从日志时间戳观察重叠）

**Step 4: Commit**

```bash
git add src/fast_model/gen_model/full_noun_mode.rs
git commit -m "refactor: use loop/prim input cache pipeline in full noun"
```

---

### Task 5: 端到端验证与回滚开关说明

**Files:**
- Modify: `docs/plans/2026-02-10-foyer-pipeline-design.md`（追加运行开关说明，若需要）

**Step 1: Add README-style notes**

补充说明：
- `AIOS_GEN_INPUT_CACHE=1`：启用 geom_input_cache（预取/缓存能力）
- `AIOS_GEN_INPUT_CACHE_PIPELINE=1`：启用 M1 pipeline（写 cache 后发 key 并行消费）
- `AIOS_GEN_INPUT_CACHE_ONLY=1`：严格 cache-only（此时 pipeline 不应运行，直接读已有 cache）

**Step 2: Verification commands**

Run:
- `cargo test`
- 任选一个小 refno 范围跑 Full Noun（同 Task 4 手工验证）

**Step 3: Commit**

```bash
git add docs/plans/2026-02-10-foyer-pipeline-design.md
git commit -m "docs: document geom_input_cache pipeline env vars"
```

---

## Execution Handoff

Plan complete and saved to `docs/plans/2026-02-10-geom-input-cache-pipeline-m1-implementation-plan.md`. Two execution options:

1. Subagent-Driven (this session) - I dispatch fresh subagent per task, review between tasks, fast iteration

2. Parallel Session (separate) - Open new session with executing-plans, batch execution with checkpoints

Which approach?

