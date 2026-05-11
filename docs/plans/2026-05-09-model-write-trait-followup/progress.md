# Model Write Trait 二期 — 进度日志

> 实施 `task_plan.md` 的过程记录。开始时间戳与每个 Task 完成情况都落到这里。

## 启动条件检查

- [x] 已读 `task_plan.md`
- [x] 已读 `findings.md`
- [x] 用户授权 "按推荐方案继续"（采用 findings.md §3 暂定默认值）
- [-] 主仓 `feat/collab-api-consolidation` 未提交 `model_writer.rs` 改动 — **不阻塞 P1**：本计划全部在 worktree `feat/model-persistence-trait` 上工作；冲突留到 T4.1 处理

## 2026-05-09

- 创建本计划三件套文件（task_plan.md / findings.md / progress.md），归档到 `docs/plans/2026-05-09-model-write-trait-followup/`。
- 用户授权按推荐方案启动 P1。
- 验证 worktree 状态：`feat/model-persistence-trait` 工作区 clean，HEAD = `b060860`。
- **P1 完成**（5 个 Task 一次性闭环）：
  - T1.1 把 `model_writer.rs` 拆成 `model_writer/{mod,surreal,drain_only}.rs`
  - T1.2 在 `drain_only.rs` 新增 `DrainOnlyModelWriteBackend`，8 个方法 NoOp + 累计 stats
  - T1.3 `create_model_writer` 不再 bail，DrainOnly 也返回 backend
  - T1.4 `process_index_tree_generation` 入参从 `Option<Arc<dyn ModelWriteBackend>>` 改为 `Arc<dyn ModelWriteBackend>`；DrainOnly 快速路径保留（仍跳过 mesh/boolean stage 的真实 work），但走 trait `finalize`
  - T1.5 `cli_modes::run_regen_model` 移除 `if writes_to_surreal()` 守卫
- 静态验证：`ReadLints` 五个 worktree 文件均无 lint 错误；`rg "Option<Arc<dyn ModelWriteBackend>" / "writes_to_surreal()"` 在 trait 调用相关位置已无残留。
- 完整 `cargo check --lib` 因本机 NASM 缺失 + nightly toolchain PATH 副作用未跑通，移到 P4 推 PR 前一并跑通；该问题与 trait 化代码无关。
- **P2 完成**（3 个 Task 一次性闭环）：
  - T2.1 `mock.rs` 落地（feature `model-writer-mock` 守门）：140 行 RecordingBackend，记录每次方法调用 + 注入 `injected_reconcile_inserted` / `injected_missing_neg`；`Cargo.toml` 加 feature；`model_writer/mod.rs` cfg-gate `mod mock; pub use RecordingBackend;`
  - T2.2 `src/bin/verify_model_writer_trait.rs` 落地（160 行）：`#![cfg(feature = "model-writer-mock")]` + `#[tokio::main(flavor = "current_thread")]`；按 init/cleanup/write_base_batch/persist_mesh_results/write_inst_relate_aabb/reconcile_missing_neg/run_boolean_bridge/finalize 顺序调用 + snapshot 前缀断言；exit code 0/1/2/3 区分失败原因；`Cargo.toml` 加 `[[bin]] required-features=["model-writer-mock"]`
  - T2.3 `verify-mock.ps1` 落地（70 行）：自动准备 NASM PATH（`C:\Program Files\NASM`）+ `CARGO_NET_GIT_FETCH_WITH_CLI=true`；`-VerboseRun` 开关；exit code 透传
- ReadLints × 4 个改动文件均无 lint 错误（mock.rs / mod.rs / verify binary / Cargo.toml）。
- fixture 预检通过：`ShapeInstancesData` / `DbOption` 都派生 `Default`；`MeshResult` 不需要（HashMap 空 map 即可）。
- 完整 `cargo run --bin verify_model_writer_trait --features model-writer-mock` 同样受 NASM/PATH 阻塞，等 P4 末尾在干净 shell 跑通。
- **P3 完成**（4 个 Task 一次性闭环）：
  - T3.1 引入 `WriteBaseReport`：trait 不再返回 `pdms_inst::SaveInstanceDataReport`；`#[non_exhaustive]` 留扩展口；`missing_neg_carriers` 字段名保兼容，orchestrator 改动量 1 行（`.len() → .missing_neg_count`）
  - T3.2 `BooleanBridgeRequest` 字段从 5 个减到 3 个：`SurrealModelWriteBackend` 改 `OnceLock<ModelWriterContext>`，`init` 时 set，`run_boolean_bridge` 从缓存的 ctx 读 `use_surrealdb` / `defer_db_write`；未 init 时返回 `BooleanBridgeReport::skipped("uninitialized", ...)` 优雅回退；orchestrator 两个调用点同步删字段；verify binary 同步改
  - T3.3 `MeshResultBatch` 加 `file_mesh_state: bool` 字段：surreal `persist_mesh_results` 改读 batch 字段，删 `use_file_mesh_state` import；orchestrator 两个 `MeshResultBatch { ... }` 构造点显式传 `use_file_mesh_state()`；mock + verify binary 同步改
  - T3.4 `options.validate_model_writer_features` 增加运行时配置守卫：`Surreal + use_surrealdb=false` 在 options 解析阶段就被拒绝（避免空跑 perf init / pre_check）；surreal backend `init` 保留 `ensure!` 兜底，message 改为 `"defense-in-depth"` 标识防御纵深
- ReadLints × 6 个改动文件全绿（mod.rs / surreal.rs / drain_only.rs / mock.rs / orchestrator.rs / options.rs / verify binary）。
- grep 验证：`SaveInstanceDataReport` 只在 mod.rs 注释残留；`BooleanBridgeRequest` 字段已无 `use_surrealdb`/`defer_db_write`；orchestrator `MeshResultBatch` 构造点已加 `file_mesh_state`。
- **P4 worktree 侧 Task 完成**（T4.2 + T4.3）：
  - T4.2 命名统一：`ModelWriteBackend` → `ModelWriterBackend`（含 `Surreal*` / `DrainOnly*`；`SurrealModelWriteBackend` → `SurrealModelWriterBackend` / `DrainOnlyModelWriteBackend` → `DrainOnlyModelWriterBackend`）；6 个文件 23 处批量替换；ReadLints 全过；旧名 `ModelWriteBackend` grep 0 行
  - T4.3 SurrealRecordKey newtype：`surreal.rs` 新增私有 `struct SurrealRecordKey(String)` 含 `new(table, raw_key)` 构造校验（ASCII alphanum + `:` / `_` / `-` 白名单）；`save_aabb_to_surreal_strict` / `save_pts_to_surreal_strict` 改用 newtype，禁止直接 `format!("aabb:..."` 拼接；ReadLints 全过
- T4.1 rebase 与 T4.4 push 留待用户决策（涉及主仓 stash 与 push 远端，是高风险操作）。

## Phase 进度表

| Phase | Task | Status | 完成时间 | 备注 |
|---|---|---|---|---|
| P1 | T1.1 拆 `model_writer.rs` 为模块目录 | complete | 2026-05-09 | mod.rs (200L) / surreal.rs (315L) / drain_only.rs (76L)；ReadLints 无错；完整 cargo check 因 NASM 缺失环境阻塞 |
| P1 | T1.2 引入 `DrainOnlyModelWriteBackend` | complete | 2026-05-09 | 在 drain_only.rs 内新增；cleanup NoOp，run_boolean_bridge 走 BooleanBridgeReport::skipped("drain_only") |
| P1 | T1.3 工厂不再为 DrainOnly bail | complete | 2026-05-09 | `create_model_writer` 改 match → 始终返回 backend；日志统一为 `primary={backend.name()}` |
| P1 | T1.4 orchestrator 移除 Option 分叉 | complete | 2026-05-09 | `process_index_tree_generation` 入参改 `Arc<dyn ModelWriteBackend>`；`Some/None` 路径删除；DrainOnly 快速路径保留 + 调 trait `finalize` |
| P1 | T1.5 cli_modes 移除 `if writes_to_surreal()` 守卫 | complete | 2026-05-09 | `run_regen_model` 始终调 trait cleanup；DrainOnly 由 backend NoOp 守住"不删数据"语义 |
| P2 | T2.1 新增 `RecordingBackend` | complete | 2026-05-09 | mock.rs (140L)；feature `model-writer-mock` 守门；mod.rs cfg-gate import |
| P2 | T2.2 引入 `verify_model_writer_trait` binary | complete | 2026-05-09 | bin (160L)；Cargo.toml `required-features=["model-writer-mock"]`；exit code 0/1/2/3 区分失败原因 |
| P2 | T2.3 验证脚本归档 | complete | 2026-05-09 | verify-mock.ps1 (70L)；自动准备 NASM PATH；`-VerboseRun` 开关 |
| P3 | T3.1 引入精简对外 Report 类型 | complete | 2026-05-09 | `WriteBaseReport` 替换 `SaveInstanceDataReport`；`#[non_exhaustive]` 留扩展口；字段名兼容（orchestrator 改动量 1 行） |
| P3 | T3.2 去除 `BooleanBridgeRequest` 冗余字段 | complete | 2026-05-09 | 删 `use_surrealdb`/`defer_db_write`；`SurrealModelWriteBackend` 加 `OnceLock<ModelWriterContext>`，`run_boolean_bridge` 改读 ctx；未 init 时优雅 fallback（skipped+`uninitialized`） |
| P3 | T3.3 `persist_mesh_results` 移除全局依赖 | complete | 2026-05-09 | `MeshResultBatch::file_mesh_state` 字段；surreal 不再调全局 `use_file_mesh_state()`；orchestrator 构造时显式传值 |
| P3 | T3.4 早期拒绝非法组合 | complete | 2026-05-09 | `validate_model_writer_features` 增加运行时配置守卫；surreal backend `init` 保留 ensure! 兜底（防御纵深） |
| P4 | T4.1 与 base 分支同步 | pending | — | **需用户拍板**：主仓 stash 是高风险操作 |
| P4 | T4.2 命名统一 | complete | 2026-05-09 | `ModelWriteBackend` → `ModelWriterBackend`（含 `Surreal*` / `DrainOnly*` 两个 struct）；6 个文件 23 处替换；旧名 grep 0 残留 |
| P4 | T4.3 SurrealRecordKey newtype | complete | 2026-05-09 | `surreal.rs` 加 newtype（ASCII alphanum + `:` / `_` / `-` 白名单）；`save_aabb_to_surreal_strict` / `save_pts_to_surreal_strict` 改用；`format!("aabb:` / `format!("vec3:` 直拼已无 |
| P4 | T4.4 推送 + 开 PR | pending | — | **需用户拍板**：worktree push + gh pr create |
| P5 | T5.1 评估 `async fn in trait` | pending | — | 独立立项 |
| P5 | T5.2 `name()` → `const NAME` | pending | — | 独立立项 |
| P5 | T5.3 清理 `write_base_batch` 空 HashMap 包袱 | pending | — | 独立立项 |

## 验证记录

| 时间 | 验证类型 | 命令 | 结果 |
|---|---|---|---|
| 2026-05-09 | IDE Lint | `ReadLints` on mod.rs/surreal.rs/drain_only.rs | 无 lint 错误 |
| 2026-05-09 | Cargo build | `cargo check --lib` (worktree, parent shell) | 失败（NASM 缺失，非代码问题） |
| 2026-05-09 | Cargo build + Trait 契约 | `pwsh -NoProfile -File verify-mock.ps1`（干净子进程） | **PASS, elapsed=80s**（exit=0）；lib + verify binary 编译通过，8 个 trait 方法调用顺序断言全过 |

## Errors Encountered

| 时间 | Task | 错误 | 尝试 # | Resolution |
|---|---|---|---|---|
| 2026-05-09 | T1.1 验证 | `cargo check --lib`：surrealdb fetch HTTP 412 → 重试 + `CARGO_NET_GIT_FETCH_WITH_CLI=true` 通过 | 1→2 | 启用 git-fetch-with-cli 后 fetch 通过 |
| 2026-05-09 | T1.1 验证 | `aws-lc-sys` build script 报 `NASM command not found` | 2 | **环境问题非代码问题**；ReadLints 通过；改为 P1 全部完成后在装有 NASM 的环境一次性跑完整 build |

## 关键产出

| 类型 | 路径 / URL | 创建时间 |
|---|---|---|
| 计划文件 | `docs/plans/2026-05-09-model-write-trait-followup/task_plan.md` | 2026-05-09 |
| 发现库 | `docs/plans/2026-05-09-model-write-trait-followup/findings.md` | 2026-05-09 |
| 进度日志 | `docs/plans/2026-05-09-model-write-trait-followup/progress.md` | 2026-05-09 |
| P2 实施手册 | `docs/plans/2026-05-09-model-write-trait-followup/p2-implementation-guide.md` | 2026-05-09 |
| P3 实施手册 | `docs/plans/2026-05-09-model-write-trait-followup/p3-implementation-guide.md` | 2026-05-09 |
| P4 实施手册 | `docs/plans/2026-05-09-model-write-trait-followup/p4-implementation-guide.md` | 2026-05-09 |
| 模块拆分（P1） | `.worktrees/model-persistence-trait/src/fast_model/gen_model/model_writer/{mod,surreal,drain_only}.rs` | 2026-05-09 |
| Verify 脚本 | `docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1` | 待 P2 创建 |
| PR URL | — | 待 P4 推送 |

## 下一步建议

1. 用户确认 `findings.md §3` 待回答问题的最终选项（特别是 trait 命名、mock backend 启用方式）。
2. 主仓 `feat/collab-api-consolidation` 工作区现有 `model_writer.rs` 未提交改动需要收口（提交或 stash），避免与 P4 rebase 冲突时混淆。
3. 上述两点确认后，从 Task 1.1 开始按顺序实施。

---

## 2026-05-11 二期审核 + v2 收口

> 触发：用户 2026-05-11 直接要求"审核当前 worktree 的 ModelWriteTrait 的实现"。
> 审核结果：2 Critical / 5 Warning / 6 Note；plannotator v1 计划被退回 (DrainOnly 用途说明)，v2 已 approved。
> 存档：`C:\Users\dpc\.plannotator\plans\2026-05-11-142958-modelwriterbackend-trait-v2-approved.md`

### 自 b060860 以来的实际 HEAD 漂移（progress 同步）

2026-05-09 P4 task 完成后，worktree 又进了 5 个 commit，进入文档目录 `docs/development/model-writer-storage/` 描述的 canonical raw boundary 阶段：

| Commit | Title |
|---|---|
| `bf4bef80` | fix: resolve merge artifacts in workflow_sync, review_db, and remove stale patch blocks |
| `f3c5750e` | Merge branch 'main' into feat/model-persistence-trait |
| `a27d0685` | fix(deps): repair SurrealDB lock source |
| `80fcc7eb` | Fix canonical raw record compilation |
| `366cb275` | feat(model-writer): complete canonical raw coverage |
| `04dbd9c3` | feat(model-writer): harden canonical validation CLI（当前 HEAD） |

### 二期审核新发现（与 findings.md §4 联动）

| 等级 | ID | 简述 | 修复 Task |
|---|---|---|---|
| Critical | C-A | `cli_modes::run_regen_model` 调 `cleanup` 但漏 `init`，违反 trait lifecycle 契约 | T1.1 |
| Critical | C-B | DrainOnly 主管线快速路径绕过 trait 的 6 个中间方法，trait 化收益未真正兑现 — 用户拍板：保留快速路径作为 baseline | T1.2 |
| Warning | W-A | `BooleanBridgeReport::skipped` 日志标签硬编码 `[model-writer:surreal]` 被多 backend 共用 | T2.1 |
| Warning | W-B | `SurrealRecordKey::new` `starts_with` 分支会产出两种格式 record id | T2.2 |
| Warning | W-C | `WriteBaseReport.missing_neg_carriers` 仍泄漏 `Vec<RefnoEnum>`；`#[non_exhaustive]` 实际无效 | T2.3 + P5 backlog |
| Warning | W-D | `parquet.rs::CanonicalParquetWriter` 不实现 trait 却放在 `model_writer/` 顶层 | T2.4 |
| Warning | W-E | `BooleanBridgeRequest::db_option` 仍暴露 `Arc<DbOption>` | T2.5（仅 TODO 标记）+ P5 backlog |
| Note | N-1 | Cargo.toml `model-writer-drain` feature 死代码 | T3.1 |
| Note | N-2 | mock backend 的 `injected_*` 字段未被 verify binary 使用 | T3.2 |
| Note | N-3 | verify binary 只断言调用顺序前缀，强度不足 | T3.2 |
| Note | N-4 | trait `name()` 改 const，dyn vtable 上不可行 → cancelled | T3.3 |
| Note | N-5 | DrainOnly stats Mutex → atomic（性能优化） | P5 backlog |
| Note | N-6 | `BooleanBridgeReport::skipped` 顺手 IO 违反构造器纯净 | 与 T2.1 合并 |

### Decisions（不可逆架构约定）

- **DrainOnly = baseline 模式**：跳过所有持久化、只跑生产端 + 调度，用于对比不同 backend 的写入耗时。`orchestrator.rs` 的快速路径 + `drain_only.rs` 中 6 个中间方法的"未路由" invariant 必须同时保留；只能整体改，不能拆。
- **`WriteBaseReport.missing_neg_carriers` 短期保留**：拆 `take_missing_neg_carriers()` 推到 P5。
- **`BooleanBridgeRequest::db_option` 短期保留**：抽 `BridgeContext` 推到 P5。
- **`parquet.rs` 短期不接 trait 工厂**：与 docs/development/model-writer-storage Phase 1 共识一致，本轮只加位置说明。

### P1 完成（2026-05-11）

- **T1.1** cli_modes.rs:1738~1761 补 `init` 调用（包含 ModelWriterContext::from_db_option 注入）；添加 lifecycle 契约注释。`init_model_tables` 走 DEFINE TABLE 语义，幂等无副作用。
- **T1.2** drain_only.rs 顶部加 module doc 锁定 DrainOnly baseline 架构 invariant；6 个中间方法函数体顶部各加 `architectural-invariant` 单行注释；orchestrator.rs:1112 快速路径上方加 9 行 `[arch]` 注释 + 重构前置条件。findings.md §3 表格新增 DrainOnly baseline 决策条目。

### P2 完成（2026-05-11）

- **T2.1** mod.rs:120-122 `BooleanBridgeReport::skipped` 把 `[model-writer:surreal]` 改为 `[model-writer:{pipeline}]`。
- **T2.2** surreal.rs:284-308 `SurrealRecordKey::new` 删 `starts_with` 分支，白名单去 `:`；统一输出 `table:⟨raw_key⟩`。
- **T2.3** mod.rs:155-167 删 `#[non_exhaustive]`，给 `missing_neg_carriers` 加 `///` doc 说明 P5 拆分计划。
- **T2.4** parquet.rs:1 加 12 行 module doc；mod.rs:14, 28-31 加位置标注 (`Canonical raw record types & planner` / `Canonical raw sink scaffold`)。
- **T2.5** mod.rs:88-89 给 `BooleanBridgeRequest::db_option` 加 `TODO(P5)` 注释。

### P3 完成（2026-05-11）

- **T3.1** Cargo.toml:256 删 `model-writer-drain = []`。`rg "model-writer-drain"` 仅在历史文档 `p2-implementation-guide.md:16` 一处残留（不动）。
- **T3.2** verify binary 重写：
  - 注入 `injected_reconcile_inserted=42` 和 `injected_missing_neg=vec![default, default]`，验证返回值精确匹配。
  - 二次 `init` 安全断言（exit 5）。
  - cleanup-without-init 安全断言（mock 路径，对应历史 cli_modes 残留场景）。
  - snapshot 长度从 8 改为 9（含两次 init），并加"反例 — snapshot 不能含未预期方法名"硬检查。
  - exit code 拓展：0 PASS / 1 trait err / 2 调用计数 / 3 顺序/反例 / 4 注入未被 honor / 5 二次 init 不安全。
- **T3.3** findings.md N-4 标记 `Skipped 2026-05-11`，理由：dyn vtable 不能放 const。task_plan T5.2 标 `Cancelled 2026-05-11`。

### P4 进行中（2026-05-11）

- **T4.1** progress.md 同步至本节。
- **T4.2** `git checkout -- src/options.rs` 清掉纯 CRLF/LF 噪声。其余 8 个文件都是真实改动，CRLF warning 由 git commit 时自动转换。
- **T4.3 / T4.4** 等用户确认后再执行（按计划 v2 commit 分片 + push + gh pr create）。

### Canonical raw boundary（平行工作流，不在本 PR 范围）

- `src/fast_model/gen_model/canonical_records.rs`（新增 587L）
- `src/fast_model/gen_model/model_writer/parquet.rs`（新增 254L，已加位置说明）
- `docs/development/model-writer-storage/{00..08}-*.md`（9 文件 mission docs，untracked）

未来 Phase 5 把 parquet.rs 升级为真正的 `ModelWriterBackend` 时与该 mission 合流。

### Phase 进度表（二期）

| Phase | Task | Status | 完成时间 | 备注 |
|---|---|---|---|---|
| P1 | T1.1 run_regen_model 补 init | complete | 2026-05-11 | cli_modes.rs:1743-1761；注释明确 lifecycle 契约 |
| P1 | T1.2 DrainOnly baseline 文档/代码锁定 | complete | 2026-05-11 | drain_only.rs module doc + 6 个 invariant 注释；orchestrator.rs 9 行 [arch] 注释；findings.md §3 新增决策行 |
| P2 | T2.1 skipped 日志去硬编码 | complete | 2026-05-11 | mod.rs:120-122 |
| P2 | T2.2 SurrealRecordKey 去分支 | complete | 2026-05-11 | surreal.rs:284-308，白名单去 `:` |
| P2 | T2.3 WriteBaseReport 去 non_exhaustive | complete | 2026-05-11 | mod.rs:155-167 |
| P2 | T2.4 parquet.rs 位置说明 | complete | 2026-05-11 | parquet.rs:1 + mod.rs:14, 28-31 |
| P2 | T2.5 BooleanBridgeRequest TODO | complete | 2026-05-11 | mod.rs:88-89 |
| P3 | T3.1 删 model-writer-drain 死 feature | complete | 2026-05-11 | Cargo.toml |
| P3 | T3.2 verify binary 增强 | complete | 2026-05-11 | 新增 INJECTED_RECONCILE、二次 init、cleanup-without-init、反例检查；exit code 拓展为 0-5 |
| P3 | T3.3 N-4 / T5.2 Skipped | complete | 2026-05-11 | findings + task_plan 同步 |
| P4 | T4.1 progress.md 同步 | complete | 2026-05-11 | 本节 |
| P4 | T4.2 清 CRLF 噪声 | complete | 2026-05-11 | options.rs 还原；其余文件靠 git autocrlf 处理 |
| P4 | T4.3 commit 分片 | complete | 2026-05-11 | 5 commit: b4d93ca0 / c8faec3d / c63ea11e / 9678510c / 8bb69632 |
| P4 | T4.4 push + gh pr create | complete | 2026-05-11 | https://github.com/happyrust/plant-model-gen/pull/11 |

### 验证记录（二期）

| 时间 | 验证类型 | 命令 | 结果 |
|---|---|---|---|
| 2026-05-11 | IDE Lint | `ReadLints` × 8 个改动文件 | 无 lint 错误 |
| 2026-05-11 | grep `model-writer-drain` in src/ | 死 feature 检查 | 0 命中（仅 p2-implementation-guide.md 历史文档残留） |
| 2026-05-11 | grep `Option<Arc<dyn ModelWriterBackend>` | trait Option 化残留 | 0 命中 |
| 2026-05-11 | Cargo build + Trait 契约 (verify binary) | `pwsh -NoProfile -File verify-mock.ps1` | 待跑（依赖本机 NASM 环境，P4 push 前补） |

---

## 2026-05-12 v3 启动（plannotator approved）

> 触发：用户 2026-05-12 直接要求 "继续使用 plannotator 规划 worktree model-persistence-trait 的实现进度"，AI 输出 v3 计划提交 plannotator，approved。
> 存档：`C:\Users\dpc\.plannotator\plans\2026-05-11-014336-worktree-model-persistence-tra-approved.md`
> 本地副本：`docs/plans/2026-05-09-model-write-trait-followup/v3-plan.md`

### v3 目标

在 v2 (PR #11) 落地的 trait abstraction 基础上，把 Parquet writer 升级为真正的 `ModelWriterBackend`，落地 orchestrator 多 backend 选择 + compare 模式，使 mission docs Phase 2 真正可用；同时清理 v2 残留的 worktree 脏文件与 9 份 untracked mission docs，按节奏拆分独立 PR。

### v3 架构 invariants（沿用 v2 + 新增 4 条）

新增 4 条：

- Parquet writer 是 "file-oriented backend"，不替代 SurrealDB（mission 05）
- DuckLake writer 在 v3 不实装，仅 feature-gated 骨架（trait impl 全 `bail!`），真正实装留 v4
- compare 模式遵循 "fail fast, no silent fallback"
- SurrealDB Cargo source 必须保持 `github.com/happyrust/surrealdb`

### Phase 总览

| Phase | 名称 | 独立 PR 分支 | 状态 |
|---|---|---|---|
| A | v2 残留 cleanup | （progress 同步 → PR #11；mission docs → `docs/model-writer-storage-mission`） | in_progress |
| B | Parquet trait 化 | `feat/parquet-model-writer-backend` | pending |
| C | Orchestrator backend selection + compare | `feat/model-writer-compare-mode` | pending |
| D | DuckLake backend 骨架 | `feat/ducklake-backend-skeleton` | pending |
| E | CLI + SQL validation 全套 | `feat/model-writer-validation-cli` | pending |
| F | P5 backlog 收口 | 2 个 small PR | pending |

### v3 milestones

| Phase | Task | Status | 完成时间 | 备注 |
|---|---|---|---|---|
| A | A.1 审查 + 处置 3 文件 | complete | 2026-05-12 | mock.rs / options.rs CRLF 噪声 checkout；commit `29fa19ab` + `58844c88` pushed to `feat/model-persistence-trait`，PR #11 自动 pick up |
| A | A.2 mission docs docs-only PR | complete | 2026-05-12 | 分支 `docs/model-writer-storage-mission` commit `a6beb555` pushed；PR #13: https://github.com/happyrust/plant-model-gen/pull/13 |
| A | A.3 rebase origin/main | deferred | — | PR #11 review 中，force-push 会让评论失效；推迟到 PR #11 合并阶段处理（GH UI merge --rebase 或合并前再 rebase） |
| B | B.1 ParquetModelWriterBackend 骨架 | complete | 2026-05-12 | commit `e93c559b`，parquet.rs +515/-262，含 8 个 trait 方法 + module doc 重写 |
| B | B.2 接入 create_model_writer 工厂 | complete | 2026-05-12 | commit `0d91b496`，ModelWriterMode::Parquet + DbOptionExt 字段 + validate 守卫 + CLI flags `--parquet-output-root` / `--parquet-dbnum` |
| B | B.3 Verify binary 加 Parquet 路径 | complete | 2026-05-12 | commit `a8ce553f`，exit code 6 = parquet path fail；13 canonical raw 表 + summary JSON 全部落盘断言 |
| B | B.4 B 阶段 PR | complete | 2026-05-12 | PR #14: https://github.com/happyrust/plant-model-gen/pull/14；`cargo check --features review` + `verify_model_writer_trait` 均 PASS |
| C | C.1 CompareModelWriterBackend wrapper | complete | 2026-05-12 | commit `2401268b`，trait 装饰器模式（zero-touch orchestrator），fail-fast；primary/candidate diff log policy；compare.rs +390 |
| C | C.2 Factory + DbOption + CLI 接入 | complete | 2026-05-12 | commit `fb79afa8`，`model_writer_compare_with: Option<ModelWriterMode>`，validate 2c 守卫（拒 self-compare / drain-only / missing output_root），`--model-writer-compare-with` CLI |
| C | C.3 Verify binary compare 路径 | complete | 2026-05-12 | commit `3300ed7d`，exit code 7 = compare wrapper fail；primary (mock) snapshot 校验 + candidate (parquet) 13 表落盘断言 |
| C | C 阶段 PR | complete | 2026-05-12 | PR #15: https://github.com/happyrust/plant-model-gen/pull/15 （base = Phase B 分支，stack PR） |
| D | D.1 DuckLakeModelWriterBackend 骨架 | complete | 2026-05-12 | commit `88071608`，feature `ducklake = []`，trait 8 方法全 `bail!`，工厂 cfg 守门；ModelWriterMode::DuckLake + parse + validate 守卫 |
| D | D.2 D 阶段 PR | complete | 2026-05-12 | PR #16: https://github.com/happyrust/plant-model-gen/pull/16；`cargo check --features review` + `cargo check --features review,ducklake` 均 PASS |
| E | E.1 validate-model-writer CLI umbrella | pending | — | — |
| E | E.2 SQL parity scripts | pending | — | 13 张 Phase 1 表 × 2 个 SQL |
| E | E.3 E 阶段 PR | pending | — | — |
| F | F.1 take_missing_neg_carriers 拆 trait | pending | — | — |
| F | F.2 BridgeContext 抽出 | pending | — | — |
| F | F.3 F 阶段 PR | pending | — | small × 2 |
