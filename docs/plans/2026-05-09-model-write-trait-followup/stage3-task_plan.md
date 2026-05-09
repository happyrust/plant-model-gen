# Model Write Trait 三期 — Commit + Rebase + Push + 长期项立项

> **For Agent**：本文件继 `task_plan.md`（二期）后继续推进；二期已完成 P1-P4 的代码侧 Tasks，剩 T4.1（rebase）/ T4.4（push+PR）/ P5 立项三块。本文件为这三块的执行手册。

**Goal**：把 `feat/model-persistence-trait` worktree 上**未 commit 的 12 个 Tasks** 切成原子 commits、rebase 到当前 `feat/collab-api-consolidation`（HEAD 已升到 `005b943b`，含 2026-05-09 上午做的 surrealdb URL 统一），跑双验证（cargo check + verify-mock.ps1），push 远端，开 PR；同时为 P5 三个长期项各建独立 plan dir。

**Architecture**：worktree 以 4 个 atomic Phase commits 落地 P1-P4，PR 一次性合并；主 feat 上未提交的 `model_writer.rs` WIP 保持独立、不在本轮纳入。

**Tech Stack**：Rust nightly（项目要求）、async_trait、tokio、anyhow、SurrealDB、PowerShell、sccache + NASM 或 `AWS_LC_SYS_NO_ASM=1` workaround、`gh` CLI（PR 创建）。

---

## 1. 当前基线（2026-05-09 晚间）

| 维度 | 状态 |
|---|---|
| Worktree | `.worktrees/model-persistence-trait`（branch `feat/model-persistence-trait`） |
| Worktree HEAD | `b060860 feat(model): introduce surreal writer backend trait` |
| Worktree 已提交领先 base | 1 commit（base 当时 = `f0aedb6`） |
| Worktree 未提交改动 | M Cargo.toml / M cli_modes.rs / M orchestrator.rs / M options.rs / D model_writer.rs ; ?? model_writer/ 目录（4 文件）/ ?? src/bin/verify_model_writer_trait.rs |
| Base 分支 | `feat/collab-api-consolidation` 已 push 到 `005b943b`（fix(deps): surrealdb URL gitee→github + CHANGELOG） |
| Worktree 还未拉到 base 新 commit | 是；rebase 后 base 变 `005b943b` |
| Worktree push 状态 | **未 push origin**（branch `feat/model-persistence-trait` 不存在于 origin） |
| 二期完成度 | P1-P4 代码侧 12 Tasks 完成（详见 progress.md），剩 T4.1 / T4.4 / P5 |

---

## 2. 范围与不做事项

**做**：

1. 把 worktree 12 个 Tasks 切成 **4 个 atomic Phase commits**（P1 / P2 / P3 / P4 各一）；如 P1 与 P3 改动文件高度重叠且 hunk 不易切，允许合并为 P1+P3 单 commit。
2. Rebase worktree 到 `feat/collab-api-consolidation` 的最新 HEAD `005b943b`。
3. 双验证：`cargo check --lib`（worktree，含 NASM 或 NO_ASM）+ `verify-mock.ps1`（PASS）。
4. `git push -u origin feat/model-persistence-trait`。
5. `gh pr create`，PR body 引用本计划与 `findings.md` 全部 review 结论与验证结果。
6. P5 三个长期项各建独立 plan dir + 骨架 task_plan.md：T5.1（async fn in trait 评估）、T5.2（`name()` → `const NAME`）、T5.3（`write_base_batch` 空 mesh_results 包袱清理）。

**不做**：

- 不 push 到 default 分支 `main`（PR 流程做合并，不直接 push）。
- 不动主 feat `feat/collab-api-consolidation` 上的 `model_writer.rs` WIP（属于另一支线）。
- 不在本轮跑 `cargo test`（按 `AGENTS.md`：web_server / aios-database 不跑 test）。
- 不在本轮做 `async_trait` → 原生 `async fn in trait` 迁移（拆 P5 立项）。
- 不重写 `pdms_inst::*` 底层（继续作为 SurrealBackend 的私有依赖）。
- 不跨仓改动 `plant3d-web` / `rs-core` / `pdms-io-fork`。

---

## 3. 阶段总览

| Phase | 名称 | 核心交付物 | 预估 |
|---|---|---|---|
| S3-P1 | 切 commit | worktree 上 4 个 atomic Phase commits（合并允许 3 个） | 1-2h |
| S3-P2 | Rebase 到新 base | worktree HEAD = `005b943b` + 4-5 commits | 30min（顺利）/ 2h（有冲突） |
| S3-P3 | 双验证 | `cargo check` 通过 + `verify-mock.ps1` PASS（rebase 后） | 30min |
| S3-P4 | Push + PR | `origin/feat/model-persistence-trait` + PR URL 落档 | 30min |
| S3-P5 | P5 长期项立项 | T5.1 / T5.2 / T5.3 各 1 plan dir | 1h |

总计 4-7h（取决于 rebase 是否冲突 + NASM 环境是否就绪）。

---

## 4. 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| Cargo.toml rebase 冲突（worktree 加 `[features] model-writer-mock` line ~190+；base 改 line 67/72 surrealdb URL） | 中 | 两者不同段落，应自动合并；若手解则保留 worktree feature + base URL |
| Commit 切片粒度过细易乱、过粗丢失 P 间界限 | 中 | 严格按 Phase 切；P1 与 P3 改动文件重叠（orchestrator/options/mod.rs 等）允许合并为单 commit，但 commit message 必须列出 P1+P3 双串号 |
| `verify-mock.ps1` 在 NASM 缺失环境失败（已知风险） | 低 | 脚本已尝试自动 PATH `C:\Program Files\NASM`；若仍缺，临时设 `AWS_LC_SYS_NO_ASM=1` workaround；若装有 NASM，移除 workaround |
| Rebase 后 web_server 编译需要重新 link aws-lc-sys（13min 量级） | 中 | 仅在 PR review 要求时跑 `cargo build --bin web_server`；正常 PR 走 lib check + verify binary 即可 |
| 主 feat WIP `model_writer.rs` 未来与本 PR merge 冲突 | 中 | 不在本轮处理；PR description 显式声明：合并本 PR 后主 feat 上对 `model_writer.rs`（已删）的 WIP 需重定向到 `model_writer/{mod,surreal,drain_only,mock}.rs` 对应位置 |
| `gh` CLI 未登录或无 `gh` | 低 | 备选：手工通过 GitHub Web 界面建 PR；URL 仍写回 progress.md |

---

## 5. 详细任务

### Phase S3-P1：切 commit

#### Task S3.1.1 — Commit P1（闭环抽象）

**Files**：

- Tracked modified：`src/cli_modes.rs`、`src/fast_model/gen_model/orchestrator.rs`（**仅 P1 hunks**——T1.4 移除 Option/T1.5 移除 writes_to_surreal）
- Tracked deleted：`src/fast_model/gen_model/model_writer.rs`
- Untracked new：`src/fast_model/gen_model/model_writer/{mod,surreal,drain_only}.rs`

**Steps**：

1. `git -C ".worktrees/model-persistence-trait" add src/fast_model/gen_model/model_writer/{mod,surreal,drain_only}.rs`
2. `git add -u src/fast_model/gen_model/model_writer.rs`（记录 deletion）
3. `git add -p src/cli_modes.rs`：仅取 `if writes_to_surreal()` 守卫拆除的 hunks
4. `git add -p src/fast_model/gen_model/orchestrator.rs`：仅取 `Option<Arc<dyn>>` 拆除 + DrainOnly 路径调整 hunks
5. `git -c user.name=happyrust -c user.email=golinuxlove@gmail.com commit -F` 写入消息：

   ```
   feat(model-writer): close trait abstraction (P1: drain-only as backend, remove Option/Arc dispatch)

   - 拆 model_writer.rs (577L) 为 model_writer/{mod,surreal,drain_only}.rs (200/315/76L)
   - DrainOnly 走 trait（cleanup NoOp），create_model_writer 不再 bail
   - process_index_tree_generation 入参 Option<Arc<dyn>> → Arc<dyn>
   - cli_modes::run_regen_model 移除 if writes_to_surreal() 守卫
   ```

**Verify**：`cargo check --lib`（worktree）通过 + `git diff HEAD~1 --stat` 仅这几个文件。

---

#### Task S3.1.2 — Commit P2（Mock + verify binary）

**Files**：

- Tracked modified：`Cargo.toml`（feature `model-writer-mock` + `[[bin]] verify_model_writer_trait`）
- Untracked new：`src/fast_model/gen_model/model_writer/mock.rs`、`src/bin/verify_model_writer_trait.rs`

**Steps**：

1. `git add Cargo.toml src/fast_model/gen_model/model_writer/mock.rs src/bin/verify_model_writer_trait.rs`
2. Commit：

   ```
   feat(model-writer): mock backend + verify binary (P2)

   - mock.rs: RecordingBackend (140L), feature-gated by model-writer-mock
   - src/bin/verify_model_writer_trait.rs: 8 method snapshot assertion
   - mod.rs: cfg-gate mock import
   ```

**Verify**：`pwsh verify-mock.ps1` PASS（exit 0）。

---

#### Task S3.1.3 — Commit P3（接口纯化）

**Files**：

- Modified：`src/fast_model/gen_model/model_writer/{mod,surreal,drain_only,mock}.rs`、`src/fast_model/gen_model/orchestrator.rs`、`src/options.rs`、`src/bin/verify_model_writer_trait.rs`

**Steps**：

1. 评估 hunks 与 P1/P2 重叠度。允许两种切法：
   - **A 方案（推荐，3 commits 总）**：P3 hunks 与 P1 共享文件高度重叠，并入 P1 commit，message 改为 `feat(model-writer): close trait abstraction + purify interface (P1+P3)`，T2 单独。
   - **B 方案（4 commits 总）**：用 `git add -p` 精切 P3 hunks（WriteBaseReport / OnceLock<ModelWriterContext> / file_mesh_state / validate_model_writer_features），独立 commit。
2. 默认走 A；若 hunk diff 视觉清晰、可机械分离则走 B。
3. （走 B 时）Commit message：

   ```
   refactor(model-writer): purify trait interface (P3: WriteBaseReport, OnceLock ctx, no global state, early validation)

   - WriteBaseReport 替换 SaveInstanceDataReport，不暴露 pdms_inst 内部
   - SurrealModelWriterBackend 加 OnceLock<ModelWriterContext>，BooleanBridgeRequest 删 use_surrealdb/defer_db_write
   - MeshResultBatch 加 file_mesh_state 字段，移除全局 use_file_mesh_state() 依赖
   - options.validate_model_writer_features 增加 Surreal+use_surrealdb=false 早期拒绝
   ```

**Verify**：`cargo check --lib` + `verify-mock.ps1` 双 PASS。

---

#### Task S3.1.4 — Commit P4（命名 + newtype）

**Files**：

- 6 文件 23 处 rename：`ModelWriteBackend` → `ModelWriterBackend`（含 `SurrealModelWriteBackend` → `SurrealModelWriterBackend` 等）
- `src/fast_model/gen_model/model_writer/surreal.rs`：`SurrealRecordKey` newtype 引入

**Steps**：

1. `git add` 全部 rename + newtype 改动
2. Commit：

   ```
   refactor(model-writer): rename to ModelWriterBackend + SurrealRecordKey newtype (P4)

   - 命名统一: ModelWriteBackend → ModelWriterBackend (含 Surreal*/DrainOnly* 派生)
   - SurrealRecordKey: ASCII alphanum + : / _ / - 白名单，禁止 format! 直拼
   - save_aabb/save_pts to_surreal_strict 改用 newtype
   ```

**Verify**：`cargo check --lib` + `rg "ModelWriteBackend" .` 0 残留 + `verify-mock.ps1` PASS。

---

### Phase S3-P2：Rebase

#### Task S3.2.1 — Pre-rebase verify（在 b060860 + 4 commits 上）

```powershell
cd D:/work/plant-code/plant-model-gen/.worktrees/model-persistence-trait
$env:CARGO_NET_GIT_FETCH_WITH_CLI = 'true'
$env:AWS_LC_SYS_NO_ASM = '1'  # 若机器无 NASM
cargo check --lib 2>&1 | Select-String 'error\[E'
pwsh -NoProfile -File ../../docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1
```

**Expected**：error 行数 0；verify-mock.ps1 输出 `[verify] PASS`。

---

#### Task S3.2.2 — Rebase 到 005b943b

```powershell
git -C .worktrees/model-persistence-trait fetch origin
git -C .worktrees/model-persistence-trait -c safe.directory=* rebase 005b943b
```

冲突预案（Cargo.toml）：

| 区段 | 取舍 |
|---|---|
| `surrealdb` 与 `surrealdb-types` URL（line 67, 72） | 取 base（github.com/happyrust/surrealdb）|
| `[features] model-writer-mock`（worktree 加） | 保留 worktree |
| `[[bin]] verify_model_writer_trait` 与 `required-features` | 保留 worktree |
| 其他段落 | 默认 base |

**Verify**：`git status` 无 unmerged；`git log --oneline origin/feat/collab-api-consolidation..HEAD` 显示 5 个 commits（b060860 旧 + S3.1.1-S3.1.4 共 4 个新）。

---

### Phase S3-P3：双验证（rebase 后）

#### Task S3.3.1 — `cargo check --lib` post-rebase

参数同 S3.2.1，`error\[E` 必须为 0。

#### Task S3.3.2 — `verify-mock.ps1` post-rebase

`[verify] PASS` 与 8 trait 调用顺序断言通过。

---

### Phase S3-P4：Push + PR

#### Task S3.4.1 — push origin

```powershell
git -C .worktrees/model-persistence-trait push -u origin feat/model-persistence-trait
```

**Expected**：远端有新 branch；`git -C ... rev-parse --abbrev-ref --symbolic-full-name @{u}` 输出 `origin/feat/model-persistence-trait`。

#### Task S3.4.2 — `gh pr create`

```powershell
gh pr create `
  --base feat/collab-api-consolidation `
  --head feat/model-persistence-trait `
  --title "feat(model): close ModelWriter trait abstraction (P1-P4)" `
  --body-file docs/plans/2026-05-09-model-write-trait-followup/pr-body.md
```

PR body 文件需包含：

- 二期 task_plan.md / findings.md / progress.md 链接
- 4 个 Phase 的代码改动摘要
- 验证结果（cargo check + verify-mock.ps1）
- 已知遗留（主 feat WIP `model_writer.rs` 后续 merge 注意事项）

#### Task S3.4.3 — PR URL 写回 `progress.md`

在 progress.md 「关键产出」表 PR URL 行更新。

---

### Phase S3-P5：P5 长期项立项

#### Task S3.5.1 — T5.1 立项：`async fn in trait` 评估

新建 `docs/plans/2026-05-09-async-fn-in-trait-evaluation/task_plan.md`，含：

- Goal：评估 nightly 原生 `async fn in trait` 替换 `async_trait` 是否值得
- Baseline：当前 `async_trait` 在 hot path 每个 batch 过 `Pin<Box<dyn Future>>`
- 范围：仅评估，不立即迁移；产出 ADR-style 结论
- 验证：基准测试（仅评估文档需要时）

#### Task S3.5.2 — T5.2 立项：`name()` → `const NAME`

新建 `docs/plans/2026-05-09-model-writer-const-name/task_plan.md`，含：

- Goal：trait 改为关联常量 `const NAME: &'static str`
- 影响面：4 个 backend impl + 调用点 1 处
- 验证：`cargo check`，无功能变化

#### Task S3.5.3 — T5.3 立项：`write_base_batch` 空 mesh_results 包袱

新建 `docs/plans/2026-05-09-write-base-batch-cleanup/task_plan.md`，含：

- Goal：`pdms_inst` 加 `save_instance_data_base_only`，trait 实现内不再造空 HashMap
- 影响面：`pdms_inst` API 增 + `SurrealModelWriterBackend::write_base_batch` 改

---

## 6. 验证策略

| 阶段 | 验证 | 手段 | 预期 |
|---|---|---|---|
| S3-P1 每个 commit 后 | 编译 | `cargo check --lib`（worktree） | error\[E 0 行 |
| S3-P1 P2/P3 commit 后 | 契约 | `verify-mock.ps1` | exit 0 + `[verify] PASS` |
| S3-P2 rebase 后 | 编译 + 契约 | 同上 | 同上 |
| S3-P4 push 前 | 集成（可选） | `cargo check --bin web_server` | error 0 行（首次需 NASM/NO_ASM） |
| PR review 期 | 视 reviewer 要求 | 任意 | 视情况 |

---

## 7. 错误处理协议（沿用二期 3-Strike）

```
ATTEMPT 1: Diagnose & Fix → 读 stack，定位根因，写入 progress.md
ATTEMPT 2: Alternative Approach → 同错复现，换路径
ATTEMPT 3: Broader Rethink → 质疑前置假设，可能拆 Task 更细
AFTER 3 FAILURES: progress.md 记录三次尝试 + check_messages 升级用户
```

---

## 8. 约束与边界

- **不动其他 worktree**：`pe-transform-backends` / `perf-cata` / `perf-mesh` / `perf-scheduler` / `perf-sink` / `room-compute-3x` 一律不动。
- **不跨仓改动**：`plant3d-web` / `rs-core` / `pdms-io-fork` 一律不动。
- **不动主 feat 工作区 WIP**：`feat/collab-api-consolidation` 现有 `model_writer.rs` 等改动是站点部署相关，与本计划独立，merge 时序由后续 plan 决定。
- **不破坏既有日志契约**：`[batch_perf]` `[model-writer:*]` 等日志格式保留。
- **PR 不直接合并**：PR 走正常 review 流程，不在本计划内做 self-merge。

---

## 9. 完成判定

- [ ] worktree HEAD = `005b943b` + 4-5 commits
- [ ] `cargo check --lib`（rebased）error 0 行
- [ ] `verify-mock.ps1`（rebased）exit 0 + `[verify] PASS`
- [ ] `origin/feat/model-persistence-trait` 存在
- [ ] PR URL 落档到 `progress.md` 关键产出表
- [ ] T5.1 / T5.2 / T5.3 各有 1 个 plan dir + task_plan.md 骨架
- [ ] 主 feat 工作区 model_writer.rs WIP 未被 disturb（grep 确认未改动）
