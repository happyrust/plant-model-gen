# Implementation Plan

## Architecture

1. `model_gen_debt` 表与写入：persist 流程尾部（数据锚点之后）幂等 UPSERT，key `[dbnum, to_sesno]`，字段含 `from_sesno`、五桶 refno、`created_at`、`consumed_at`（空 = 存活欠账）。
2. 模型生成水位与欠账读取 helper：max `model_gen` 锚点 sesno、存活欠账行、区间链 `(from, to]` 覆盖性检查（新模块 `versioned_db/model_gen_debt.rs`，与 `version_commit.rs` 同层）。
3. 从 `run_increment` 提炼可复用的追赶核心：合并欠账五桶 → pe_owner 证据检查 → Incremental scope 生成 → 后处理 → 发锚点 → 标记消费；本轮增量路径与 watch 追赶路径共用同一函数。
4. watch 循环接入：数据步之后对每个候选 dbnum 执行水位比对与追赶；失败隔离沿用加固计划 T2 语义（per-dbnum continue，不退进程）。
5. `generate_model` 默认翻转 + `--no-generate-model` 逃生口；`incremental-sesno` 与 web 增量入口对齐同一默认。
6. `model-version catch-up` 子命令：`--dbnum`（可选，缺省全部）、`--dry-run`、`--json`、`--allow-full-regen`（洞兜底的唯一入口）。
7. 属性级过滤：采集分类点接 `model_impact::attribute_affects_model` 的生成链路包装（未知属性 → 触发），`model_neutral_changes` 观测字段 + `--no-model-impact-filter` 逃生口。
8. smoke 脚本与文档对齐（CHANGELOG / ops-notes / AGENTS.md）。

## Rollout

- 行为变化集中在 `generate_model` 默认翻转：CHANGELOG 与 ops-notes 标注；纯数据同步站点升级前改用 `--no-generate-model`。
- 升级后第一轮 watch 对存量断更站点只告警（洞语义），不自动重建；运维按告警清单分批执行 `catch-up --allow-full-regen`。
- 验证遵循 AGENTS.md 约束：CLI `--json` 断言 + `db-data/*.surql` + `scripts/smoke/*.ps1`，不使用 `cargo test`。

## Dependencies

- 建议在增量加固计划（2026-07-20）M1/M2（watch 起点 = watermark、失败隔离、连续性门禁）合入后实施，追赶闭环直接建立在其语义之上。
- 与 `sesno_increment.rs` 相关改动（加固计划 M3/M4 同文件）串行合入，避免冲突。
