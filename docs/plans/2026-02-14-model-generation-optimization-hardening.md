# Model Generation Optimization Hardening Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为“模型生成优化路径（foyer cache-first + 批量查询）”补齐正确性护栏、缓存隔离/失效策略、dbnum 映射一致性与可观测性，避免快而错/跨目录串 cache/静默退化。

**Architecture:** 在不重写主流程的前提下，围绕 `transform_cache`/`geom_input_cache`/`db_meta` 三条关键链路做最小侵入加固：引入 cache scope（目录级隔离）、为全局 OnceCell 增加目录一致性断言、统一 dbnum 推导只认本仓 db_meta_manager、并在关键阶段输出 hit/miss/fallback 统计。

**Tech Stack:** Rust（aios-database 单 crate）、tokio、foyer HybridCache、SurrealDB client（aios_core::SUL_DB）、serde/serde_json、once_cell/tokio::sync::OnceCell。

---

## Scope / Non-Goals

- In-scope:
  - `transform_cache` 目录隔离与 OnceCell 目录锁定风险控制
  - `geom_input_cache` 同类目录锁定风险控制
  - `dbnum` 推导路径统一（严守 ref0 != dbnum）
  - 关键路径的最小可观测性（统计 + 采样日志）
  - `precheck` 中可能导致性能回退的“全量刷新”行为降级为显式开关
- Out-of-scope:
  - 将 loops/neg_refnos 全面改为真正 batch SQL（可作为后续性能任务）
  - 重构 aios_core 内部 world_transform 惰性算法（可作为单独方案 B）

---

## Acceptance Criteria

- (A) 同一进程内若尝试以不同 cache_dir 初始化全局 `transform_cache`/`geom_input_cache`，会 **明确报错**（或显式拒绝切换），不再静默串目录。
- (B) 默认 `transform_cache`（以及 `geom_input_cache` 若适用）按 **cache scope** 隔离（至少区分 `project_name` + `target_sesno`/“latest”），避免历史会话复用错误缓存。
- (C) 所有“refno -> dbnum” 推导均遵循本仓 `db_meta_manager` 映射，**禁止**用 ref0 兜底；映射缺失时行为可配置（报错/跳过）且有日志。
- (D) `get_world_transforms_cache_first_batch` 输出（按 env 开关）可看到 `cache_hit/cache_miss/db_batch_hit/fallback_count/still_missing` 等统计，方便判断优化是否生效。
- (E) `precheck::ensure_pe_transform_for_refnos` 不再默认触发潜在“按 dbnum 全量刷新”，除非显式开启开关。

---

### Task 1: 为 transform_cache 引入 cache scope（目录级隔离）

**Files:**
- Modify: `src/options.rs`
- Modify: `src/fast_model/transform_cache.rs`
- Test: `tests/transform_cache_scope_test.rs` (new)

**Step 1: Write the failing test**

Create `tests/transform_cache_scope_test.rs`:

```rust
use aios_database::options::DbOptionExt;
use aios_core::options::DbOption;

#[test]
fn transform_cache_dir_includes_project_and_sesno_scope() {
    let mut base = DbOption::default();
    base.project_name = "AvevaMarineSample".to_string();
    let mut ext = DbOptionExt::from(base);
    ext.target_sesno = Some(123);

    let dir = aios_database::fast_model::transform_cache::transform_cache_dir_for_option(&ext);
    let s = dir.to_string_lossy();
    assert!(s.contains("AvevaMarineSample"));
    assert!(s.contains("transform_cache"));
    assert!(s.contains("sesno_123"));
}
```

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q transform_cache_dir_includes_project_and_sesno_scope
```

Expected: FAIL（当前目录不包含 sesno scope）。

**Step 3: Write minimal implementation**

- 在 `src/options.rs` 增加一个小函数（或 `impl DbOptionExt` 方法）用于构造 cache scope 字符串：
  - `sesno_<n>`（当 `target_sesno=Some(n)`）
  - 否则 `latest`
  - 允许 env `AIOS_CACHE_SCOPE` 覆盖（用于调试/回滚）。
- 在 `src/fast_model/transform_cache.rs` 的 `transform_cache_dir_for_option` 改为：
  - `db_option.get_foyer_cache_dir().join("transform_cache").join(scope)`

**Step 4: Run test to verify it passes**

Run:
```bash
cargo test -q transform_cache_dir_includes_project_and_sesno_scope
```
Expected: PASS

**Step 5: Commit**
```bash
git add src/options.rs src/fast_model/transform_cache.rs tests/transform_cache_scope_test.rs
git commit -m "fix(transform-cache): add sesno/project scoped cache dir"
```

---

### Task 2: 为 transform_cache 的 OnceCell 增加目录一致性断言（防串目录）

**Files:**
- Modify: `src/fast_model/transform_cache.rs`
- Test: `tests/transform_cache_oncecell_guard_test.rs` (new)

**Step 1: Write the failing test**

Create `tests/transform_cache_oncecell_guard_test.rs`:

```rust
use aios_database::options::DbOptionExt;
use aios_core::options::DbOption;

#[tokio::test]
async fn transform_cache_rejects_different_cache_dir_in_same_process() {
    let mut opt1 = DbOption::default();
    opt1.project_name = "P1".to_string();
    let ext1 = DbOptionExt::from(opt1);

    let mut opt2 = DbOption::default();
    opt2.project_name = "P2".to_string();
    let ext2 = DbOptionExt::from(opt2);

    aios_database::fast_model::transform_cache::init_global_transform_cache(&ext1)
        .await
        .unwrap();

    let r = aios_database::fast_model::transform_cache::init_global_transform_cache(&ext2).await;
    assert!(r.is_err(), "must reject switching cache dir");
}
```

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q transform_cache_rejects_different_cache_dir_in_same_process
```
Expected: FAIL（当前会静默复用第一次目录）。

**Step 3: Write minimal implementation**

在 `src/fast_model/transform_cache.rs` 内新增一个全局 OnceCell 记录已初始化的目录（PathBuf）：
- 首次 init：记录 dir
- 再次 init：若 dir 不同，`anyhow::bail!`，错误信息包含 old/new 路径

**Step 4: Run test to verify it passes**

Run:
```bash
cargo test -q transform_cache_rejects_different_cache_dir_in_same_process
```
Expected: PASS

**Step 5: Commit**
```bash
git add src/fast_model/transform_cache.rs tests/transform_cache_oncecell_guard_test.rs
git commit -m "fix(transform-cache): guard OnceCell against dir switching"
```

---

### Task 3: 为 geom_input_cache 增加同类 OnceCell 目录一致性断言

**Files:**
- Modify: `src/fast_model/foyer_cache/geom_input_cache.rs`
- Test: `tests/geom_input_cache_oncecell_guard_test.rs` (new)

**Step 1: Write the failing test**

Create `tests/geom_input_cache_oncecell_guard_test.rs`（按与 Task2 同样模式，初始化两套不同 project 目录应报错）。

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q geom_input_cache
```
Expected: FAIL

**Step 3: Write minimal implementation**

同 Task2：增加 OnceCell 记录 dir 并断言一致。

**Step 4: Run test to verify it passes**

Run:
```bash
cargo test -q geom_input_cache
```
Expected: PASS

**Step 5: Commit**
```bash
git add src/fast_model/foyer_cache/geom_input_cache.rs tests/geom_input_cache_oncecell_guard_test.rs
git commit -m "fix(geom-input-cache): guard OnceCell against dir switching"
```

---

### Task 4: 统一 dbnum 推导，移除不确定的 aios_core fallback

**Files:**
- Modify: `src/fast_model/db_meta_cache.rs`
- Modify: `src/fast_model/gen_model/tree_index_manager.rs`
- Modify: `src/fast_model/gen_model/utilities.rs`
- Test: `tests/dbnum_resolution_policy_test.rs` (new)

**Step 1: Write the failing test**

Create `tests/dbnum_resolution_policy_test.rs`:

```rust
use aios_database::fast_model::db_meta_cache;
use aios_core::RefnoEnum;

#[test]
fn db_meta_cache_does_not_guess_dbnum_from_ref0() {
    // 选择一个任意 refno：在未加载 db_meta_info.json 的情况下应该返回 None。
    let r = RefnoEnum::from("24381/145018");
    let dbnum = db_meta_cache::get_dbnum_for_refno(r);
    assert!(dbnum.is_none());
}
```

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q db_meta_cache_does_not_guess_dbnum_from_ref0
```
Expected: FAIL（当前实现调用 aios_core::get_dbnum_by_refno，行为不可控）。

**Step 3: Write minimal implementation**

- 将 `src/fast_model/db_meta_cache.rs::get_dbnum_for_refno` 改为：
  - `crate::data_interface::db_meta_manager::db_meta().ensure_loaded().ok();`
  - `crate::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(refno)`
  - 若未加载或无映射 -> `None`
- 同时在 `TreeIndexManager::resolve_dbnum_for_refno` 与 `utilities.rs` 中：
  - 删除/弱化 `db_meta_cache` fallback（或仅作同源映射的薄封装）
  - 映射缺失时统一报错信息指向 `output/<project>/scene_tree/db_meta_info.json`

**Step 4: Run test to verify it passes**

Run:
```bash
cargo test -q db_meta_cache_does_not_guess_dbnum_from_ref0
```
Expected: PASS

**Step 5: Commit**
```bash
git add src/fast_model/db_meta_cache.rs src/fast_model/gen_model/tree_index_manager.rs src/fast_model/gen_model/utilities.rs tests/dbnum_resolution_policy_test.rs
git commit -m "fix(dbnum): enforce db_meta mapping only (no ref0 fallback)"
```

---

### Task 5: 为 world_transform batch 路径增加可观测性（统计输出）

**Files:**
- Modify: `src/fast_model/transform_cache.rs`
- Modify: `src/fast_model/gen_model/input_cache_pipeline.rs` (可选：打印阶段耗时)
- Test: `tests/transform_cache_stats_test.rs` (new, unit-level)

**Step 1: Write the failing test**

Create `tests/transform_cache_stats_test.rs`（纯单元测试，不连 DB）：
- 目标：验证统计结构存在且在 mock 路径可递增（建议将统计逻辑提取成 `pub(crate)` helper）。

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q transform_cache_stats
```
Expected: FAIL（当前无统计结构）。

**Step 3: Write minimal implementation**

- 在 `transform_cache.rs` 增加 `TransformFetchStats`（本次调用内局部统计）：
  - `total, cache_hit, cache_miss, db_rows, fallback_ok, fallback_fail, still_missing`
- 通过 env `AIOS_TRANSFORM_CACHE_STATS=1` 控制输出：
  - 仅在 batch 结束打印一行汇总（避免刷屏）

**Step 4: Run test to verify it passes**

Run:
```bash
cargo test -q transform_cache_stats
```
Expected: PASS

**Step 5: Commit**
```bash
git add src/fast_model/transform_cache.rs src/fast_model/gen_model/input_cache_pipeline.rs tests/transform_cache_stats_test.rs
git commit -m "feat(transform-cache): add batch stats for observability"
```

---

### Task 6: 降级 precheck 的 pe_transform “按 dbnum 刷新”为显式开关

**Files:**
- Modify: `src/fast_model/precheck.rs`
- Modify: `src/fast_model/gen_model/precheck_coordinator.rs` (可选：文案更正)
- Test: `tests/precheck_refresh_gate_test.rs` (new)

**Step 1: Write the failing test**

Create `tests/precheck_refresh_gate_test.rs`：
- 在默认情况下调用 `ensure_pe_transform_for_refnos` 不应触发 refresh（可通过注入/封装刷新函数指针实现可测）。

**Step 2: Run test to verify it fails**

Run:
```bash
cargo test -q precheck_refresh
```
Expected: FAIL（当前会直接调用 refresh_pe_transform_for_dbnums）。

**Step 3: Write minimal implementation**

- 新增 env：`AIOS_REFRESH_PE_TRANSFORM=1` 才启用刷新；否则直接返回 Ok 并打印一次性 warn/info。
- 保留原行为作为显式开关，避免“优化路径”被全量刷新拖慢。

**Step 4: Run test to verify it passes**

Run:
```bash
cargo test -q precheck_refresh
```
Expected: PASS

**Step 5: Commit**
```bash
git add src/fast_model/precheck.rs src/fast_model/gen_model/precheck_coordinator.rs tests/precheck_refresh_gate_test.rs
git commit -m "fix(precheck): gate pe_transform refresh behind explicit flag"
```

---

## Verification (manual)

在 debug 模式下跑一轮小批次（不要 release，不要 cargo clean）：

```bash
set AIOS_TRANSFORM_CACHE_STATS=1
cargo run -q --bin aios-database -- <你的参数>
```

Expected:
- 打印 transform_cache stats（hit/miss/fallback）
- 不再出现“跨项目串 cache”静默现象（若误用应直接报错）
- 若 `target_sesno` 变化，transform_cache 路径随之变化

