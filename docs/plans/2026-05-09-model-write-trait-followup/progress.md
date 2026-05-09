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
