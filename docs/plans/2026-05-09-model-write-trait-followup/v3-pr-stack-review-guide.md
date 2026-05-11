# v3 PR Stack — Reviewer Guide (2026-05-12)

> v3 计划完整产出：**8 个 PR** + 1 个 PR (#11) 的增量。PR 之间 stack 顺序严格，建议按时间线 / 拓扑顺序 review。本文档帮 reviewer 知道每个 PR 的范围、依赖、关键验证点。

## PR 拓扑

```
                                                    feat/model-persistence-trait
                                                    ─┬─────────────────────────────
PR #11  (v2 trait + v3 plan increments, force-push-free)
   │
   ├── PR #13   docs/model-writer-storage-mission       ← docs-only, 独立
   │
   └── PR #14   feat/parquet-model-writer-backend       ← Phase B
         │
         └── PR #15   feat/model-writer-compare-mode    ← Phase C, base = #14
               │
               └── PR #16   feat/ducklake-backend-skeleton  ← Phase D, base = #15
                     │
                     └── PR #17   feat/model-writer-validation-cli  ← Phase E, base = #16
                           │
                           └── PR #18   refactor/take-missing-neg-carriers ← F.1, base = #17
                                 │
                                 └── PR #19   refactor/bridge-context        ← F.2, base = #18
```

## 推荐 Review 顺序

| 顺位 | PR | Why this order |
|---|---|---|
| 1 | **#13** | docs-only，独立于代码 PR；先过这个让 reviewer 建立 mission 视角 |
| 2 | **#11** | v3 plan 文件 (`v3-plan.md`) 在这里，让 reviewer 看到 v3 整体路线图 |
| 3 | **#14** | 第一个 backend 实装（Parquet），是后续所有 backend 工作的样本 |
| 4 | **#15** | Compare wrapper：trait 装饰器模式的例子；不动 orchestrator |
| 5 | **#16** | DuckLake skeleton：feature-gated 占位；最小 PR，可 5 分钟过 |
| 6 | **#17** | CLI + SQL：与 backend 实装解耦，可单独 merge |
| 7 | **#18** | F.1 trait 接口纯化：删 `WriteBaseReport.missing_neg_carriers` |
| 8 | **#19** | F.2 接口纯化收尾：删 `BooleanBridgeRequest::db_option` |

> 顺位 6-8 可乱序 review，互相独立（虽然 git stack 上 #18 base 是 #17、#19 base 是 #18，但代码上完全正交）。

## 每个 PR 的"看什么"

### PR #13 — Mission docs
- 9 份 markdown 文件，无代码
- 检查：Phase 1 / Phase 2 boundary（mission 00）、`ducklake-canonical` schema 描述（04）、SurrealDB Cargo source 锁定（00 / 07）
- 关键判断：mission scope 是否同意

### PR #11 — v2 + v3 plan
- v3 plan 文件 `docs/plans/2026-05-09-model-write-trait-followup/v3-plan.md`（510 行）
- 检查：6 个 Phase 拆分、8 条架构 invariants、风险表
- 关键判断：v3 路线图是否同意（这个一旦同意，#14..#19 都顺水推舟）

### PR #14 — Parquet backend
- 新增 `parquet.rs::ParquetModelWriterBackend` (~250 行)，挂到 `create_model_writer` 工厂
- 关键看 trait 8 个方法实现：
  - `init`：缓存 context + 准备 raw_root
  - `write_base_batch`：`CanonicalRawPlanner` → 13 JSONL
  - `persist_mesh_results`：NoOp（Phase 1 不含 mesh，mission 05）
  - `write_inst_relate_aabb`：NoOp（base 阶段已落，避免双写）
  - `reconcile_missing_neg`：返回 0 + `approximate=true` log
  - `run_boolean_bridge`：skipped(`phase2_boolean_not_supported`)
- 关键判断：Parquet 作为 "file-oriented backend, not SurrealDB replacement" 的定位是否同意

### PR #15 — Compare wrapper
- 新增 `compare.rs::CompareModelWriterBackend` (~390 行)，trait 装饰器
- **关键设计**：装饰器零侵入 orchestrator 调用面
- 检查：
  - 每个方法都 primary→candidate 顺序
  - candidate 失败立即 bail（mission 03 "fail fast, no silent fallback"）
  - `run_boolean_bridge` 只走 primary（candidate 在 Phase 2 范围，不路由）
  - `take_missing_neg_carriers` 由 F.1 引入，fan-out 到两边、diff log
- 关键判断：装饰器选型 vs 修改 orchestrator 入参，哪种更好

### PR #16 — DuckLake skeleton
- ~80 行，全部 `bail!`
- 关键看：
  - `Cargo.toml` 新增 `ducklake = []` feature（**未引 duckdb crate**）
  - `cargo check --features review` + `cargo check --features review,ducklake` 都得跑通
  - `create_model_writer` 未启 feature 时 bail 信息清晰
- 关键判断：v4 之前用 feature-gated 占位 vs 不占位，哪种更好

### PR #17 — Validation CLI + SQL
- 两部分：
  - `aios-database model-writer diff-summary` CLI（cli_modes.rs + main.rs，~200 行）
  - 26 个 DuckDB SQL parity 脚本 + README + 维护 generator
- 关键看：
  - CLI smoke：`match` exit 0、`diff --fail-on-diff` exit 2
  - SQL 模板的 `EXCEPT` 双向 diff 逻辑
  - Phase 1 表覆盖是否完整（13 张）
- 关键判断：v4 typed `.parquet` 物化后能否只换 `read_json_auto` → `read_parquet`

### PR #18 — F.1 take_missing_neg_carriers
- 删 `WriteBaseReport.missing_neg_carriers` 字段
- 加 trait method `take_missing_neg_carriers(&self) -> Result<Vec<RefnoEnum>>` (default empty)
- Surreal/Mock 实现 drain；orchestrator 改读方式
- 关键看：drain 幂等性断言（verify binary exit code 4）
- 关键判断：trait method default-empty 选型

### PR #19 — F.2 db_option 入 ctx
- 删 `BooleanBridgeRequest::db_option` 字段
- 加 `ModelWriterContext::db_option: Arc<DbOption>`
- Surreal `run_boolean_bridge` 从 cached ctx 取 db_option
- 关键看：trait 接口面再减一字段
- 关键判断：是否同意 "缓存胜过新 newtype" 的选型（vs 抽 `BridgeContext` newtype）

## 验证 / 编译矩阵

每个 PR 都需要：

```powershell
cargo check --bin aios-database --features review
```

#16 还需要额外：

```powershell
cargo check --bin aios-database --features "review,ducklake"
```

#14 / #15 / #18 / #19 还需要 verify binary：

```powershell
pwsh -NoProfile -File docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1
# 等同于：
cargo run --bin verify_model_writer_trait --features model-writer-mock
```

退出码语义：

| Code | Meaning |
|---|---|
| 0 | PASS |
| 1 | trait 方法返回 Err |
| 2 | snapshot 调用计数不符 |
| 3 | snapshot 顺序不符 |
| 4 | 注入值未被 backend 返回（含 F.1 drain idempotent） |
| 5 | 二次 init / cleanup-without-init 不安全 |
| 6 | Parquet 端到端失败 |
| 7 | Compare wrapper 端到端失败 |
| 8 | (reserved for v4 DuckLake) |

## 架构 Invariants (整个 v3 都不能破)

1. DrainOnly 快路径 = baseline 模式（跳过持久化作 IO 对比基准）
2. SurrealDB Cargo source 必须保持 `github.com/happyrust/surrealdb`
3. `[model-writer:*]` 日志前缀契约不破
4. 不引入新 crate 依赖（feature-gated 例外：`ducklake`）
5. boolean 表是 Phase 2，所有非 Surreal backend 跳过
6. Fail fast, no silent fallback（mission 03）
7. trait 接口不泄漏 `pdms_inst::` / `Arc<DbOption>` / `Vec<RefnoEnum>`

## 合并节奏建议

1. **先 merge docs (#13)**：reviewer 看完 mission 后好做后面代码 review
2. **再 merge #11 v3 plan**：让 reviewer 看到整体路线（如果不 merge plan，后续 PR review 容易跑偏）
3. **#14 / #15 / #16 / #17 顺序 merge**：每个独立逻辑闭环
4. **#18 / #19 可一起 merge**：两个小重构，互相独立
5. **如果 main 有冲突**：用 GH UI `Rebase and merge` 解决，不要 force push（A.3 deferred 的原因）

## v3 完成的统计

- **代码改动**：~2500 行新增 + ~150 行删除（净增量；CRLF/LF 噪声让 git diff stat 看起来更大）
- **新文件**：12 个 Rust 模块 / binary / 文档
- **trait 方法数**：8 → 9（加 take_missing_neg_carriers）
- **backend 数**：3 → 5（加 Parquet + Compare wrapper + DuckLake skeleton）
- **CLI 子命令**：1 → 2（加 diff-summary）
- **SQL parity 脚本**：0 → 26
- **PR 数**：1 → 9 (#11 增量 + #13..#19)
