# v4 候选议题 (草稿，待 plannotator 正式立项)

> v3 (2026-05-12) 全部 PR 化完成（#13..#19 + #11 增量），整个 trait 抽象闭环 + Parquet/Compare/DuckLake-skeleton/CLI+SQL validation/P5-backlog 二项收口落地。本文档罗列 v3 期间产生的、明确推迟到 v4 的候选议题，等用户决定优先级与拆 PR 节奏后通过 plannotator 正式立项。

## 0. 状态前提

- v3 PR 栈：#11 → #13 / #14 → #15 → #16 → #17 → #18 → #19，本地分支 `feat/model-persistence-trait`、`docs/model-writer-storage-mission`、`feat/parquet-model-writer-backend`、`feat/model-writer-compare-mode`、`feat/ducklake-backend-skeleton`、`feat/model-writer-validation-cli`、`refactor/take-missing-neg-carriers`、`refactor/bridge-context`
- 整个 trait 接口面已纯化：`WriteBaseReport` 只剩 `batch_id` + `missing_neg_count`；`BooleanBridgeRequest` 只剩 `mode` + `bool_tasks`；不再泄漏 `pdms_inst::`/`RefnoEnum`/`DbOption`
- 5 个 backend 全部到位：Surreal / DrainOnly / Parquet / DuckLake(skeleton) / Mock + Compare wrapper
- CLI: `aios-database model-writer {validate-canonical-parquet, diff-summary}` + 26 个 DuckDB SQL parity 脚本
- verify binary 退出码 0-7 全覆盖

---

## 1. 高优先级 (v4 主目标)

### 1.1 DuckLake 真实写入实装

**Why now**：mission `04-ducklake-writer.md` 把它列为 target architecture；v3 仅留了 feature-gated 骨架（PR #16），所有 trait 方法 `bail!`。

**Scope**：

- 引入 `duckdb` crate 依赖（feature-gated by `ducklake`）
- DuckLake metadata.ducklake 路径配置（DbOption 字段：`ducklake_metadata_path` + `ducklake_data_root`，沿用现有 `transform_ducklake_metadata` 风格）
- `init`：打开 DuckDB / attach DuckLake / 建 `ducklake-canonical` schema if absent
- `write_base_batch` / `persist_mesh_results` / `write_inst_relate_aabb`：transaction-style upsert 到 13 张 raw 表
- `cleanup` / `reconcile_missing_neg`：SQL DELETE + missing-key 扫描
- `finalize`：projection refresh SQL（per mission `04` §Table strategy）
- Verify binary 加 exit code 8 = ducklake path fail
- Compare 模式可挂 ducklake 作为 candidate（trait 已支持）

**Non-goals**：

- 不替代 SurrealDB 主写入（仍是默认）
- 不重写 v3 已落地的 Parquet sink（JSONL fallback 仍保留作为 export 备份）

**预估**：~600 行 Rust + ~150 行 SQL + DbOption 字段。**风险**：duckdb crate 在 Windows 上 build 重，需要 LLVM/MSVC；mission 00 §Non-goals 明确 SurrealDB 源不能改。

---

### 1.2 Parquet 真正 `.parquet` 物化（替换 JSONL fallback）

**Why now**：mission `05-parquet-writer.md` §Phase boundary 明确 JSONL 只是 v3 fallback；CLI E.2 的 26 个 SQL parity 脚本目前用 `read_json_auto`，typed `.parquet` 直接换成 `read_parquet` 性能 + schema 强度都提升。

**Scope**：

- 引入 `arrow` + `parquet` crate（feature-gated by `parquet-canonical`，避免污染默认构建）
- `CanonicalRawTable` per-table Arrow schema 静态定义（用 `arrow_schema::Schema`）
- `CanonicalParquetWriter::write_raw_batch` 走 `parquet::arrow::ArrowWriter`，按现有目录 layout 落 `.parquet`
- 旧 JSONL 路径保留为兼容回退（`--parquet-format jsonl|parquet`，默认 parquet）
- SQL 脚本 `read_json_auto(...)` 改为 `read_parquet(...)`（一行替换）
- Verify binary 加 schema 断言：每张表的列类型与 canonical_records.rs 字段一致

**Non-goals**：

- 不引入 polars
- 不改 canonical_records.rs schema 定义（仅 derive Parquet schema）

**预估**：~400 行 + arrow/parquet crate 依赖。

---

## 2. 中优先级

### 2.1 `inst_relate_bool` / `inst_relate_cata_bool` Phase 2 boolean canonical records ⚠️ PARTIAL (PR #22 schema scaffold only)

**Status**: 2026-05-12 schema-only 部分完成。PR #22 (`feat/phase2-boolean-canonical-schema`)，base = `docs/async-fn-in-trait-research` (PR #21)，单文件 `canonical_records.rs` ~70 行。

实施时发现 `manifold_bool.rs` 的 boolean worker 直接 emit SQL 进 SurrealDB，`BoolWorkerReport` 只有聚合计数没有 per-row data；让所有 backend `run_boolean_bridge` 落 canonical 需要先重构 worker 暴露 per-row 输出，超出 ~250 行预算。

**实际落地**（PR #22）：
- 加 `CanonicalRawTable::RawInstRelateBool` / `RawInstRelateCataBool` 枚举变体
- 加 `RawInstRelateBoolRecord` / `RawInstRelateCataBoolRecord` struct（字段从 v3 Surreal SQL 反推）
- 加 `phase2_limitation()` + `all_phase2()` 方法
- `CanonicalRawRowCounts::set` 回退到 phase2_limitation
- `CanonicalRawBatch` **不动**（不加 Vec 字段，否则强迫所有 backend flush 实际行）

**留到 v5+**（PR 拆分清单）：
1. 给 `CanonicalRawBatch` 加 `inst_relate_bool` / `inst_relate_cata_bool` Vec 字段
2. 重构 `manifold_bool.rs` 把 worker per-row 输出 expose 为 channel 或 BoolWorkerReport 扩展
3. Surreal/Parquet/Compare 三个 backend 收集 boolean rows + finalize flush
4. `scripts/sql/model-writer-parity-phase2/` 新建 + 4 个 SQL
5. verify binary 加 Phase 2 fixture

**Why now**：mission `00` + `08` 把 boolean 表明确推到 Phase 2；v3 所有 backend 的 `run_boolean_bridge` 现在都跳过这两张表（Parquet/Compare 返回 `phase2_boolean_not_supported` skip log）。

**Scope**：

- canonical_records.rs 加 `RawInstRelateBoolRecord` + `RawInstRelateCataBoolRecord`
- `CanonicalRawTable` enum 加两个变体（注意 v3 `all_phase1()` 是 Phase 1 only，新建 `all_phase2()` 或合并）
- 所有 backend `run_boolean_bridge` 改写：成功路径落 canonical raw records；Parquet sink 多写 2 张 JSONL/parquet 表
- E.2 SQL 脚本目录扩 `scripts/sql/model-writer-parity-phase2/`
- verify binary 加 phase 2 fixture

**Non-goals**：

- 不动 boolean worker 内部 SQL；只在 trait boundary 多落一层 canonical

**预估**：~250 行 + 4 个 SQL 脚本。

---

### 2.2 `async fn in trait` 调研报告 ✅ DONE (PR #21 — decision: NO migrate)

**Status**: 2026-05-12 完成。PR #21 (`docs/async-fn-in-trait-research`)，base = `refactor/drain-only-stats-atomic` (PR #20)，docs-only。结论：**v4 不迁移**。理由：native AFIT 在 Rust 1.97-nightly 仍不 `dyn`-compatible；项目通过 `Arc<dyn ModelWriterBackend>` 全程走 dyn 路径；迁移需要 `trait-variant` 新依赖或拆分 trait（surface ×2），dyn 路径仍 box future、零 perf 收益。详见 `docs/development/model-writer-storage/09-async-trait-migration.md`。Reconsider 触发：`dyn_compatible_for_dispatch` 稳定（rust-lang/rust#107011）。

**Why now**：v3 全程用 `#[async_trait]`，每个 trait 方法都过 `Pin<Box<dyn Future>>`，hot path 上有开销。Rust 1.75+ nightly 已稳定 native AFIT。

**Scope** (Documentation-only, not code change)：

- 在 `docs/development/model-writer-storage/` 加 `09-async-trait-migration.md`
- 内容：调研 native AFIT vs async_trait 对 `Send + Sync + dyn` 的兼容性、性能差异、迁移成本
- 决策：迁移 / 不迁移 / 部分迁移；如果决策迁移则在 v4 后续 PR 实施

**Non-goals**：

- 不立即迁移（先调研）

**预估**：1 份文档 + 任选 1 个 trait 方法做 benchmark fixture。

---

### 2.3 DrainOnly stats `Mutex` → atomic ✅ DONE (PR #20)

**Status**: 2026-05-12 完成。PR #20 (`refactor/drain-only-stats-atomic`)，base = `refactor/bridge-context` (PR #19)，单文件改动 `drain_only.rs`，cargo check + verify-mock 均 PASS。`run_drain_only_sink` 主路径未动（仍 stack stats + plain usize +=），仅 trait-routed 那条路改 atomic。

**Why now**：v2 N-5；DrainOnly 是 baseline，stats 走 `Mutex<DrainOnlyStats>`，hot path 上 lock 开销稍大。

**Scope**：

- `DrainOnlyStats` 字段全改 `AtomicU64` / `AtomicUsize`
- `print_summary` 用 `load(Ordering::Relaxed)` 读
- 所有 trait 方法的 stats 累加改 `fetch_add`

**Non-goals**：

- 不动 DrainOnly 快路径（v2 invariant 锁定）

**预估**：~80 行。**小 PR**。

---

## 3. 低优先级（视情况打包到 v4 或更后）

### 3.1 PR #11 合并时的 rebase 处理（A.3 deferred）

- 当前 `feat/model-persistence-trait` 落后 main 13 commits、领先 18 commits（含 v3 increments）
- 推荐：reviewer 通过 GH UI 选 `Rebase and merge`，避免 force push 污染评论
- 若合并冲突过大，转 worktree 本地 rebase + force push（需用户授权）

### 3.2 9 个 mission docs 的格式标准化

- 部分 docs 还是 Markdown 草稿，可能加 frontmatter / TOC / cross-link
- 不阻塞功能，纯文档美化

### 3.3 BridgeContext 升级到更彻底拆分

- v3 F.2 把 `Arc<DbOption>` 移到 `ModelWriterContext`，本质是 "把字段从 request 搬到 context"
- 真正彻底的拆分：抽 `BridgeContext { use_surrealdb, defer_db_write, bool_worker_batch_size }`，让 boolean worker 函数签名也不再依赖 `Arc<DbOption>`
- v3 找了简化路径（缓存而非抽 newtype），如果 v4 要做 native AFIT 迁移、或要把 boolean worker 模块化，可能值得重新拆

---

## 4. v3 残余事项不在 v4 范围

- `_gen_sql.ps1` 重生成时的 here-string 行为：已修正模板但保留为维护工具，不入 production path
- worktree 卫生（本轮新建 7 个 worktree 已 push 到 origin；可随时 `git worktree remove .worktrees/<name>` 回收磁盘）

---

## 5. v4 拆 PR 建议节奏（草稿）

```
v4 PR #1   docs(model-writer-storage): 09-async-trait-migration.md (调研)
v4 PR #2   refactor(model-writer): DrainOnly stats Mutex → atomic   (small)
v4 PR #3   feat(model-writer): Parquet typed materialization        (B.x v2)
v4 PR #4   feat(model-writer): DuckLake real implementation         (主目标)
v4 PR #5   feat(model-writer): Phase 2 boolean canonical records    (E2.5)
```

PR #4 是 v4 主目标，依赖 #3（typed parquet 让 SQL parity 更强）。其余可并行。

---

## 6. 下一步建议

1. 等 v3 PR 栈（#13..#19）逐步 review + merge；任何 review feedback 都先回到对应 v3 PR 处理
2. PR #11 通过 GH UI `Rebase and merge` 解决 A.3
3. v3 全部 merge 后，把本文件转成 `v4-plan.md` 通过 plannotator 提交审核，按 §5 顺序立项

> 本文件是草稿，**不是** plannotator approved 计划。正式执行前需要：
> 1. 用户确认 §1-§3 的优先级
> 2. 用户拍板 §5 的拆分节奏
> 3. 通过 plannotator submit_plan 走 v4 审核流程
