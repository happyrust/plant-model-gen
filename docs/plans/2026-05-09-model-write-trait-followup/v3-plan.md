# Worktree `model-persistence-trait` 推进计划 (v3)

> **历史脉络**：
> - v1 (2026-05-11-142521) 被拒：DrainOnly 快路径用途说明缺失
> - v2 (2026-05-11-142958) approved：trait abstraction follow-up，已落地 PR #11 (5 commits 至 HEAD `8bb69632`)
> - **本 v3**：在 PR #11 基础上继续推进 multi-backend 实装与 canonical raw boundary 平行工作流的合流
> - plannotator 存档：`C:\Users\dpc\.plannotator\plans\2026-05-11-014336-worktree-model-persistence-tra-approved.md`

> **For Agent**：本文件是结构化数据。所有 Task 按 `## 5. 详细任务` 顺序执行，每完成一个立即更新 `docs/plans/2026-05-09-model-write-trait-followup/progress.md` 的 v3 小节。

**仓库**：https://github.com/happyrust/plant-model-gen.git · worktree `.worktrees/model-persistence-trait` · 分支 `feat/model-persistence-trait` · HEAD `8bb69632`

**Goal**：在 v2 (PR #11) 落地的 trait abstraction 基础上，把 `model_writer/parquet.rs` 从 "canonical raw sink scaffold" 升级为真正的 `ModelWriterBackend`，并落地 orchestrator 多 backend 选择 + compare 模式，使 mission docs Phase 2 真正可用；同时清理 v2 残留的 3 个未提交脏文件与 9 份 untracked mission docs，按节奏拆分独立 PR。

---

## 0. 架构既定决策 (Architecture Invariants — 沿用 v2 + 新增)

沿用 v2 的 4 条：

1. DrainOnly 快路径 = baseline 模式（跳过持久化作 IO 耗时对比基准），不允许重构进主管线。
2. DrainOnly backend 只走 `init` / `finalize` 两个 lifecycle 方法，其余 6 个为 mock/verify 防御性存在。
3. DrainOnly stats 走旧 `run_drain_only_sink` 函数，不联动 trait 中间方法。
4. canonical_records.rs / parquet.rs / docs/development/model-writer-storage/ 是平行工作。

**v3 新增**：

5. **Parquet writer 是 "file-oriented backend"**，不替代 SurrealDB，定位见 mission 05-parquet-writer.md。本轮把它升级为 trait 实现的目标只是 "可被 `create_model_writer` 工厂构造、可走主管线"，不追求与 Surreal 的 SQL/projection 完整 parity。
6. **DuckLake writer 在本 v3 不实装**，仅做最小骨架（trait impl 全 `bail!("not yet implemented")`），避免引入 `duckdb` crate 依赖污染本轮 review。真正实装留 v4。
7. **compare 模式遵循 mission 03 "fail fast, no silent fallback"**：任一 backend 写失败立即终止，不静默忽略。
8. **SurrealDB Cargo source 不变**：必须保持 `github.com/happyrust/surrealdb`（mission 00 / 07 多次强调）。

---

## 1. 当前基线

| 维度 | 状态 |
|---|---|
| Worktree HEAD | `8bb69632` (v2 5 commits 已 push 至 PR #11) |
| 工作区未提交 | 3 文件 modified: `progress.md` / `mock.rs` / `options.rs`；9 份 untracked mission docs in `docs/development/model-writer-storage/` |
| Trait | `ModelWriterBackend` (8 方法，v2 已纯化) |
| Backend 实现 | Surreal（主） / DrainOnly（baseline） / RecordingBackend（mock，feature-gated） |
| Parquet scaffold | `CanonicalParquetWriter` 不实现 trait，目前用 JSONL fallback + `validate-canonical-parquet` CLI 子命令 |
| Canonical records | `canonical_records.rs` (587L) — 13 张 Phase 1 表的 raw record 结构 + `CanonicalRawPlanner` |
| DuckLake | 仅 mission docs (`04-ducklake-writer.md`)，无代码 |
| PR #11 状态 | open，等 review |

---

## 2. 范围与不做事项

**做**：

1. **A 阶段**：v2 残留 cleanup —— commit 3 文件、决定 9 份 mission docs 的归属 PR、保证 worktree 干净。
2. **B 阶段**：`CanonicalParquetWriter` 升级为 `ModelWriterBackend` 实现 + 接入 `create_model_writer` 工厂。
3. **C 阶段**：orchestrator 加 `BackendSelection` 枚举（surreal/parquet/ducklake/compare），DbOption 增 `model_writer_compare_with` 字段。
4. **D 阶段**：`DuckLakeModelWriterBackend` 最小骨架（trait impl 全 `bail!`），仅占位以便 v4 推进。
5. **E 阶段**：CLI + SQL validation 全套——把现有 `validate-canonical-parquet` 扩为 `validate-model-writer` 子命令族，覆盖 Phase 1 13 张表的 row count parity 检查。
6. **F 阶段**：P5 backlog 收口（`take_missing_neg_carriers` / `BridgeContext` 拆分），保留 `async fn in trait` 评估留 v4。
7. 每个阶段独立 PR，避免 PR #11 review 阻塞下游工作。

**不做**：

- 不跑 `cargo test`（AGENTS.md 禁，沿用 verify binary + ps1 脚本）。
- 不重写 Surreal backend 的 SQL helper（已纯化，无新发现问题）。
- 不实装 DuckLake 真实写入（D 阶段仅骨架，留 v4）。
- 不动 DrainOnly 快路径与中间方法语义（v2 锁定）。
- 不引入 `duckdb` crate 依赖（避免污染 PR），仅在 D 阶段加 `#[cfg(feature = "ducklake")]` 守门的骨架代码。
- 不动其他 worktree（pe-transform-backends / perf-* / room-compute-3x）。
- 不跨仓修改（rs-core / pdms-io-fork / plant3d-web 不动）。
- 不破坏现有日志契约（`[model-writer:*]` 前缀 + `[batch_perf] batch=...`）。

---

## 3. 阶段总览

| Phase | 名称 | 解决的问题 / 推进的能力 | 预估改动量 | 独立 PR? |
|---|---|---|---|---|
| A | v2 残留 cleanup | worktree 脏文件 + untracked docs | ~50 行 | 是（docs-only + small fix） |
| B | Parquet trait 化 | mission Phase 2 部分兑现，多 backend 实证 | ~250 行新代码 + ~30 行 orchestrator 接入 | 是 |
| C | Orchestrator backend selection + compare | mission Phase 3 主体兑现 | ~150 行 | 是（依赖 B） |
| D | DuckLake backend 骨架 | 为 v4 准备 + 接口验证 | ~80 行 + feature flag | 是（small） |
| E | CLI + SQL validation 全套 | mission Phase 4 兑现 | ~200 行 CLI + ~150 行 SQL | 是 |
| F | P5 backlog 收口 | 把 P5 中可在本轮完成的拆出来 | ~100 行 | 1-2 个 small PR |

---

## 4. 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| PR #11 review 期间合 main 引入冲突 | 中 | A 阶段先 `git fetch && git rebase origin/main`，A 之后任何 Phase 启动前都 sync 一次 |
| Parquet 写入引入 `arrow`/`parquet` crate 依赖膨胀 | 中 | B 阶段先沿用现有 JSONL fallback（mission 05 已认可）；真正 `.parquet` 物化推迟到独立 PR |
| compare 模式双写性能下降 | 中 | C 阶段加 `compare_mode_max_batch_size` 节流，文档警告 "compare 模式仅用于 validation，不上生产" |
| DuckLake 骨架 `bail!` 出现在 release 路径 | 高 | D 阶段强制 `#[cfg(feature = "ducklake")]` 守门，`Cargo.toml` 默认不启用；`create_model_writer` 在未启 feature 时 `bail!("ducklake backend requires --features ducklake")` |
| validate-model-writer CLI 与现有 `validate-canonical-parquet` 命名冲突 | 低 | E 阶段先 alias 兼容老命令，新增 `validate-model-writer` 作为 umbrella 命令 |
| 9 份 mission docs commit 到 trait PR 会让 review 范围模糊 | 中 | A 阶段把 mission docs 单独发 docs PR（不阻塞代码 PR） |
| Phase F 拆 `take_missing_neg_carriers` 改 trait 签名会破 mock + verify binary | 中 | F 阶段同时改 mock.rs + verify binary，跑 `verify-mock.ps1` 兜底 |

---

## 5. 详细任务

### Phase A：v2 残留 cleanup（独立 PR：mission-docs + small fix）

#### Task A.1 — 审查 3 个未提交文件

**Files**：
- `docs/plans/2026-05-09-model-write-trait-followup/progress.md`
- `src/fast_model/gen_model/model_writer/mock.rs`
- `src/options.rs`

**判定**（2026-05-12 执行已确认）：
- `mock.rs` / `options.rs`：纯 CRLF/LF 噪声（diff 为空），`git checkout --` 还原
- `progress.md`：真实改动（T4.3/T4.4 状态从 pending 改为 complete + PR #11 URL）+ v3 milestones 表追加

**Steps**：
1. `git checkout -- src/fast_model/gen_model/model_writer/mock.rs src/options.rs` （已执行）
2. 在 progress.md 末尾追加 "## 2026-05-12 v3 启动" 章节 + v3 milestones 表
3. 新增 `docs/plans/2026-05-09-model-write-trait-followup/v3-plan.md`（本文件）
4. commit：`docs(plan): publish v3 plan + sync v2 progress status to PR #11`
5. push 到 `feat/model-persistence-trait`（PR #11 自动 pick up，**不 amend、不 force**）

**Verify**：`git status --short` 仅留 mission docs 的 `??` 行。

---

#### Task A.2 — Mission docs 独立 docs-only PR

**Files**：`docs/development/model-writer-storage/{00..08}-*.md` (9 文件)

**Steps**：
1. 基于 `origin/main` 起新分支 `docs/model-writer-storage-mission`
2. `git add docs/development/model-writer-storage/` 并 commit：`docs(model-writer-storage): add Phase 1 mission docs (overview + canonical schema + backend roadmap)`
3. push + `gh pr create`，PR body 引用 mission 00 overview
4. PR 标题：`docs(model-writer-storage): Phase 1 mission docs (canonical raw + parquet/ducklake roadmap)`
5. PR URL 写回 worktree progress.md "v3 milestones" 表

**Expected**：mission docs 走独立 review，不与 trait abstraction PR #11 缠绕，不阻塞下游 B/C/D 阶段。

**Verify**：`gh pr list --search "head:docs/model-writer-storage-mission"` 返回一行。

---

#### Task A.3 — 同步 main 到 worktree

**Steps**：
```powershell
git -C .worktrees/model-persistence-trait fetch origin
git -C .worktrees/model-persistence-trait rebase origin/main
```
冲突处理原则：保留 trait 化骨架，main 上的小改按语义合入。

**Expected**：worktree HEAD 基于最新 `origin/main`，PR #11 force-push 一次（用户授权前提下）。

**Verify**：`git log --oneline origin/main..HEAD` 仍是 v2 那 5 commit + A.1 增量。

---

### Phase B：Parquet trait 化（独立 PR）

> 依赖：A 完成。

#### Task B.1 — `ParquetModelWriterBackend` 骨架

**Files**：
- Modify: `src/fast_model/gen_model/model_writer/parquet.rs`
- Modify: `src/fast_model/gen_model/model_writer/mod.rs`

**Steps**：
1. 在 parquet.rs 新增 `pub struct ParquetModelWriterBackend { writer: Arc<Mutex<CanonicalParquetWriter>>, context: OnceLock<ModelWriterContext> }`
2. impl `ModelWriterBackend`：
   - `name()` → `"parquet"`
   - `init` → 缓存 context；调 `CanonicalParquetWriter::ensure_output_layout` 准备目录
   - `cleanup` → 清理目标 `dbnum/refno` 范围下旧文件（按 mission 05 layout：`output/<project>/model_writer_storage/raw/<table>/project_name=...`）
   - `write_base_batch` → 收 batch，调 `CanonicalRawPlanner::plan_from_base` 转 canonical records，调 writer JSONL fallback 落盘；返回 `WriteBaseReport { batch_id, missing_neg_count: 0 }`
   - `persist_mesh_results` → batch.file_mesh_state=true 时跳过；否则按 mesh canonical records 落盘
   - `write_inst_relate_aabb` → canonical inst_relate_aabb 落盘
   - `reconcile_missing_neg` → 读取已落盘 inst_relate / neg_relate，做内存 set diff，返回 inserted 行数（≤ Surreal 语义近似，标 "approximate" 日志）
   - `run_boolean_bridge` → 返回 `BooleanBridgeReport::skipped("parquet", 0, "phase2_boolean_not_supported")`（mission 05 明确 boolean 是 Phase 2）
   - `finalize` → 把 `CanonicalParquetWriter` flush + summary JSON 输出（mission 07 §2a 已有约定）
3. mod.rs 顶部 `pub use parquet::ParquetModelWriterBackend;`，把 "Canonical raw sink (not a backend)" 注释升级为 "Canonical Parquet/JSONL sink backend"

**Verify**：`cargo check --bin plant_db_cli --features review` 通过；`ReadLints` 通过。

---

#### Task B.2 — 接入 `create_model_writer` 工厂

**Files**：`src/fast_model/gen_model/model_writer/mod.rs`

**Steps**：
1. `ModelWriterMode` 加 `Parquet { output_root: PathBuf }` 变体（或单独的 `parquet_output_root` 字段挂在 DbOption 上，看哪种侵入小）
2. `create_model_writer` match 加分支：`ModelWriterMode::Parquet { output_root } => Arc::new(ParquetModelWriterBackend::new(output_root.clone())?)`
3. `options.validate_model_writer_features` 加守卫：Parquet 模式不需要 `use_surrealdb`，但要求 `output_root` 可写

**Verify**：`cargo check` 通过；用 `model_writer_mode=parquet` 跑一个 dbnum 应成功落盘到 mission 05 §Layout 描述的目录结构。

---

#### Task B.3 — Verify binary 加 Parquet 路径

**Files**：`src/bin/verify_model_writer_trait.rs`

**Steps**：
1. 在现有 RecordingBackend 验证之后，加一段 `ParquetModelWriterBackend` 走完整 trait 8 方法的 e2e
2. 校验目标目录有 13 张 canonical raw 表的 JSONL/summary 文件
3. exit code 拓展：6 = parquet path fail

**Verify**：`pwsh -NoProfile -File docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1` exit=0。

---

#### Task B.4 — B 阶段 PR

**Steps**：
1. 分支 `feat/parquet-model-writer-backend`
2. commit 分片：B.1 (骨架) / B.2 (factory + DbOption) / B.3 (verify)
3. PR 标题：`feat(model-writer): wire Parquet backend into ModelWriterBackend trait`
4. PR body 引用 mission 05-parquet-writer.md + B.1 接入说明

---

### Phase C：Orchestrator backend selection + compare 模式

> 依赖：B 完成。

#### Task C.1 — `BackendSelection` 枚举

**Files**：
- Modify: `src/options.rs`（DbOption）
- Modify: `src/fast_model/gen_model/model_writer/mod.rs`

**Steps**：
1. DbOption 加字段：`pub model_writer_compare_with: Option<ModelWriterMode>`（default None）
2. `create_model_writer` 改返回 `(Arc<dyn ModelWriterBackend>, Option<Arc<dyn ModelWriterBackend>>)`，第二个是 compare backend
3. 或者更稳：保持 `create_model_writer` 单返回，新增 `create_compare_writer` 工厂；orchestrator 决策是否拉起 compare

**Verify**：`cargo check` 通过；`ReadLints` 通过。

---

#### Task C.2 — Orchestrator compare 路径

**Files**：`src/fast_model/gen_model/orchestrator.rs`

**Steps**：
1. `process_index_tree_generation` 入参拓展为 `(primary: Arc<dyn ModelWriterBackend>, compare: Option<Arc<dyn ModelWriterBackend>>)`
2. 每个 trait 方法调用点：先调 primary，再调 compare（compare fail 立即 bail，不静默）
3. `finalize` 时输出 primary + compare 两份 summary，diff 关键字段（batch counts / raw rows by table）
4. 日志加 `[model-writer:compare] primary={surreal} candidate={parquet} table={inst_relate} primary_rows=N candidate_rows=M`

**Verify**：用最小 dbnum 同时跑 `model_writer_mode=surreal, compare_with=parquet`，确认两个 backend 都落数据；compare 模式日志输出 diff。

---

#### Task C.3 — C 阶段 PR

**Steps**：
1. 分支 `feat/model-writer-compare-mode`
2. commit 分片：C.1 (enum + DbOption) / C.2 (orchestrator)
3. PR 标题：`feat(model-writer): add compare mode for dual-write parity validation`

---

### Phase D：DuckLake backend 骨架

> 依赖：C 完成（确认 `BackendSelection` 枚举位置）。

#### Task D.1 — `DuckLakeModelWriterBackend` 占位

**Files**：
- Create: `src/fast_model/gen_model/model_writer/ducklake.rs`
- Modify: `src/fast_model/gen_model/model_writer/mod.rs`
- Modify: `Cargo.toml`

**Steps**：
1. Cargo.toml 加 `ducklake = []` feature（暂不引 `duckdb` crate）
2. ducklake.rs `#![cfg(feature = "ducklake")]`，定义 `pub struct DuckLakeModelWriterBackend { ... }`
3. impl 所有 8 个 trait 方法 → 全 `bail!("DuckLake backend skeleton, not yet implemented (mission docs/04). Use parquet/surreal in v3.")`
4. mod.rs 在 `#[cfg(feature = "ducklake")]` 守门下 `pub use ducklake::DuckLakeModelWriterBackend;`
5. `create_model_writer` match 加 DuckLake 分支：未启 feature 时 `bail!("ducklake requires --features ducklake build")`，启了 feature 时返回骨架

**Verify**：默认 `cargo check` 不带 feature 跑通；`cargo check --features ducklake` 也跑通。

---

#### Task D.2 — D 阶段 PR（small）

**Steps**：
1. 分支 `feat/ducklake-backend-skeleton`
2. commit：单 commit `feat(model-writer): add DuckLake backend skeleton behind ducklake feature`
3. PR body 强调 "v3 仅骨架，真实写入留 v4，主要为 v4 PR 不动 trait 签名做准备"

---

### Phase E：CLI + SQL validation 全套（独立 PR）

> 依赖：B 完成（Parquet 已可作为 candidate backend 输出 13 张表的数据）。

#### Task E.1 — `validate-model-writer` CLI umbrella

**Files**：`src/cli_modes.rs`（或 model-writer 子模块）

**Steps**：
1. 新增 CLI 子命令族 `aios-database model-writer <subcmd>`：
   - `validate-canonical-parquet`（保留现有，alias）
   - `validate-compare --primary surreal --candidate parquet --dbnum N`：跑 compare 模式生成两份数据并执行 E.2 SQL 检查
   - `diff-summary --left ... --right ...`：仅做 summary JSON 比较
2. 命令出参 JSON 化（mission 07 §2a 已建立模式）

**Verify**：`aios-database model-writer validate-compare --primary surreal --candidate parquet --dbnum <small>` 退出码 0 + JSON 报告。

---

#### Task E.2 — SQL parity scripts

**Files**：
- Create: `scripts/sql/model-writer-parity/*.sql`
- Create: `scripts/sql/model-writer-parity/README.md`

**Steps**：
1. 13 张 Phase 1 表，每张表至少 2 个 SQL：row count + key-set diff（mission 07 §3 列表照抄）
2. 用 DuckDB 读 Parquet/JSONL，配合 SurrealDB export 走 SQL
3. README 给运行示例

**Verify**：手动跑过一遍 SQL，输出文档化到 README。

---

#### Task E.3 — E 阶段 PR

**Steps**：
1. 分支 `feat/model-writer-validation-cli`
2. PR 标题：`feat(model-writer): add validate-compare CLI + Phase 1 SQL parity scripts`

---

### Phase F：P5 backlog 收口（1-2 个 small PR）

#### Task F.1 — `take_missing_neg_carriers` 拆 trait 方法（P5 backlog #1）

**Files**：
- Modify: `src/fast_model/gen_model/model_writer/mod.rs`
- Modify: `src/fast_model/gen_model/model_writer/surreal.rs`
- Modify: `src/fast_model/gen_model/model_writer/mock.rs`
- Modify: `src/fast_model/gen_model/model_writer/parquet.rs`（B 阶段产出）
- Modify: `src/fast_model/gen_model/orchestrator.rs`
- Modify: `src/bin/verify_model_writer_trait.rs`

**Steps**：
1. 在 trait 加 `async fn take_missing_neg_carriers(&self) -> Vec<RefnoEnum> { Vec::new() }`（有 default）
2. `WriteBaseReport` 删 `missing_neg_carriers` 字段
3. SurrealBackend 内部 `Mutex<Vec<RefnoEnum>>`，`write_base_batch` 累计，`take_missing_neg_carriers` drain
4. mock backend 同 pattern + `injected_missing_neg` 走 take
5. orchestrator 改读取方式

**Verify**：`verify-mock.ps1` 退出 0；`cargo check` 通过。

---

#### Task F.2 — `BridgeContext` 抽出（P5 backlog #2）

**Files**：同 F.1（去掉 verify binary）

**Steps**：
1. 新增 `pub struct BridgeContext { pub mode: BridgeMode, pub use_surrealdb: bool, pub defer_db_write: bool, pub bool_tasks_count: usize }`
2. `BooleanBridgeRequest::db_option` 字段删除，换成 `pub context: BridgeContext`
3. orchestrator 构造时填 context
4. SurrealBackend 改读 context（不再走 `Arc<DbOption>`）
5. mock + parquet backend 同步

**Verify**：同 F.1。

---

#### Task F.3 — F 阶段 PR (small × 2)

**Steps**：
1. F.1 单独 PR：`refactor(model-writer): hoist missing_neg_carriers to trait method`
2. F.2 单独 PR：`refactor(model-writer): replace BooleanBridgeRequest::db_option with BridgeContext`
3. F 不收 `async fn in trait` 评估（留 v4 调研）

---

## 6. 验证策略

| 验证类型 | 手段 | 触发时机 |
|---|---|---|
| IDE Lint | `ReadLints` | 每个 Task 后 |
| 编译 | `cargo check --bin plant_db_cli --features review` | 每个 Phase 末 |
| 编译（ducklake 守门） | `cargo check --features ducklake` | D 阶段末 |
| Trait 契约 | `pwsh -NoProfile -File docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1` | B.3 / F.1 / F.2 后 |
| Parquet 落盘 | 跑一个最小 dbnum `model_writer_mode=parquet`，校验 13 张表的 JSONL/summary 完整 | B 阶段末 |
| Compare 模式 | 同 dbnum `compare_with=parquet`，输出 diff JSON | C 阶段末 |
| SQL parity | E.2 SQL 脚本手动跑过一次 | E 阶段末 |
| 集成 | 起 `web_server` 跑 `/api/health` + admin smoke | 每个 Phase PR push 前 |

---

## 7. 错误处理协议 (3-Strike)

```
ATTEMPT 1: 诊断 + 修复，错误写 progress.md "v3 errors" 表
ATTEMPT 2: 换不同实现路径
ATTEMPT 3: 质疑前置假设（如：Parquet 真能 JSONL fallback 走 trait 吗？）
AFTER 3 FAILURES: progress.md 记录三次尝试详情，调 best-mcp-sqlite-16 check_messages 升级用户
```

---

## 8. 约束与边界

- 不动其他 worktree / 不跨仓修改 / 不动主分支未提交内容
- 未经用户明确授权不 `push -f` / 不 amend 已推送 commit
- 不破坏 `[model-writer:*]` 日志契约
- 不动 DrainOnly 快路径（v2 锁定）
- 不引入 `duckdb` / `arrow` / `parquet` crate（D 阶段仅 feature-gated 骨架；真正物化留 v4）
- SurrealDB Cargo source 必须保持 `github.com/happyrust/surrealdb`（mission 00/07）

---

## 9. 完成判定

- [ ] **A**：worktree `git status` 干净（仅留计划性 untracked）；mission docs PR 已开
- [ ] **B**：`model_writer_mode=parquet` 单 backend 跑通最小 dbnum，13 张表 JSONL/summary 输出；verify-mock.ps1 退 0；PR 已开
- [ ] **C**：`compare_with=parquet` 与 surreal 并写，finalize 输出 diff；PR 已开
- [ ] **D**：`--features ducklake` 编过，DuckLake 工厂返回骨架；PR 已开
- [ ] **E**：`validate-compare` CLI + 13 张表的 SQL parity 脚本完整；PR 已开
- [ ] **F**：F.1 + F.2 PR 已开；P5 backlog 中 `async fn in trait` 评估写入 v4 计划文件
- [ ] 所有 PR URL 写入 `progress.md` v3 milestones 表
- [ ] `findings.md` §4 补 v3 期间新发现

---

## 10. 与历史计划的衔接

- v2 (2026-05-11-142958) 闭环 PR #11 不动；v3 新工作走独立 PR 分支
- mission docs Phase 1 完成于 v3 Phase E（CLI + SQL validation 落地）
- mission docs Phase 2 部分兑现（Parquet）于 v3 Phase B；DuckLake 真实写入留 v4
- mission docs Phase 3 主体兑现于 v3 Phase C
- mission docs Phase 4 兑现于 v3 Phase E
- mission docs Phase 5（boolean 表）整体留 v4 或更后

---

## 11. v4 候选议题（不在本 v3 范围）

- DuckLake 真实写入实装（引入 `duckdb` crate + `ducklake-canonical` schema + projection refresh SQL）
- `async fn in trait` 调研报告 + 决策（如做，全 trait 迁移）
- `inst_relate_bool` / `inst_relate_cata_bool` Phase 2 boolean canonical records
- DrainOnly stats Mutex → atomic
- Parquet 真正 `.parquet` 物化（替换 JSONL fallback）
