# Feature Specification: 7997 release 模型生成写入流水线提速（spec 004）

## User Need

`ams7997_0001` 的 release 模型生成需要稳定完成，并把总耗时降低至少 30%。当前生成过程不是系统资源耗尽，而是 SurrealDB 写入与 `inst_relate_aabb` 持久化拖慢下游，导致 `pending_mesh_outputs` 长期贴近队列上限，最终容易出现写入失败、任务中断或 Parquet 产物缺失。

## Evidence

- 当前运行配置：
  - `index_tree_max_concurrent_targets = 6`
  - `index_tree_batch_size = 200`
  - `gen_mesh = true`
  - `apply_boolean_operation = true`
  - 实际 worker：base 写库 8、mesh 4、`inst_relate_aabb` 2。
- 资源侧并非瓶颈：
  - D 盘剩余约 163GB。
  - 系统可用内存约 21GB。
  - CPU 总体约 30% 到 47%。
- 流水线侧瓶颈：
  - `pending_mesh_outputs` 多次堆到 108，说明 mesh 输出在等待 base 写库结果 join。
  - `base_write_ms` 曾达到 33s 级。
  - `inst_aabb_ms` 曾达到 35s 级。
  - `producer send_wait_ms` 已出现秒级等待，说明背压已传导到生产端。
- 历史失败：
  - 旧失败包含 `inst_relate_aabb:24381_146695 already exists` / `Cannot COMMIT`。
  - 后续失败包含 `写入失败超出重试限制` 与 `job_failed exit_code=-1`。
  - Parquet 验收仍缺 `manifest_7997.json`、`instances.parquet`、`geo_instances.parquet`、`transforms.parquet`、`aabb.parquet`。

## Scope

- 重构 release 模型生成写入流水线中 SurrealDB base batch 与 `inst_relate_aabb` 的持久化策略。
- 将高频逐批 `DELETE + INSERT` 改为两阶段批量写入：生成阶段收集 touched rows，收尾阶段按 dbnum / touched refnos 统一删除并 bulk insert。
- 给写入相关批大小、事务块大小和并发度提供配置化开关，便于 release benchmark。
- 建立 7997 release 性能基线与验收脚本，确认生成成功、Parquet 完整、耗时降低。

## Non-Goals

- 不改变 CATA 按需解析闭包语义。
- 不引入 DuckLake/文件后端作为本期默认写入后端。
- 不以跳过 `inst_relate_aabb` 作为最终生产方案。
- 不用牺牲生成正确性换速度；调参必须以 release 完整成功为前提。

## Decisions（grill-me）

| # | 决策 | 结论 |
|---|---|---|
| Q1 | 第一优先级 | 先保证 release 生成稳定完成，再提速 |
| Q2 | 改动边界 | 允许重构写库 / AABB 持久化路径 |
| Q3 | 成功标准 | 7997 release 完整成功 + 生成耗时降低 30% |
| Q4 | 写入策略 | ~~两阶段批量写入：生成阶段收集，结束后统一删除 + 批量插入~~ **已被 spec 005 取代**（2026-06-12）：入口 `pre_cleanup_for_regen` 整体清理 + 全程 `INSERT IGNORE` 幂等写入，写入层零 DELETE 零预查，见 `specs/005-idempotent-write-pipeline/` |
| Q5 | 剩余瓶颈归属 | **归属 spec 006**（2026-06-12 实测）：005 落地后总耗时仍 127min，其中 99.9% 墙钟耗在 `persist_mesh_results` 每批全量重写 aabb/vec3 表（全局 map 累积 37k+ 行 × 2,415 批 ÷ 2 workers，每批固定 ~5.5s 与行数无关，7696cf04 引入的 O(N²) 回归）。增量化 + 收尾补写方案见 `specs/006-incremental-aabb-pts-persistence/` |

## Requirements

1. `ams7997_0001` release 生成必须以 0 退出，日志出现成功完成标记，且不得出现新的 `job_failed`、`Cannot COMMIT`、`already exists` 或 `写入失败超出重试限制`。
2. 生成完成后必须产出并通过存在性校验：
   - `parquet/manifest_7997.json`
   - `parquet/7997/instances.parquet`
   - `parquet/7997/geo_instances.parquet`
   - `parquet/7997/transforms.parquet`
   - `parquet/7997/aabb.parquet`
3. `inst_relate_aabb` 写入必须避免逐批并发 `DELETE + INSERT` 冲突。同一 refno 的最终记录以本轮生成的最后结果为准。
4. 生成阶段允许把 `inst_relate_aabb` rows 暂存到内存或本地 spool 文件；大批量数据不得导致内存不可控增长。
5. base 写库、mesh 计算、AABB 持久化的并发和队列容量必须可通过 `DbOption-generate.toml` 配置覆盖。
6. 性能验收必须记录：
   - release 总耗时。
   - `pending_mesh_outputs` 最大值和长期贴近上限的持续情况。
   - `base_write_ms`、`inst_aabb_ms` 的 p50 / p95 / max。
   - Parquet 导出耗时与产物大小。
7. 相比当前可靠基线，总耗时至少降低 30%；若基线无法稳定成功，则以首个稳定成功版本作为稳定性基线，再比较后续优化轮次。

## Acceptance Criteria

- `cargo build --release --bin aios-database` 成功。
- 7997 release 生成稳定完成一次，随后至少再重复一次成功，排除偶发通过。
- 必需 Parquet 文件全部存在且非空，部署校验不再因 Parquet 缺失 blocking。
- 日志汇总显示：
  - `pending_mesh_outputs` 不再长期贴近 `batch_channel_capacity`。
  - `inst_aabb_ms` p95 明显下降。
  - 没有事务冲突耗尽重试。
- 生成总耗时较基线降低 >= 30%。

## Open Questions

- `inst_relate_aabb` 暂存使用内存还是 spool 文件？推荐：先内存实现，达到阈值后自动 spill 到 `output/<project>/diagnostics/spool/`。
- 统一删除范围按 `touched_refnos` 还是按 `manual_db_nums` 全 dbnum？推荐：先按 `touched_refnos`，避免误删其它并行任务数据。
- base 表是否也进入两阶段写入？推荐：先只重构 `inst_relate_aabb`，base 表保留现状并配置化并发；若仍慢，再扩展 base 表 bulk writer。
