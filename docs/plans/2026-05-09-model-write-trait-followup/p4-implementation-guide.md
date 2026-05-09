# P4 实施手册 — 工程化收口

> **状态**：P1 已完成；P2/P3 实施手册已写。本手册细化 `task_plan.md` §5 Phase 4 的 4 个 Task。
> **核心目标**：把 worktree 改动变成可 review、可合并的 PR，并解决环境/命名/SQL 安全的最后几个 Note。
> **读者**：执行 P4 的 agent / 工程师；假设已读过 task_plan.md / findings.md / p2-implementation-guide.md / p3-implementation-guide.md。

---

## 0. P4 上下文与前置

### 0.1 前置条件（开始 P4 前必须满足）

- [ ] P1 全部 Task 完成（5 个 Task complete）
- [ ] P2 全部 Task 完成（mock + verify binary + ps1）
- [ ] P3 至少完成 T3.1（W1 接口耦合泄漏被修），T3.2/T3.3/T3.4 可在后续 PR 续做
- [ ] 主仓 `feat/collab-api-consolidation` 工作区现有 `model_writer.rs` 未提交改动已收口（提交或 stash 或归档说明）

### 0.2 worktree 当前状态预期

```powershell
git -C .worktrees/model-persistence-trait status --short
# 预期输出：worktree clean（所有改动已 commit）
git -C .worktrees/model-persistence-trait log --oneline feat/collab-api-consolidation..HEAD
# 预期输出：N+1 个 commit（b060860 + N 个本计划新 commit）
```

### 0.3 commit 粒度建议

P1-P3 的代码落地按以下粒度切 commit（推 PR 时用 `git rebase -i` 整理）：

| Commit | 内容 | 对应 Task |
|---|---|---|
| 1 | `feat(model): split model_writer.rs into module dir` | T1.1 |
| 2 | `feat(model): introduce DrainOnlyModelWriteBackend` | T1.2 + T1.3 |
| 3 | `refactor(model): unify trait call path, remove Option<ModelWriteBackend>` | T1.4 + T1.5 |
| 4 | `feat(model-test): RecordingBackend + verify_model_writer_trait binary` | T2.1 + T2.2 + T2.3 |
| 5 | `refactor(model): introduce WriteBaseReport, hide pdms_inst types from trait` | T3.1 |
| 6 | `refactor(model): cache ModelWriterContext in backend, drop redundant request fields` | T3.2 |
| 7 | `refactor(model): pass file_mesh_state via MeshResultBatch instead of global` | T3.3 |
| 8 | `feat(options): early reject Surreal+use_surrealdb=false combination` | T3.4 |
| 9 | `chore(model): rename trait to ModelWriterBackend; SurrealRecordKey newtype` | T4.2 + T4.3 |

总计 9 个 commit，覆盖 P1-P3 + T4.2 + T4.3。

---

## 1. T4.1 — 与 base 分支同步（rebase）

### 1.1 现状预判

主仓 `feat/collab-api-consolidation` 工作区当前有：

```
modified:   src/fast_model/gen_model/db_model.rs
modified:   src/fast_model/gen_model/model_writer.rs   ← 与本计划同改文件
modified:   src/fast_model/gen_model/orchestrator.rs   ← 与本计划同改文件
... 其他 ~15 个文件
```

`feat/collab-api-consolidation` 已比 `origin/feat/collab-api-consolidation` 领先 5 个本地 commits。

### 1.2 rebase 策略选择

#### 1.2.1 选项 A：主仓先收口提交 → worktree rebase 主仓

1. 主仓先把 `model_writer.rs` 与 `orchestrator.rs` 的未提交改动单独提交（commit message 标"non-trait pre-rebase fixup"）。
2. worktree 上：

   ```powershell
   git -C .worktrees/model-persistence-trait fetch origin
   git -C .worktrees/model-persistence-trait rebase feat/collab-api-consolidation
   ```

3. 冲突处理：以 worktree 上的 trait 化版本为主，把主仓上的小改动按语义合入对应 trait 方法内部（surreal.rs / orchestrator.rs）。

**优点**：主仓改动有 commit 历史，conflict 可视化好。
**缺点**：主仓那条 commit 不属于本计划任务，commit 归属易混。

#### 1.2.2 选项 B：主仓 stash → worktree 推 PR → 主仓恢复 stash

1. 主仓 `git stash push -- src/fast_model/gen_model/model_writer.rs src/fast_model/gen_model/orchestrator.rs`
2. worktree rebase + push + 开 PR
3. PR 合并后主仓 `git stash pop` 重新应用

**优点**：清晰隔离两个任务的改动。
**缺点**：stash 含 model_writer 的旧改动，pop 时必冲突，需要再次手工解。

#### 1.2.3 选项 C（推荐）：主仓 stash 全部 → 单独评估

1. 主仓 `git stash push -m "site-deployment uncommitted, pre-trait-rebase"`
2. worktree rebase 主仓现在的 HEAD（已 stash，干净）
3. worktree push + 开 PR
4. PR 合并后回主仓 `git stash pop`，逐文件评估：站点部署任务是否需要继续；如继续，则在另一个 PR 中提交

**优点**：本 PR 边界清晰，主仓另一个任务独立处理。
**缺点**：主仓的 hooks / progress 文件等也会被 stash，pop 时要审视。

### 1.3 推荐

按 §1.2.3（**选项 C**）。要点：

- 主仓 stash 时 message 要明确："site-deployment uncommitted, pre-trait-rebase, recover later"
- worktree rebase 之前先 backup：`git tag worktree-pre-rebase-2026-05-09`
- rebase 冲突解决后跑 `verify-mock.ps1` 与 `cargo check --lib`

### 1.4 verify 命令

```powershell
# rebase 前
git -C .worktrees/model-persistence-trait log --oneline feat/collab-api-consolidation..HEAD
# 期望：列出 P1-P3 的 N 个 commit

# rebase 后
git -C .worktrees/model-persistence-trait status
# 期望：clean
git -C .worktrees/model-persistence-trait log --oneline feat/collab-api-consolidation..HEAD
# 期望：commit 数与 rebase 前相同（除非主动 squash）

# 完整 build（NASM + git-fetch-with-cli 都需就绪）
$env:PATH = "C:\Program Files\NASM;" + $env:PATH
$env:CARGO_NET_GIT_FETCH_WITH_CLI = "true"
cd .worktrees/model-persistence-trait
cargo check --bin web_server --features web_server 2>&1 | Select-Object -Last 30
# 期望：finished `dev` profile
```

### 1.5 风险与回退

| 风险 | 处置 |
|---|---|
| rebase 大量冲突 | 用 `worktree-pre-rebase-2026-05-09` tag 回退；重新评估是否要做"主仓先合 PR"再 rebase |
| 主仓 stash 含敏感文件（如 `db_options/*.toml`） | stash 之前先 `git status` 审视，敏感文件单独 commit 或 .gitignore |
| `cargo check` 在 worktree 仍因 NASM/PATH 失败 | 在临时 PowerShell 子进程显式设置 PATH：`pwsh -NoProfile -Command "$env:PATH='C:\Program Files\NASM;'+$env:PATH; cargo check --lib"` |
| 主仓 stash pop 冲突无法解决 | 暂存 stash，用户决策；本计划范围内不再继续主仓恢复 |

---

## 2. T4.2 — 命名统一

### 2.1 现状

```
trait ModelWriteBackend  ← 没有 r
struct SurrealModelWriteBackend
struct DrainOnlyModelWriteBackend
mod   model_writer
enum  ModelWriterMode
struct ModelWriterContext
field model_writer_mode
fn    create_model_writer
```

trait 名是 `ModelWrite**Backend**`（动词 + Backend），与其他名字不一致。

### 2.2 目标

全部统一为 `ModelWriter*`：

```
trait ModelWriterBackend   ← + r
struct SurrealModelWriterBackend
struct DrainOnlyModelWriterBackend
struct RecordingBackend → 不需要改（特殊用途，命名正交）
```

### 2.3 实现差量

#### 2.3.1 grep 全部引用

```powershell
rg -n "ModelWriteBackend" .worktrees/model-persistence-trait/src/ docs/
```

预期出现位置：

- `model_writer/mod.rs`：trait 定义 + factory 返回类型 + use 导出
- `model_writer/surreal.rs`：impl 头 + struct 名
- `model_writer/drain_only.rs`：impl 头 + struct 名
- `model_writer/mock.rs`：impl 头
- `orchestrator.rs`：use 段、函数签名 `model_writer: Arc<dyn ModelWriteBackend>`
- `cli_modes.rs`：（可能没有，因为只用 create_model_writer）
- `verify_model_writer_trait.rs`：use 段
- `findings.md` / `task_plan.md` / `progress.md` / `p2-implementation-guide.md` / `p3-implementation-guide.md`：文档引用

#### 2.3.2 批量替换

代码层（保留 `Backend` 后缀）：

```
ModelWriteBackend           → ModelWriterBackend
SurrealModelWriteBackend    → SurrealModelWriterBackend
DrainOnlyModelWriteBackend  → DrainOnlyModelWriterBackend
```

文档层（findings/task_plan/progress 等同步改）。

**注意**：命名重构涉及多文件全局替换，**放在 P4 倒数第二步**（T4.3 之后、T4.4 之前），避免污染 P1/P2/P3 的 commit diff。

#### 2.3.3 验证

```powershell
rg -n "ModelWriteBackend|SurrealModelWriteBackend|DrainOnlyModelWriteBackend" .worktrees/model-persistence-trait/src/
# 期望：0 行匹配（替换完毕）

rg -n "ModelWriterBackend|SurrealModelWriterBackend|DrainOnlyModelWriterBackend" .worktrees/model-persistence-trait/src/
# 期望：与替换前的旧名 grep 数一致
```

### 2.4 风险

| 风险 | 处置 |
|---|---|
| 改名时漏掉文档/注释里的旧名 | grep 不限 src/，包括 docs/ 和 *.md |
| 主仓有依赖 `aios_database::fast_model::gen_model::model_writer::ModelWriteBackend` 的代码（如 web_api） | 主仓在 stash 状态下没 import；rebase 后主仓代码不会再引用本 trait（只用 factory） |

---

## 3. T4.3 — `SurrealRecordKey` newtype

### 3.1 现状

```rust
// surreal.rs save_aabb_to_surreal_strict
let id_key = if k.starts_with("aabb:") {
    k.to_string()
} else {
    format!("aabb:⟨{}⟩", k)
};
rows.push(format!("{{'id':{id_key}, 'd':{d}}}"));

// save_pts_to_surreal_strict
rows.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", k, v.value()));
```

key 来源是内部 hash，**当前安全**；但代码无类型层守卫。

### 3.2 目标

引入 newtype：

```rust
/// SurrealDB record id 的安全包装。构造时强制 ASCII alphanum + ':' + '_'，禁止任意 String。
pub(super) struct SurrealRecordKey(String);

impl SurrealRecordKey {
    pub fn new(table: &'static str, raw_key: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            raw_key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-')),
            "SurrealRecordKey rejects non-ASCII raw_key: {:?}",
            raw_key
        );
        let id = if raw_key.starts_with(&format!("{}:", table)) {
            raw_key.to_string()
        } else {
            format!("{}:⟨{}⟩", table, raw_key)
        };
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

`save_aabb_to_surreal_strict` 改用：

```rust
let id_key = SurrealRecordKey::new("aabb", k)?;
rows.push(format!("{{'id':{}, 'd':{d}}}", id_key.as_str()));
```

`save_pts_to_surreal_strict` 改 `SurrealRecordKey::new("vec3", &k.to_string())?`。

### 3.3 字符集决策

当前 helper 接受的 key：

- `aabb` 表：`String`（来自 mesh hash，pdms_io 输出，**实际是十六进制 hash**）
- `vec3` 表：`u64`（mesh pts hash，二进制转字符串后是数字）

ASCII alphanum + `:` + `_` + `-` 足够覆盖。如果未来引入"XML escape" 或 "UTF-8 hash" 类的 key，需要扩展字符集策略。

### 3.4 风险

| 风险 | 处置 |
|---|---|
| 现有 key 中含字符不在白名单 | 在引入 newtype 时立即跑一次小 dbnum，捕获 ensure! 失败的 key；如有，扩展白名单或改 escape |
| `SurrealRecordKey::new` 失败传播 anyhow::Error 到 helper 调用链 | helper 已用 `anyhow::Result`，传播路径不变 |

### 3.5 验证

```powershell
rg -n "format!\(\"aabb:|format!\(\"vec3:" .worktrees/model-persistence-trait/src/fast_model/gen_model/model_writer/surreal.rs
# 期望：0 行匹配（已被 SurrealRecordKey 替代）

rg -n "SurrealRecordKey" .worktrees/model-persistence-trait/src/fast_model/gen_model/model_writer/surreal.rs
# 期望：1 个 newtype 定义 + 至少 2 个 ::new 调用
```

---

## 4. T4.4 — 推送 + 开 PR

### 4.1 前置 checklist

- [ ] T4.1 rebase 完成，worktree HEAD 基于最新 base
- [ ] T4.2 命名替换完成，grep 验证无残留
- [ ] T4.3 SurrealRecordKey 落地，grep 验证无遗漏
- [ ] `verify-mock.ps1` 跑通（exit=0）
- [ ] `cargo check --bin web_server --features web_server` 跑通
- [ ] `cargo check --bin aios-database --features review` 跑通（如时间允许）
- [ ] commit 历史按 §0.3 整理，每个 commit 独立有意义
- [ ] PR 描述模板（§4.3）填充完毕

### 4.2 push 命令

```powershell
git -C .worktrees/model-persistence-trait push -u origin feat/model-persistence-trait
```

### 4.3 PR 描述模板

```markdown
## Summary

把 `feat/model-persistence-trait` 的 trait 抽象闭环：

- **P1 闭环抽象**：`ModelWriterBackend` trait 现有 3 个实现（Surreal / DrainOnly / mock），调用方不再有 `Option<Arc<dyn _>>` 二分支；DrainOnly 的 cleanup/finalize 也走 trait
- **P2 Mock 与契约验证**：新增 `RecordingBackend`（feature `model-writer-mock`）+ `verify_model_writer_trait` binary，断言 8 个 trait 方法的调用顺序
- **P3 接口纯化**：trait 不再返回 `pdms_inst::SaveInstanceDataReport`；`BooleanBridgeRequest` 删去冗余 `use_surrealdb` / `defer_db_write` 字段（backend 内部缓存 init context）；`MeshResultBatch` 显式传 `file_mesh_state`，trait 行为不再依赖全局；`options::validate_model_writer_features` 早期拒绝 Surreal+use_surrealdb=false
- **P4 工程化收口**：trait 命名统一为 `ModelWriter*`；Surreal record id 走 `SurrealRecordKey` newtype 守卫

## Review 起点

完整 review 结论与设计推演见：

- `docs/plans/2026-05-09-model-write-trait-followup/findings.md` — 2 项 Critical / 6 项 Warning / 7 项 Note
- `docs/plans/2026-05-09-model-write-trait-followup/task_plan.md` — 阶段总规划
- `docs/plans/2026-05-09-model-write-trait-followup/p2/p3/p4-implementation-guide.md` — 各 Phase 细颗粒度手册
- `docs/plans/2026-05-09-model-write-trait-followup/progress.md` — 实施日志

## 测试

按 `AGENTS.md` 不跑 `cargo test`：

- `cargo check --bin web_server --features web_server` 通过（耗时 X 分钟）
- `cargo check --bin aios-database --features review` 通过（耗时 X 分钟）
- `pwsh docs/plans/2026-05-09-model-write-trait-followup/verify-mock.ps1` 通过（耗时 X 秒）
- 用最小 dbnum 跑 `gen-model` 完成；`Surreal` 模式 / `DrainOnly` 模式各 1 次（如时间允许，结果记入 progress.md）

## 不在本 PR 范围

P5 长期改进（async fn in trait / `const NAME` / 清理空 mesh_results 包袱）独立后续 PR 处理。

## Breaking Changes

- 现存配置 `model_writer = "surreal"` + `use_surrealdb = false` 的组合现会在 options 解析阶段就被拒绝（之前会等到 init 才报错）；该配置组合本身从未真正工作过，理论上无生产影响。
```

### 4.4 开 PR 命令

```powershell
gh pr create --base feat/collab-api-consolidation `
  --title "refactor(model): close ModelWriterBackend trait abstraction (P1-P4)" `
  --body-file docs/plans/2026-05-09-model-write-trait-followup/pr-body.md
```

（`pr-body.md` 由 §4.3 模板生成，提交前填充耗时占位符。）

### 4.5 PR 合并后

把 PR URL 写入 `progress.md`：

```markdown
| PR URL | https://github.com/... | 2026-05-XX |
```

并在 `progress.md` 末尾追加：

```markdown
## P4 完成

- 2026-05-XX rebase 主仓最新 commit X 个 → 冲突 Y 处已解决
- 2026-05-XX 命名统一替换完成（替换数：N）
- 2026-05-XX SurrealRecordKey newtype 落地
- 2026-05-XX 推送 origin/feat/model-persistence-trait
- 2026-05-XX PR 开启：[URL]
- 2026-05-XX PR review pass
- 2026-05-XX PR merged
```

---

## 5. P5 启动条件（仅记录，不在 P4 内执行）

P4 PR merge 之后，P5 三个 Task 各自独立立项 PR：

### T5.1 — 评估 `async fn in trait`

输出文档：`docs/plans/2026-05-09-model-write-trait-followup/n3-async-fn-evaluation.md`，包含：

- nightly toolchain 当前对 `async fn in trait` + `dyn` 的支持矩阵
- hot path 上 `Pin<Box<dyn Future>>` boxing 的实测开销（用 `cargo bench` 或 `profile_*.json`）
- 切换到 `impl Future` 返回类型对调用方的影响（dyn trait object 是否还可用）

### T5.2 — `name()` → `const NAME`

简单，独立 PR：

```rust
pub trait ModelWriterBackend: Send + Sync {
    const NAME: &'static str;
    // ... 8 个方法，去掉 fn name()
}
```

`finalize` 内 `self.name()` 改 `Self::NAME`。

### T5.3 — 清理 `write_base_batch` 内空 HashMap 包袱

需要改 `pdms_inst.rs`，给 `save_instance_data_with_report` 增加无 mesh 重载或拆分函数。属于跨模块改动，独立评估。

---

## 6. P4 完成判定

- [ ] T4.1 rebase 完成，worktree clean
- [ ] T4.2 命名替换完成，grep 0 残留
- [ ] T4.3 `SurrealRecordKey` newtype 落地
- [ ] T4.4 PR 开启，URL 写入 `progress.md`
- [ ] PR 描述按 §4.3 模板，所有时间/耗时占位符已填
- [ ] `verify-mock.ps1` 在最终 commit 上跑通
- [ ] `cargo check --bin web_server --features web_server` 在最终 commit 上跑通
- [ ] CHANGELOG.md 增加一行（如项目用 CHANGELOG）

---

## 7. P4 风险全景表

| 风险 | 等级 | 缓解 | 触发后回退 |
|---|---|---|---|
| 主仓未提交改动 stash 后冲突 | 中 | 选项 C 隔离边界 | tag + 用户决策 |
| rebase 冲突过大 | 高 | 单 commit 切片 + 逐个 cherry-pick | 回退 tag，重新走"主仓先 PR"路径 |
| 命名重构漏改 | 低 | grep 双向验证（旧名 0 + 新名 N） | 单独 follow-up commit |
| `SurrealRecordKey::new` 拒绝既有合法 key | 中 | T4.3 落地后立即跑小 dbnum 验收 | 扩展白名单或改 escape |
| `cargo check` 仍因环境失败 | 中 | 准备干净 PowerShell 子进程命令模板（§1.4） | 在 PR 描述中标"待 reviewer 复跑 build" |
| PR review 被打回需重做接口纯化 | 低 | P3 已闭环，单独问题点改动量小 | follow-up commit |

---

## 8. 错误升级协议

按 `task_plan.md §7` 3-strike：

- 第 1 次失败：诊断 + 修；写入 `progress.md` 错误表
- 第 2 次失败：换路径（如选项 C → 选项 A）
- 第 3 次失败：升级用户；可能需要分两个 PR（先合 P1+P2，后续 P3+P4）

---

## 9. 与 P1/P2/P3 手册的接口

P4 是收尾阶段，不再改 trait 接口语义；仅做：

- 命名（T4.2）— 全文件全局替换
- 安全（T4.3）— surreal.rs 内部加 newtype
- 工程（T4.1 / T4.4）— git 操作

P2 / P3 手册中所有"mock / verify binary 同步改"的约定在 P4 已生效完毕；P4 不再单独要求 mock 同步（除非 T4.2 命名重构动了 mock 文件——确实动，但只是 trait 名替换）。

P5 三个 Task 不在本 PR；如 reviewer 要求 P5 也合并，按 task_plan §5 评估是否重新拆 PR。
