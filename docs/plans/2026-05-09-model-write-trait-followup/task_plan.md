# Model Write Trait 二期收口计划

> **For Agent:** 本文件是结构化数据，不是直接执行指令；所有任务按 `## 5. 详细任务` 的顺序逐个执行，每完成一个 Task 立即更新 `progress.md`。

**Goal**：把 `feat/model-persistence-trait`（HEAD `b060860`）的 trait 抽象闭环，使 trait 真正承担"统一持久化抽象"的职责——支持多 backend、可被 mock 验证、接口与具体后端解耦——并完成工程化推送。

**Architecture**：以 `aios_database::fast_model::gen_model::model_writer::ModelWriteBackend` 为唯一管线写入抽象，所有 backend 实现位于 `model_writer/` 模块内；调用方（orchestrator、cli_modes）只持有 `Arc<dyn ModelWriteBackend>`，不再做模式分叉。

**Tech Stack**：Rust nightly（项目要求）、tokio、async_trait、anyhow、SurrealDB、PowerShell（Windows 验证脚本）。

---

## 1. 当前基线

| 维度 | 状态 |
|---|---|
| Worktree | `.worktrees/model-persistence-trait`（feat/model-persistence-trait） |
| HEAD | `b060860 feat(model): introduce surreal writer backend trait` |
| 领先 base | 1 commit（base = `f0aedb6`） |
| 推送状态 | **未 push origin**，仅本地 |
| Trait 方法数 | 8（init / cleanup / write_base_batch / persist_mesh_results / write_inst_relate_aabb / reconcile_missing_neg / run_boolean_bridge / finalize） |
| Backend 实现数 | 1（SurrealModelWriteBackend） |
| 调用方分叉 | 仍有 `Option<Arc<dyn ModelWriteBackend>>` 与 `if writes_to_surreal()` 二分支 |
| 测试覆盖 | 无 |

详细 review 结论见同目录 `findings.md`，分为 2 项 Critical / 6 项 Warning / 7 项 Note。

## 2. 范围与不做事项

**做**：

1. 把 DrainOnly 纳入 trait（解决 Critical C1）。
2. 引入 Mock backend + 最小契约验证（解决 Critical C2 + Warning W5）。
3. 接口纯化：移除 `SaveInstanceDataReport` / `Arc<DbOption>` 等内部类型在 trait 接口的直接暴露（解决 Warning W1/W2/W3）。
4. 早期校验：把 `Surreal + use_surrealdb=false` 拒绝时机前移到 options 校验（W4）。
5. 工程化：拆文件、命名统一、推 PR（W6 + Note）。

**不做**：

- 不跑 `cargo test`（按 `AGENTS.md`：web_server/aios-database 一律不编译/不跑 test）。
- 不动 `feat/collab-api-consolidation` 主分支上未提交的 `model_writer.rs` 改动（避免跨任务污染）。
- 不在本计划内迁移 `async_trait` → 原生 `async fn in trait`（性能优化项 N3，单独立项）。
- 不重写 `pdms_inst::*` 底层函数（保留作为 SurrealBackend 的私有依赖）。
- 不跨仓修改（`plant3d-web` / `rs-core` / `pdms-io-fork` 不动）。

## 3. 阶段总览

| Phase | 名称 | 解决的问题 | 预估改动量 |
|---|---|---|---|
| P1 | 闭环抽象 | C1 + N2 | 拆 1 文件为 3 文件，新增 1 个 backend，去 ~30 行调用方分叉 |
| P2 | Mock 与契约验证 | C2 + W5 | 新增 1 个 mock backend + 1 个 verify binary/script |
| P3 | 接口纯化 | W1/W2/W3/W4 | trait 签名调整 + 调用方适配 |
| P4 | 工程化收口 | W6 + N1/N4/N5 | 命名重构、newtype、rebase、推 PR |
| P5 | 长期改进 | N3/N6/N7 | 单独立项（不在本轮 PR） |

## 4. 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| AGENTS.md 禁 `cargo test`，无传统单元测试入口 | 高 | P2 用独立 binary + 实际 dbnum 跑通 + RecordingBackend 断言 |
| `feat/collab-api-consolidation` 工作区有未提交 `model_writer.rs` 改动，rebase 必冲突 | 高 | P4 第一步先与 base 同步，本计划任务全部基于 sync 后的 HEAD |
| DrainOnly 改成走 trait 后，cleanup 必须实现为 NoOp，否则真删现有数据 | 中 | T1.2 强制要求 `DrainOnlyModelWriteBackend::cleanup` 直接返回 Ok 并日志记录 |
| 命名重构 `ModelWriteBackend` → `ModelWriterBackend` 涉及多文件替换 | 低 | P4 用 grep 全局确认；放在最后一步避免污染 P1-P3 diff |
| 接口纯化（移除冗余字段）改动可能让 `run_boolean_bridge` 行为微变 | 中 | P3 每个改动配 1 个最小 dbnum 跑通验证 |

## 5. 详细任务

### Phase 1：闭环抽象

#### Task 1.1 — 拆 `model_writer.rs` 为模块目录

**Files**：

- Delete: `src/fast_model/gen_model/model_writer.rs`
- Create: `src/fast_model/gen_model/model_writer/mod.rs`
- Create: `src/fast_model/gen_model/model_writer/surreal.rs`
- Create: `src/fast_model/gen_model/model_writer/drain_only.rs`
- Modify: `src/fast_model/gen_model/mod.rs`（如需调整模块声明）

**Steps**：

1. 在 `model_writer/mod.rs` 保留 trait 定义、所有 Request/Report 类型、`create_model_writer` 工厂、`pub use` 重新导出。
2. 把 `SurrealModelWriteBackend` + `save_aabb_to_surreal_strict` + `save_pts_to_surreal_strict` 移到 `surreal.rs`，作为 `pub(super)` 或 `pub(crate)` 暴露。
3. 把 `DrainOnlyStats` + `run_drain_only_sink` 移到 `drain_only.rs`。
4. `mod.rs` 通过 `mod surreal;` `mod drain_only;` 引入；导出仅暴露公共 API。

**Expected**：`cargo check --bin plant_db_cli --features review` 通过；外部 import 路径不变（靠 `pub use`）。

**Verify**：

```powershell
cargo check --bin plant_db_cli --features review 2>&1 | Select-String "error\["
```

输出空 = 通过。

---

#### Task 1.2 — 引入 `DrainOnlyModelWriteBackend`

**Files**：

- Modify: `src/fast_model/gen_model/model_writer/drain_only.rs`

**Steps**：

1. 新建 `pub struct DrainOnlyModelWriteBackend { stats: Arc<Mutex<DrainOnlyStats>> }`（或用 atomic 计数器）。
2. 实现 `ModelWriteBackend`：
   - `name()` → `"drain-only"`
   - `init` / `cleanup` / `reconcile_missing_neg` / `run_boolean_bridge` / `finalize` → 仅日志 + Ok（cleanup 必须 NoOp，禁止删数据）
   - `write_base_batch` → 累计 stats，返回空 `SaveInstanceDataReport`
   - `persist_mesh_results` / `write_inst_relate_aabb` → 累计 stats + Ok
3. `finalize` 时打印 stats summary（复用现有 `DrainOnlyStats::print_summary`）。

**Expected**：DrainOnly 完整实现 trait，所有方法日志前缀 `[model-writer:drain-only] stage=...`。

**Verify**：阅读 diff 确认 8 个方法签名匹配 + cleanup 是 NoOp。

---

#### Task 1.3 — `create_model_writer` 不再为 DrainOnly bail

**Files**：

- Modify: `src/fast_model/gen_model/model_writer/mod.rs`

**Steps**：

1. 把 `ModelWriterMode::DrainOnly => bail!(...)` 改为返回 `Arc::new(DrainOnlyModelWriteBackend::default())`。
2. 工厂日志统一为 `"[model-writer] factory selected primary={} mirror=none fail_fast=true"`。

**Expected**：工厂对所有 ModelWriterMode 都返回有效 backend，无 bail 路径。

---

#### Task 1.4 — orchestrator 移除 `Option<Arc<dyn ModelWriteBackend>>`

**Files**：

- Modify: `src/fast_model/gen_model/orchestrator.rs`

**Steps**：

1. `process_index_tree_generation` 入参从 `Option<Arc<dyn ModelWriteBackend>>` 改为 `Arc<dyn ModelWriteBackend>`。
2. `run_base_writer` / `run_inst_aabb_writer` 同改。
3. 删除 `if db_option.model_writer_mode == ModelWriterMode::DrainOnly { None } else { ... }` 二分支，统一调 `create_model_writer + init`。
4. DrainOnly 旧的 `run_drain_only_sink` 路径如仍有依赖，将其挪到 backend 内部或删除（trait 化后 sink 行为内化）。

**Expected**：orchestrator 不再出现 `Option::Some/None` 关于 model_writer 的分支；diff 净减少调用点 if/else。

**Verify**：

```powershell
rg -n "Option<Arc<dyn ModelWriteBackend>" src/
```

仅应在已删除/改名前的注释里出现，运行后无匹配。

---

#### Task 1.5 — cli_modes 移除 `if writes_to_surreal()` 守卫

**Files**：

- Modify: `src/cli_modes.rs`（`run_regen_model` 周边）

**Steps**：

1. 把 `run_regen_model` 中 `if db_option_override.model_writer_mode.writes_to_surreal() { create + cleanup } else { 跳过 }` 直接改为始终 `create + cleanup`，依靠 DrainOnly backend 的 NoOp cleanup。
2. 移除 `"   - drain-only 压测模式：跳过 regen cleanup..."` 日志（改由 backend 内部 cleanup 日志体现）。

**Expected**：调用方无模式判定，靠 backend 多态决定行为。

---

### Phase 2：Mock 与契约验证

> **细颗粒度执行手册**：见同目录 [`p2-implementation-guide.md`](p2-implementation-guide.md)，包含 mock.rs 完整骨架、verify binary 完整代码、PowerShell 脚本、风险表与回退方案。

#### Task 2.1 — 新增 `RecordingBackend`

**Files**：

- Create: `src/fast_model/gen_model/model_writer/mock.rs`

**Steps**：

1. 在 `mock.rs` 内 `pub struct RecordingBackend { calls: Arc<Mutex<Vec<String>>> }`。
2. 实现 `ModelWriteBackend`：每个方法把 `"<method>(<key_args>)"` push 到 `calls`，并返回最小默认 Ok。
3. 提供 `pub fn snapshot(&self) -> Vec<String>` 给验证脚本读取。
4. 用 `#[cfg(any(test, feature = "model-writer-mock"))]` 守卫，避免污染 release 构建。

**Expected**：mock backend 可由测试 fixture / verify binary 注入。

---

#### Task 2.2 — 引入 `verify-trait` binary 跑契约断言

**Files**：

- Create: `src/bin/verify_model_writer_trait.rs`

**Steps**：

1. 新建一个最小 binary：构造 `RecordingBackend`，喂一个手工拼的 `BaseInstanceBatch` / `MeshResultBatch` / `InstRelateAabbBatch`，依次调 trait 8 个方法。
2. 校验 `snapshot()` 返回的调用序列等于预期：

   ```
   ["init", "cleanup", "write_base_batch:batch=1",
    "persist_mesh_results:batch=1", "write_inst_relate_aabb:batch=1",
    "reconcile_missing_neg", "run_boolean_bridge:db_legacy",
    "finalize"]
   ```

3. 不匹配则 `process::exit(1)` + 打印 diff。

**Expected**：`cargo run --bin verify_model_writer_trait --features model-writer-mock` 退出码 0。

**Verify**：

```powershell
cargo run --bin verify_model_writer_trait --features model-writer-mock 2>&1
echo "exit=$LASTEXITCODE"
```

---

#### Task 2.3 — 把验证脚本归档

**Files**：

- Create: `docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1`

**Steps**：

1. 内容：调用 T2.2 binary，断言 exit=0，记录耗时。
2. README 段落：使用方式、预期输出。

**Expected**：脚本可独立运行，输出 `[verify] PASS` / `[verify] FAIL: ...`。

---

### Phase 3：接口纯化

> **细颗粒度执行手册**：见同目录 [`p3-implementation-guide.md`](p3-implementation-guide.md)，包含每个 Task 的现状代码截取、目标骨架、`OnceLock` 缓存 context 的实现细节、与 mock 同步约定。

#### Task 3.1 — 引入精简对外 Report 类型

**Files**：

- Modify: `src/fast_model/gen_model/model_writer/mod.rs`

**Steps**：

1. 定义 `pub struct WriteBaseReport { pub batch_id: u64, pub missing_neg_count: usize }`。
2. `write_base_batch` 返回值由 `SaveInstanceDataReport` 改为 `WriteBaseReport`。
3. `SurrealModelWriteBackend::write_base_batch` 内部调 `pdms_inst::save_instance_data_with_report` 后转换为 `WriteBaseReport`，把 `missing_neg_carriers: Vec<RefnoEnum>` 用单独的 trait 方法 `take_missing_neg_carriers(&self) -> Vec<RefnoEnum>` 暴露（或塞进 `WriteBaseReport`）。
4. 调用方 orchestrator 同步适配。

**Expected**：trait 接口不再暴露 `pdms_inst::SaveInstanceDataReport` 内部细节。

---

#### Task 3.2 — 去除 `BooleanBridgeRequest` 中的冗余字段

**Files**：

- Modify: `src/fast_model/gen_model/model_writer/mod.rs`
- Modify: `src/fast_model/gen_model/model_writer/surreal.rs`
- Modify: `src/fast_model/gen_model/orchestrator.rs`

**Steps**：

1. 从 `BooleanBridgeRequest` 删除 `use_surrealdb` / `defer_db_write`。
2. `SurrealModelWriteBackend` 内部缓存 init 时的 `ModelWriterContext`（改 backend 持有 `OnceCell<ModelWriterContext>` 或 `Arc<RwLock<...>>`），`run_boolean_bridge` 直接读 context。
3. 调用方 orchestrator 移除这两个字段的赋值。

**Expected**：`BooleanBridgeRequest` 仅保留 `mode`、`db_option`、`bool_tasks` 三字段。

---

#### Task 3.3 — `persist_mesh_results` 移除全局 `use_file_mesh_state()` 依赖

**Files**：

- Modify: `src/fast_model/gen_model/model_writer/mod.rs`
- Modify: `src/fast_model/gen_model/model_writer/surreal.rs`
- Modify: `src/fast_model/gen_model/orchestrator.rs`

**Steps**：

1. 在 `MeshResultBatch` 增加 `pub file_mesh_state: bool` 字段。
2. orchestrator 在构造 batch 时填入 `use_file_mesh_state()` 当前值。
3. `SurrealModelWriteBackend::persist_mesh_results` 改读 `batch.file_mesh_state`，不再调全局函数。

**Expected**：trait 行为可由参数完全决定，mock backend 可模拟 file mesh 模式。

---

#### Task 3.4 — `Surreal + use_surrealdb=false` 早期拒绝

**Files**：

- Modify: `src/options.rs`（`validate_model_writer_features`）

**Steps**：

1. 在 `validate_model_writer_features` 内增加：

   ```rust
   if matches!(self.model_writer_mode, ModelWriterMode::Surreal) && !self.use_surrealdb {
       anyhow::bail!("model_writer=surreal 要求 use_surrealdb=true");
   }
   ```

2. `SurrealModelWriteBackend::init` 中保留同名 ensure 作为兜底（Defense in Depth）。

**Expected**：非法组合在 options 解析后立即拒绝，不再走完 perf init / pre_check 才报错。

---

### Phase 4：工程化收口

> **细颗粒度执行手册**：见同目录 [`p4-implementation-guide.md`](p4-implementation-guide.md)，包含 9 个 commit 的切片建议、rebase 三种策略对比（推荐选项 C）、`SurrealRecordKey` newtype 完整代码、PR 描述模板。

#### Task 4.1 — 与 base 分支同步

**Files**：

- Worktree: `.worktrees/model-persistence-trait`

**Steps**：

1. 主仓库 `feat/collab-api-consolidation` 上未提交的 `model_writer.rs` 改动用 `git stash` 暂存或先收口提交。
2. worktree 上 `git fetch origin && git rebase feat/collab-api-consolidation`（处理冲突）。
3. 冲突解决原则：保留 trait 化骨架，base 那边的小改动按语义合入 trait 实现内部。

**Expected**：worktree HEAD 直接基于最新 base，无 stale 风险。

**Verify**：

```powershell
git -C .worktrees/model-persistence-trait log --oneline feat/collab-api-consolidation..HEAD
```

应只列出本计划本身的 commits + 原 `b060860`。

---

#### Task 4.2 — 命名统一

**Files**：

- 全仓库 grep：`ModelWriteBackend`

**Steps**：

1. trait 改名为 `ModelWriterBackend`，与 `ModelWriterMode` / `ModelWriterContext` / `model_writer_mode` 对齐。
2. 用 `rg --files-with-matches "ModelWriteBackend"` 找所有引用，逐一替换。

**Expected**：所有命名均为 `ModelWriter*`。

---

#### Task 4.3 — Surreal Record ID 加 newtype（N5）

**Files**：

- Modify: `src/fast_model/gen_model/model_writer/surreal.rs`

**Steps**：

1. 定义 `struct SurrealRecordKey(String)`，`new` 时校验 `[A-Za-z0-9_:]` 或显式 escape。
2. `save_aabb_to_surreal_strict` / `save_pts_to_surreal_strict` 改用 `SurrealRecordKey` 拼接 SQL，禁止直接 String。

**Expected**：record id 走类型层守卫，未来若 key 来源扩展不会直接变注入点。

---

#### Task 4.4 — 推送 + 开 PR

**Files**：

- Worktree: `.worktrees/model-persistence-trait`

**Steps**：

1. `git push -u origin feat/model-persistence-trait`。
2. 用 `gh pr create` 开 PR，PR body 引用本计划文件 + `findings.md` review 结论 + 各 Phase 实现差异。
3. PR 标题：`feat(model): close ModelWriter trait abstraction (drain-only + mock + interface purification)`。

**Expected**：远端有 PR，URL 写回 `progress.md`。

---

### Phase 5：长期改进（独立 PR，不阻塞本轮）

#### Task 5.1 — 评估 `async fn in trait`（N3）

**Steps**：调研 nightly 支持度，对比 dyn trait + 性能开销，评估是否值得在 hot path 上做。**输出**：评估报告归档到 `docs/plans/2026-05-09-model-write-trait-followup/n3-async-fn-evaluation.md`。

#### Task 5.2 — ~~`name()` 改 `const NAME`（N4）~~ **Cancelled 2026-05-11**

trait 通过 `Arc<dyn ModelWriterBackend>` 走 vtable 调用，关联 const 不进 vtable，无法替代 method。保留 method 形式即可，无需独立 Phase 5 立项。

#### Task 5.3 — 清理 `write_base_batch` 空 mesh_results 包袱（N6）

**Steps**：给 `pdms_inst` 加 `save_instance_data_base_only(...)`，trait 实现内不再造空 HashMap。

---

## 6. 验证策略

按 `AGENTS.md` 不跑 `cargo test`：

| 验证类型 | 手段 | 触发时机 |
|---|---|---|
| 编译 | `cargo check --bin plant_db_cli --features review` | 每个 Task 完成后 |
| Trait 契约 | `cargo run --bin verify_model_writer_trait --features model-writer-mock` | P2 完成后 + 每个改 trait 接口的 Task 后 |
| Surreal 写入 | 选 1 个最小 dbnum 跑 `gen-model` 命令，对比写前/写后 SurrealDB 行数 | P1 + P3 各跑一次 |
| DrainOnly 行为 | 用 `model_writer_mode = "drain-only"` 跑同 dbnum，确认 stats 输出与原 `run_drain_only_sink` 一致 | P1 完成后 |
| 集成 | 启动 `web_server` 走 `/api/health` 与已有 admin smoke | P4 推送前 |

## 7. 错误处理协议（3-Strike）

```
ATTEMPT 1: Diagnose & Fix
  → 读错误堆栈，确认根因；写入 progress.md 错误表

ATTEMPT 2: Alternative Approach
  → 同错复现 → 改用不同实现路径
  → 如 trait 接口冲突，回退到上一个 Task 的检查点

ATTEMPT 3: Broader Rethink
  → 质疑前置假设（如：DrainOnly 真能 NoOp cleanup 吗？）
  → 是否需要把 Task 拆得更细

AFTER 3 FAILURES:
  → 在 progress.md 记录三次尝试详情
  → 调用 best-mcp-sqlite-5 check_messages 升级给用户
```

所有错误一律写入 `progress.md` 的 "Errors Encountered" 表，不做静默吞错。

## 8. 约束与边界

- **不动其他 worktree**：`pe-transform-backends` / `perf-cata` / `perf-mesh` / `perf-scheduler` / `perf-sink` / `room-compute-3x` 一律不动。
- **不跨仓改动**：`plant3d-web` / `rs-core` / `pdms-io-fork` / `pid-parse` 一律不动。
- **不动主分支未提交内容**：`feat/collab-api-consolidation` 工作区现有修改是站点部署任务的产物，与本计划无关。
- **不破坏既有日志契约**：`[batch_perf] batch=...` 等已被运维使用的日志格式保留不动。

## 9. 完成判定

- [ ] P1 所有 Task 完成 → `cargo check` 通过 + DrainOnly 走 trait 跑通
- [ ] P2 所有 Task 完成 → verify binary exit=0
- [ ] P3 所有 Task 完成 → trait 接口纯化 diff 通过 review
- [ ] P4 所有 Task 完成 → PR URL 写入 progress.md
- [ ] 同目录 `progress.md` 每个 Phase 都有完成时间戳
- [ ] 同目录 `findings.md` 增量记录新发现的问题（如有）
