# Model Generation Optimization Performance Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 以“性能优先”为宗，消弭模型生成输入批处理（LOOP/PRIM）中的 N+1/重复 IO 与过量并发开销，重点优化 `neg_refnos`（及可选 `cmpf_neg_refnos`）阶段。

**Architecture:** 在不重写流水线之下，于 `input_cache_pipeline` 侧引入“按 dbnum 分组、单次加载 TreeIndex、按 root 产出映射”的查询函数；避免每个 refno 调一次 `query_provider::query_multi_descendants_with_self(&[r], ...)` 所导致的重复读取配置文件/重复构造过滤器/过度 task 调度。并以最小可观测性（阶段耗时 + 计数）护航。

**Tech Stack:** Rust, tokio, DashMap/Lazy, TreeIndex（`.tree`），DbMetaManager（`db_meta_info.json`），现有 perf 脚本（PowerShell）。

---

## Scope / Non-Goals

- In-scope:
  - `fetch_loop_inputs_map_batch` / `fetch_prim_inputs_map_batch` 的 `neg_refnos` 查询改为“按 dbnum 分组、复用 TreeIndex/过滤器”。
  - （可选）`cmpf_neg_refnos` 同路优化，避免多次 TreeIndex 解析路径。
  - 在 batch 内增加阶段级耗时日志（可用 env 开关）。
- Out-of-scope（后续可另立案）:
  - 将 `aios_core::fetch_loops_and_height` 改造为真正的批量 API（涉及 rs-core，工作量较大）。
  - 大规模重构 `query_provider` 的接口语义（仅做最小新增，不破坏现有调用方）。

---

## Observed Hotspots (Why)

### 1) `neg_refnos` 的 per-refno 调用导致重复开销

在 `src/fast_model/gen_model/input_cache_pipeline.rs` 中：
- `fetch_loop_inputs_map_batch` 与 `fetch_prim_inputs_map_batch` 皆以 `buffer_unordered(32)` 逐 refno 调用：
  - `query_provider::query_multi_descendants_with_self(&[r], &GENRAL_NEG_NOUN_NAMES, false)`

而 `query_multi_descendants_with_self` 内部会：
- 通过 `TreeIndexManager::with_default_dir(Vec::new())` 计算 `tree_dir`（会读取配置文件解析 project_name）。
- 计算 `noun_hashes` 并构造 `TreeQueryOptions`。
- 逐 root 解析 dbnum 并做 BFS 收集。

当 refnos 数量较大时，此路径易出现：重复读取配置文件、重复构造过滤器、过量 task 调度、锁竞争（db_meta/RwLock、DashMap）。

---

## Acceptance Criteria

- (A) `neg_refnos` 在 batch 内：
  - `tree_dir` 仅计算一次（不随 refno 反复读取配置）。
  - `noun_hashes`/`TreeQueryOptions` 仅构造一次或按 dbnum 复用。
  - 每个 dbnum 的 `.tree` 最多加载一次（复用全局缓存即可）。
- (B) 功能不变：对任意 root，产出的 `neg_refnos` 集合与旧逻辑一致（至多顺序差异，但建议保持 BFS 输出顺序）。
- (C) 可观测性：在 `AIOS_GEN_INPUT_CACHE_STAGE_TIMING=1` 时输出 batch 各阶段耗时（attmap/world/loops/neg/cmpf_neg/total）。
- (D) 在现有脚本 `scripts/perf_test_7997_pane.ps1` 上可见总耗时下降，且 perf JSON 中相关阶段耗时下降（以实际数据为准）。

---

## Implementation Plan, Task List and Thought in Chinese

### Task 1: 为“按 dbnum 分组的 neg 映射查询”新增独立函数（仅计划，不改现有调用）

**Files:**
- Create: `src/fast_model/gen_model/neg_query.rs`
- Modify: `src/fast_model/gen_model/mod.rs`（若需导出 module）
- Test: `src/fast_model/gen_model/neg_query.rs`（同文件单元测试：纯分组逻辑）

**Step 1: Write the failing test**

在 `src/fast_model/gen_model/neg_query.rs` 新增一个纯函数（先写测试）：
- `group_by_dbnum(refnos, resolver)`：给定 refnos 与 resolver（闭包/trait），返回 `HashMap<u32, Vec<RefnoEnum>>`。

示例测试（无需真实 tree 文件）：
```rust
#[test]
fn test_group_by_dbnum_keeps_roots() {
    use std::collections::HashMap;
    use aios_core::RefnoEnum;

    let r1: RefnoEnum = "24381/1".into();
    let r2: RefnoEnum = "24381/2".into();
    let r3: RefnoEnum = "9304/3".into();

    let mut m = HashMap::<u64, u32>::new();
    m.insert(r1.refno().0, 1112);
    m.insert(r2.refno().0, 1112);
    m.insert(r3.refno().0, 7997);

    let grouped = super::group_by_dbnum(&[r1, r2, r3], |r| Ok(*m.get(&r.refno().0).unwrap()));
    let grouped = grouped.unwrap();
    assert_eq!(grouped.get(&1112).unwrap().len(), 2);
    assert_eq!(grouped.get(&7997).unwrap().len(), 1);
}
```

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q test_group_by_dbnum_keeps_roots
```
Expected: FAIL（函数/模块尚未实现）。

**Step 3: Write minimal implementation**

实现 `group_by_dbnum`（纯逻辑，保持输入顺序，避免去重以免改变行为；去重交由后续每 root 内部去重）。

**Step 4: Run test to verify it passes**

Run:
```bash
cargo test -q test_group_by_dbnum_keeps_roots
```
Expected: PASS。

**Step 5: Commit**
```bash
git add src/fast_model/gen_model/neg_query.rs src/fast_model/gen_model/mod.rs
git commit -m "feat(gen-model): add grouped neg query helper (no behavior change yet)"
```

---

### Task 2: 实现 `query_descendants_map_by_dbnum`（root -> Vec<desc>）

**Files:**
- Modify: `src/fast_model/gen_model/neg_query.rs`

**Step 1: Write the failing test**

此处难以在单测中构造真实 `.tree`，故以“编译期 + 行为契约”测试为主：
- 新增一个 `#[test]` 仅校验函数签名可用、输入为空返回空 map。

```rust
#[test]
fn test_query_descendants_map_empty() {
    use std::path::PathBuf;
    use aios_core::RefnoEnum;
    let tree_dir = PathBuf::from("output/does-not-matter");
    let m = super::query_descendants_map_by_dbnum(&tree_dir, &[] as &[RefnoEnum], &["FOO"], false);
    assert!(m.unwrap().is_empty());
}
```

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q test_query_descendants_map_empty
```
Expected: FAIL（函数尚未实现）。

**Step 3: Write minimal implementation**

实现逻辑（要点）：
- `tree_dir` 由调用方传入（来自 `DbOptionExt::get_scene_tree_dir()`），不再每 root 反复读配置文件。
- 对 roots 先调用 `TreeIndexManager::resolve_dbnum_for_refno(root).await?` 做 dbnum 分组（可并发：按 dbnum 数并发，而非按 root）。
- 每个 dbnum：
  - `load_index_with_large_stack(&tree_dir, dbnum)` 取 `Arc<TreeIndex>`（命中全局缓存则不建线程）。
  - 预先计算 noun_hashes（`db1_hash`）与 `TreeQueryOptions`，对该 dbnum 组内所有 root 复用。
  - 对每 root 调 `index.collect_descendants_bfs(root.refno(), &options)`，并做：
    - `RefnoEnum::from(u64)`，过滤 `is_valid()`；
    - per-root `seen` 去重但保持顺序。
- 返回 `HashMap<RefnoEnum, Vec<RefnoEnum>>`。

**Step 4: Run test to verify it passes**

Run:
```bash
cargo test -q test_query_descendants_map_empty
```
Expected: PASS。

**Step 5: Commit**
```bash
git add src/fast_model/gen_model/neg_query.rs
git commit -m "feat(gen-model): add per-root descendants map query (grouped by dbnum)"
```

---

### Task 3: 替换 LOOP batch 的 `neg_refnos` 实现为“映射查询”

**Files:**
- Modify: `src/fast_model/gen_model/input_cache_pipeline.rs`

**Step 1: Write the failing test**

此处以“编译 + 基准脚本验证”为主；先加一个最小编译型断言（避免漏引模块）：
- 为 `fetch_loop_inputs_map_batch` 增加一个私有 helper 调用点，确保能编译通过。

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q
```
Expected: FAIL（若未正确引入新模块/函数）。

**Step 3: Write minimal implementation**

在 `fetch_loop_inputs_map_batch` 中：
- 用 `let tree_dir = db_option.get_scene_tree_dir();` 获取 tree 目录（一次）。
- 调 `neg_query::query_descendants_map_by_dbnum(&tree_dir, refnos, &GENRAL_NEG_NOUN_NAMES, false)` 获取 `neg_map`。
- 移除 per-refno `stream::iter(...).buffer_unordered(NEG_CONCURRENCY)` 的逻辑。

同时（建议）：
- 用 env `AIOS_GEN_INPUT_CACHE_STAGE_TIMING=1` 打印 `neg_refnos_ms` 与 `total_ms`，便于对比优化收益。

**Step 4: Run tests to verify**

Run:
```bash
cargo test -q
```
Expected: PASS。

**Step 5: Commit**
```bash
git add src/fast_model/gen_model/input_cache_pipeline.rs
git commit -m "perf(gen-model): batch neg_refnos via grouped TreeIndex query (loop inputs)"
```

---

### Task 4: 替换 PRIM batch 的 `neg_refnos` 实现为“映射查询”

**Files:**
- Modify: `src/fast_model/gen_model/input_cache_pipeline.rs`

**Step 1: Implementation**

同 Task 3，于 `fetch_prim_inputs_map_batch` 替换 `neg_refnos` 获取逻辑。

**Step 2: Verify**

Run:
```bash
cargo test -q
```
Expected: PASS。

**Step 3: Commit**
```bash
git add src/fast_model/gen_model/input_cache_pipeline.rs
git commit -m "perf(gen-model): batch neg_refnos via grouped TreeIndex query (prim inputs)"
```

---

### Task 5 (Optional): 优化 `cmpf_neg_refnos`（避免二次/多次 TreeIndex 解析路径）

**Files:**
- Modify: `src/fast_model/gen_model/input_cache_pipeline.rs`
- (Optional) Modify: `src/fast_model/gen_model/neg_query.rs`

**Approach A (KISS, 低风险):**
- 先保留当前逻辑，只把 `get_descendants_by_types` 与 `query_multi_descendants` 的 tree_dir 读取/构造过滤器部分消掉：
  - 新增 `neg_query::query_descendants_vec(tree_dir, roots, nouns)` 之类 helper，接入同一套 tree_dir 与 noun_hashes 复用。

**Approach B (更快, 但更复杂):**
- per root：先取 CMPF descendants（noun= CMPF），再对这些 CMPF roots 批量取 neg descendants，并聚合为 cmpf_neg_refnos。
- 适用于 CMPF 数量很少的场景；若 CMPF 众多，仍可能偏慢。

**Verification:**
- 仅在 `AIOS_OPT_CMPF_NEG=1` 开启（避免行为变化难追）。

---

## Verification (Manual / Perf)

### 1) Debug 模式性能对比（推荐）

Run:
```powershell
.\scripts\perf_test_7997_pane.ps1
```

观测：
- `output/YCYK-E3D/profile/perf_report_pane_*.md` 中 total 与各阶段耗时。
- 重点看与 `neg_refnos` 相关阶段是否下降（若已加入 stage timing，则对比更直观）。

### 2) 功能回归（结构统计）

Run:
```powershell
.\scripts\compare_cacheonly_vs_now.ps1
```

观测：
- `cache_flush` 的结构统计（inst_info/inst_geos/neg 等）是否异常波动。

