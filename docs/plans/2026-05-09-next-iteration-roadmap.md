# plant-model-gen 下一步开发方案

> 日期：2026-05-09
> 当前分支：`feat/collab-api-consolidation`
> 项目路径：`D:/work/plant-code/plant-model-gen`

---

## 0. 当前状态总览

### 已完成的里程碑

| 里程碑 | 状态 | 说明 |
|---|---|---|
| 异地协同 Sprint B/C/D | ✅ 已完成 | MQTT 后端补齐、站点管理体验优化、SSE 实时化 |
| APS 站点部署验证 | ✅ 已完成 | 安全脱敏、公开列表修复、ASCII task id |
| SurrealDB 依赖统一 | ✅ 已完成 | 三仓 git 源对齐到 github.com/happyrust |
| Model Write Trait 一期 | ✅ 已完成 | P1-P4 worktree 侧闭环（DrainOnly trait 化、Mock 验证、接口纯化、命名统一） |
| PE Transform 多后端 | ⚠️ 部分完成 | SurrealDB/Parquet/Rkyv 后端 + CLI 参数，冲刺 A-E 待执行 |
| Features 重组 | ✅ 已完成 | 按业务能力切片，review/web_server/parquet-export 分离 |
| form_id 同 task 去重 | ✅ 已完成 | 最新 commit `ce914c86` |

### 未提交改动（feat/collab-api-consolidation 工作树）

| 文件 | 改动范围 |
|---|---|
| `model_writer.rs` | ModelWriter trait + SurrealModelWriter + DrainOnlyWriter（+224 行） |
| `orchestrator.rs` | base_writer / mesh_stage 重构为 worker pool（+452 行） |
| `transform_cache.rs` | prime_global_transform_cache_from_pe_entries（+54 行） |
| `pe_transform_refresh.rs` | compat 函数签名调整 |
| `db_model.rs` / `review_annotation_state.rs` | 数据模型微调 |
| `e3d_tree_api.rs` / `mbd_pipe_api.rs` / `stream_generate.rs` | web API 微调 |

### 活跃 Worktree 分支

| 分支 | 用途 | 状态 |
|---|---|---|
| `feat/model-persistence-trait` | Model Write Trait 二期 | P1-P4 code done，待 rebase + push |
| `feat/pe-transform-backends` | PE Transform 多后端 | 已提交，冲刺 A-E 待执行 |
| `perf/cata-worker-tuning` | cata worker 性能调优 | worktree 保留 |
| `perf/mesh-parallel-runtime` | 并行 mesh 运行时 | worktree 保留 |
| `perf/scheduler-pipeline` | 调度器管线优化 | worktree 保留 |
| `perf/sink-db-io-observability` | DB IO 可观测性 | worktree 保留 |

---

## 1. 冲刺 S1 — 主分支收口与合并（优先级：P0，~1 天）

### 目标
将 `feat/collab-api-consolidation` 工作树上的 20 个未提交文件整理提交，然后合入 `main`。

### 任务

| # | 任务 | 详情 | 验收 |
|---|---|---|---|
| S1.1 | 分类未提交改动 | 将改动按来源分组：(a) model_writer trait 相关 (b) review/annotation 修复 (c) web API 微调 (d) 规则文件更新 | 改动分类文档 |
| S1.2 | 分批提交 | 按 S1.1 分组结果逐批 `git add -p` + commit，commit message 遵循项目 conventional commits 风格 | 每个 commit 单一职责 |
| S1.3 | 编译验证 | `cargo check --bin web_server --features web_server` 通过 | 0 new errors |
| S1.4 | 运行验证 | 启动 web_server 并 HTTP smoke：`/api/health`、`/api/version`、`/api/deployment-sites` | 3 个端点 200 |
| S1.5 | 合入 main | `git checkout main && git merge feat/collab-api-consolidation`，推送远端 | `origin/main` 包含全部改动 |

### 风险
- model_writer.rs 的改动与 worktree `feat/model-persistence-trait` 有重叠，需在 S1.2 时明确哪些改动属于主分支哪些留给 worktree
- 工作树有 `.factory/` 截图删除，确认这些不再需要

---

## 2. 冲刺 S2 — Model Write Trait 二期合入（优先级：P0，~0.5 天）

### 前置：S1 完成

### 目标
将 `feat/model-persistence-trait` worktree 的 P1-P4 成果 rebase 到最新 main 并推送 PR。

### 任务

| # | 任务 | 详情 |
|---|---|---|
| S2.1 | Rebase | `cd .worktrees/model-persistence-trait && git rebase main`，解决 transform_cache.rs / pe_transform_refresh.rs / main.rs 冲突 |
| S2.2 | 编译验证 | worktree 内 `cargo check --lib` 通过 |
| S2.3 | 契约验证 | 运行 `verify-mock.ps1`，8 个 trait 方法调用顺序断言全过 |
| S2.4 | 推送 + 开 PR | `git push -u origin feat/model-persistence-trait` → `gh pr create` |

### 验收
- PR 可 review，CI 编译通过
- RecordingBackend 断言 exit code 0

---

## 3. 冲刺 S3 — PE Transform Parquet 运行验证（优先级：P1，~0.5 天）

### 前置：S1 完成

### 目标
在 `feat/pe-transform-backends` worktree 上用 dual 模式实际写出 Parquet 文件并验证读回一致性。

### 任务

| # | 任务 | 详情 |
|---|---|---|
| S3.1 | 重编译 | `cargo build --features "review,transform-store-parquet"` |
| S3.2 | dual 模式刷新 | `--refresh-transform 1112 --transform-write-backend dual --transform-parquet-dir output/pe_transform` |
| S3.3 | 验证 Parquet 文件 | 检查行数 ≥ 40 万、字段完整性 |
| S3.4 | compare 对比 | `--transform-compare-backends parquet` → max_delta 应为 0 |
| S3.5 | 纯 parquet 读取 | `--transform-read-backend parquet --refresh-transform 1112`，验证与 surreal 一致 |

### 验收
- Parquet 行数 ≥ 40 万
- compare max_delta = 0，missing = 0
- 无 panic / 数据丢失

---

## 4. 冲刺 S4 — 两大 Feature 分支合并（优先级：P1，~0.5 天）

### 前置：S2 + S3 完成

### 目标
将 `feat/pe-transform-backends` 和 `feat/model-persistence-trait` 的改动统一合入 `main`。

### 冲突预案

| 文件 | 冲突风险 | 策略 |
|---|---|---|
| `transform_cache.rs` | 高 | 两分支都新增 prime 函数，取 pe-transform 版本 + ModelWriter Copy derive |
| `pe_transform_refresh.rs` | 中 | pe-transform 改 flush_entries，model-trait 改 compat 签名 |
| `main.rs` | 中 | pe-transform 加 7 个 CLI 参数，model-trait 改 model-writer 逻辑 |
| `model_writer/` | 低 | 仅 model-trait 改动 |

### 任务

| # | 任务 |
|---|---|
| S4.1 | 合并 `feat/model-persistence-trait` PR 到 main |
| S4.2 | Rebase `feat/pe-transform-backends` 到 main |
| S4.3 | 解决冲突并验证 `cargo check --features review` |
| S4.4 | 合并 pe-transform PR 到 main |

---

## 5. 冲刺 S5 — ModelWriter + Transform 端到端集成（优先级：P1，~1 天）

### 前置：S4 完成

### 目标
让模型生成管线同时使用 ModelWriter trait 和 pe_transform 多后端，验证完整流水线。

### 任务

| # | 任务 | 详情 |
|---|---|---|
| S5.1 | orchestrator 集成 | `gen_all_geos_data` 根据 `model_writer_mode` 创建 ModelWriter，根据 `transform_write_backend` 控制刷新路径 |
| S5.2 | 端到端验证 | 完整 gen_model + Parquet 双写，确认几何输出不变 |
| S5.3 | drain-only + parquet | 验证 `--model-writer drain-only --transform-write-backend parquet` 压测路径 |
| S5.4 | 性能基线 | surreal / parquet / rkyv 各后端 load 时间测量 |

### 验收
- 完整 dbnum=1112 gen_model 运行成功
- 几何输出（GLB/XKT 文件）与基线一致
- 性能数据记录到 docs/plans/

---

## 6. 冲刺 S6 — 异地协同 Sprint E/F（优先级：P2，~2 天）

### 目标
完成站点管理剩余两个 Sprint 的收口工作。

### Sprint E 任务

| # | 任务 | 详情 |
|---|---|---|
| S6.1 | E1 站点健康看板 | `/api/admin/sites/health-summary` 聚合端点，返回全局健康统计 |
| S6.2 | E2 站点 SSE 事件扩展 | 站点状态变更推送到前端实时刷新 |
| S6.3 | E3 日志轮转 | 后端日志文件按大小/日期轮转，避免无限增长 |

### Sprint F 任务

| # | 任务 | 详情 |
|---|---|---|
| S6.4 | F1 站点模板 | 预设配置模板一键创建站点 |
| S6.5 | F2 多项目支持 | 单站点绑定多个 AVEVA 项目路径 |

---

## 7. 冲刺 S7 — 性能分支清理与合入（优先级：P2，~1.5 天）

### 目标
评估并收口 4 个 `perf/*` worktree 分支。

### 任务

| # | 任务 | 详情 |
|---|---|---|
| S7.1 | 评估 cata-worker-tuning | 对比 baseline 性能指标，决定合入/弃用 |
| S7.2 | 评估 mesh-parallel-runtime | 并行 mesh 生成的稳定性和提升幅度 |
| S7.3 | 评估 scheduler-pipeline | 调度器管线优化的实际收益 |
| S7.4 | 评估 sink-db-io-observability | DB IO 可观测性指标的实用性 |
| S7.5 | 清理或合入 | 有价值的合入 main，已过时的删除 worktree |

---

## 8. 冲刺 S8 — 长期改进与技术债（优先级：P3，持续）

### Model Write Trait P5（独立立项）

| # | 任务 | 详情 |
|---|---|---|
| S8.1 | async fn in trait | 评估移除 `async_trait` 宏，使用 Rust 原生异步 trait |
| S8.2 | name() → const NAME | trait 方法改为关联常量 |
| S8.3 | write_base_batch 空 HashMap | 清理遗留的空 HashMap 参数包袱 |

### DuckLake 后端（可选）

| # | 任务 | 详情 |
|---|---|---|
| S8.4 | register_ducklake 实现 | 让 Parquet 文件通过 DuckLake 注册到数据湖 |
| S8.5 | load_entries_from_ducklake | DuckDB 查询 DuckLake 注册的 Parquet |

### rs-core SurrealDB 弹性

| # | 任务 | 详情 |
|---|---|---|
| S8.6 | idle 断连恢复 | 按 `2026-05-07-rs-core-sul-db-idle-resilience-plan.md` 实施 |
| S8.7 | rollout 计划 | 按 `2026-05-07-rs-core-sul-db-rollout-plan.md` 分阶段上线 |

---

## 9. 推荐执行时间线

```
周一  5/12: S1 主分支收口（1天）
周二  5/13: S2 Model Trait PR（上午）+ S3 Parquet 验证（下午）
周三  5/14: S4 分支合并（上午）+ S5 端到端集成（下午起）
周四  5/15: S5 端到端集成（完成）+ 性能基线记录
周五  5/16: S6 Sprint E 启动
下周  5/19+: S6 Sprint E/F 收尾 + S7 perf 分支评估
持续: S8 长期改进按需推进
```

---

## 10. 风险登记

| 风险 | 等级 | 缓解 |
|---|---|---|
| S1 提交时 model_writer.rs 与 worktree 冲突 | P1 | 先明确边界：主分支只提交 trait 定义骨架，具体实现留给 worktree |
| S4 三分支合并产生逻辑回退 | P1 | 每步合并后立即 cargo check + HTTP smoke |
| Parquet 序列化精度丢失 | P2 | compare 工具已覆盖，max_delta > 0 即告警 |
| NASM 环境不稳定影响编译 | P2 | verify-mock.ps1 已自动处理 NASM PATH |
| 4 个 perf worktree 可能已过时 | P2 | S7 先评估再决策，不盲目合入 |
| rs-core 跨仓改动（idle 弹性）引入新风险 | P3 | 独立分支 + 灰度上线 |

---

## 11. 输出产物

本计划审批通过后，将在项目中创建以下文件：

1. `docs/plans/2026-05-09-next-iteration-roadmap.md` — 本计划正式版
2. 每个冲刺完成后更新 `CHANGELOG.md`
3. 每个冲刺的验证报告归档到 `docs/plans/` 对应日期目录
