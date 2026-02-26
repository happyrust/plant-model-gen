# SurrealKV Model Write Separation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让模型生成阶段实现“SurrealDB 仅负责输入读取，模型数据仅写入 SurrealKV”，并保留可回滚的双写模式。

**Architecture:** 在 `rs-core` 增加统一模型写入网关（按模式路由到 SurrealDB/KV/双写），`gen_model-dev` 全部模型写路径改为调用该网关。对依赖 `pe`/`fn::*` 的写入字段改为 Rust 预计算，避免 KV-only 模式下 SQL 失效。通过配置项控制运行模式，默认 `kv_only`，`dual` 仅用于对比分析。

**Tech Stack:** Rust, SurrealDB (WS), SurrealKV (embedded), Tokio, serde, anyhow

---

### Task 1: 增加模型写入模式配置（rs-core）

**Files:**
- Modify: `../rs-core/src/options.rs`
- Test: `../rs-core/src/options.rs`（同文件内 `#[cfg(test)]`）

**Step 1: Write the failing test**

在 `DbOption` 的测试中新增：
- 缺省配置时 `model_write_mode` 默认为 `kv_only`
- TOML 设置 `model_write_mode="kv_only"` 可正确反序列化

```rust
#[test]
fn model_write_mode_defaults_to_kv_only() {
    let toml = "project_name='x'\nproject_code='x'\n...";
    let opt: DbOption = toml::from_str(toml).unwrap();
    assert_eq!(opt.model_write_mode.as_deref(), Some("kv_only"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test model_write_mode_defaults_to_kv_only -- --nocapture`
Expected: FAIL（字段不存在或默认值不符）。

**Step 3: Write minimal implementation**

在 `DbOption` 增加字段并设置默认函数：
- `model_write_mode: Option<String>`
- 默认 `Some("kv_only".to_string())`
- 提供解析辅助方法（返回枚举或标准化字符串）

**Step 4: Run test to verify it passes**

Run: `cargo test model_write_mode_defaults_to_kv_only -- --nocapture`
Expected: PASS。

**Step 5: Commit**

```bash
git add ../rs-core/src/options.rs
git commit -m "feat(rs-core): add model_write_mode option for model persistence routing"
```

### Task 2: 实现统一模型写网关（rs-core）

**Files:**
- Modify: `../rs-core/src/rs_surreal/mod.rs`
- Modify: `../rs-core/src/runtime.rs`
- Test: `../rs-core/src/rs_surreal/mod.rs`（同文件测试）

**Step 1: Write the failing test**

新增路由单元测试（无需真连库）：
- `dual` 模式 => 目标包含 `SUL_DB` 与 `KV_DB`
- `kv_only` 模式 => 仅 `KV_DB`
- `surreal_only` 模式 => 仅 `SUL_DB`

**Step 2: Run test to verify it fails**

Run: `cargo test model_write_routing -- --nocapture`
Expected: FAIL（路由函数未实现）。

**Step 3: Write minimal implementation**

在 `rs_surreal/mod.rs` 增加：
- `enum ModelWriteMode { SurrealOnly, Dual, KvOnly }`
- `resolve_model_write_mode(&DbOption) -> ModelWriteMode`
- `model_query_response(sql: &str, mode: ModelWriteMode)`（统一入口）
- 保留 `kv_dual_write` 兼容旧调用，但标记为过渡 API

在 `runtime.rs` 初始化时缓存当前写模式，避免热路径重复解析。

**Step 4: Run test to verify it passes**

Run: `cargo test model_write_routing -- --nocapture`
Expected: PASS。

**Step 5: Commit**

```bash
git add ../rs-core/src/rs_surreal/mod.rs ../rs-core/src/runtime.rs
git commit -m "feat(rs-core): add unified model write gateway with surreal/dual/kv-only modes"
```

### Task 3: 切换 gen_model 写入到统一网关

**Files:**
- Modify: `src/fast_model/gen_model/pdms_inst.rs`
- Modify: `src/fast_model/utils.rs`
- Modify: `src/fast_model/gen_model/orchestrator.rs`（仅模式与日志）

**Step 1: Write the failing test**

新增最小行为测试（可用 mock/feature gate）：
- `use_surrealdb=false` + `model_write_mode=kv_only` 时，模型写流程不触发 `SUL_DB` 写调用。

**Step 2: Run test to verify it fails**

Run: `cargo test kv_only_does_not_write_surreal_model_tables -- --nocapture`
Expected: FAIL（仍有 `SUL_DB.query` 直写）。

**Step 3: Write minimal implementation**

- 将模型表写入统一改为调用 `aios_core` 新网关。
- 删除/替换直接 `SUL_DB.query` 的模型写路径（保留读取路径）。
- 保留错误日志与批处理重试语义不变。

**Step 4: Run test to verify it passes**

Run: `cargo test kv_only_does_not_write_surreal_model_tables -- --nocapture`
Expected: PASS。

**Step 5: Commit**

```bash
git add src/fast_model/gen_model/pdms_inst.rs src/fast_model/utils.rs src/fast_model/gen_model/orchestrator.rs
git commit -m "refactor(gen_model): route model writes through rs-core unified write gateway"
```

### Task 4: 去除 KV-only 下会失效的 fn::* 写时依赖

**Files:**
- Modify: `src/fast_model/gen_model/pdms_inst.rs`
- Modify: `src/fast_model/gen_model/query_provider.rs`（如需批量取 zone/spec）
- Test: `src/fast_model/gen_model/pdms_inst.rs`（同文件或相邻测试模块）

**Step 1: Write the failing test**

构造包含 `inst_relate` 记录的 SQL 生成测试：
- 不再出现 `fn::find_ancestor_type`/`fn::ses_date`
- `zone_refno/spec_value/dt` 来自 Rust 侧预计算值

**Step 2: Run test to verify it fails**

Run: `cargo test inst_relate_sql_should_not_depend_on_fn_calls -- --nocapture`
Expected: FAIL（旧 SQL 仍包含 `fn::*`）。

**Step 3: Write minimal implementation**

- 在写入前批量查询并缓存 `zone_refno/spec_value/dt`。
- 生成 `inst_relate` SQL 时直接填值，不内嵌 Surreal 函数。
- 保持字段语义不变，避免导出侧回归。

**Step 4: Run test to verify it passes**

Run: `cargo test inst_relate_sql_should_not_depend_on_fn_calls -- --nocapture`
Expected: PASS。

**Step 5: Commit**

```bash
git add src/fast_model/gen_model/pdms_inst.rs src/fast_model/gen_model/query_provider.rs
git commit -m "refactor(gen_model): precompute inst_relate fields and remove fn::* write-time dependency"
```

### Task 5: 配置与样例落地

**Files:**
- Modify: `src/options.rs`
- Modify: `db_options/DbOption.toml`
- Modify: `db_options/DbOption-cache.toml`
- Modify: `db_options/DbOption-shadow.toml`
- Modify: `db_options/DbOption-prefetch-cacheonly.toml`

**Step 1: Write the failing test**

增加配置加载测试：
- `model_write_mode=kv_only` 可从扩展配置读取到 `DbOptionExt`
- 与 `use_surrealdb/use_cache` 组合校验通过

**Step 2: Run test to verify it fails**

Run: `cargo test db_option_ext_reads_model_write_mode -- --nocapture`
Expected: FAIL。

**Step 3: Write minimal implementation**

- 在 `DbOptionExt` 透传并打印 `model_write_mode`。
- 更新示例配置，给出 `dual` 与 `kv_only` 两套推荐值。

**Step 4: Run test to verify it passes**

Run: `cargo test db_option_ext_reads_model_write_mode -- --nocapture`
Expected: PASS。

**Step 5: Commit**

```bash
git add src/options.rs db_options/DbOption.toml db_options/DbOption-cache.toml db_options/DbOption-shadow.toml db_options/DbOption-prefetch-cacheonly.toml
git commit -m "chore(config): add model_write_mode samples and load path"
```

### Task 6: 回归验证与文档

**Files:**
- Create: `docs/surrealkv-kv-only-runbook.md`
- Modify: `CHANGELOG.md`

**Step 1: Write the failing test/check**

定义验收命令（先执行应失败或不满足预期）：
- 生成一轮模型后，检查 SurrealDB `inst_*` 无新增（kv_only）
- 检查 KV 中对应表有新增

**Step 2: Run check to verify it fails (before final fixes)**

Run: 项目现有模型生成命令（debug 配置）
Expected: 至少一项不满足（用于证明检查有效）。

**Step 3: Write minimal implementation/docs**

- 写运行手册：配置示例、校验 SQL、回滚到 dual 的步骤。
- 更新 changelog 记录行为变化与兼容策略。

**Step 4: Run full verification**

Run:
- `cargo check`
- `cargo test -- --nocapture`
- 一轮最小模型生成（debug）
Expected: 全部通过。

**Step 5: Commit**

```bash
git add docs/surrealkv-kv-only-runbook.md CHANGELOG.md
git commit -m "docs: add SurrealKV kv-only rollout and rollback runbook"
```

### Task 7: 迁移后观察期（可选但建议）

**Files:**
- Modify: `src/fast_model/gen_model/orchestrator.rs`（日志）
- Modify: `../rs-core/src/rs_surreal/mod.rs`（统计计数器）

**Step 1: Add lightweight counters**

记录：`surreal_write_count`、`kv_write_count`、`fallback_count`。

**Step 2: Run one debug generation**

Run: 既有 debug 模式命令。
Expected: `kv_only` 下 `surreal_write_count=0`。

**Step 3: Commit**

```bash
git add src/fast_model/gen_model/orchestrator.rs ../rs-core/src/rs_surreal/mod.rs
git commit -m "chore(observability): add model write routing counters"
```
