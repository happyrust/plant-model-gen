# 7997 Release 模型生成写入流水线架构与原理

## 产物

- 架构图 SVG：`specs/004-model-generation-write-pipeline-performance/model-generation-write-pipeline-architecture.svg`
- 流程图 draw.io：`specs/004-model-generation-write-pipeline-performance/solution-flow.drawio`
- Spec kit：
  - `specs/004-model-generation-write-pipeline-performance/spec.md`
  - `specs/004-model-generation-write-pipeline-performance/plan.md`
  - `specs/004-model-generation-write-pipeline-performance/tasks.md`

## 一句话结论

`ams7997_0001` release 生成慢，不是 CPU、内存或磁盘打满，而是模型生成流水线里“上游生产与 mesh 计算快，下游 SurrealDB 写入和 `inst_relate_aabb` 持久化慢”。因此队列看起来很满，真正要优化的是写入策略和背压控制，而不是盲目提高并发。

## 当前流水线

当前生成路径可以理解为五段：

1. `IndexTree` 生产者从 DESI/CATA/LOOP/PRIM 等目标生成 `ShapeInstancesData` batch。
2. `run_batch_sink` 把 batch 同时送入 base 写库队列和 mesh 计算队列。
3. `run_base_writer` 写入 `inst_info`、`inst_geo`、`inst_relate` 等基础表。
4. `run_mesh_stage` 生成 mesh / aabb 结果。
5. `run_inst_aabb_writer` 等待 base 写库结果与 mesh 输出 join 后，持久化 mesh 结果和 `inst_relate_aabb`。

关键代码入口：

- `src/fast_model/gen_model/orchestrator.rs`
- `src/fast_model/gen_model/model_writer.rs`
- `src/fast_model/gen_model/pdms_inst.rs`

## 为什么会“这么满”

日志里的 `pending_mesh_outputs` 不是系统内存满，而是 join 阶段暂存的 mesh 输出数量。它高说明 mesh 结果已经来了，但对应的 base 写库结果还没回来。

本次现场证据显示：

- `pending_mesh_outputs` 多次贴近 108。
- `base_write_ms` 曾达到 33 秒级。
- `inst_aabb_ms` 曾达到 35 秒级。
- `producer send_wait_ms` 出现秒级等待，说明背压已经从写库端传导到生产端。
- 系统资源并不紧张：D 盘仍有约 163GB，内存约 21GB 可用，CPU 总体约 30% 到 47%。

这说明瓶颈不在 mesh 计算，也不在机器资源，而在 SurrealDB 写事务。

## 当前写入为何慢

当前 `save_instance_data_with_report` 会在每个 batch 内写多组 SurrealDB 事务。`inst_relate_aabb` 更敏感，因为它按历史策略使用成对的：

```text
DELETE [inst_relate_aabb ids]
INSERT INTO inst_relate_aabb [...]
```

虽然单个 batch 内部已经把事务并发降到 2，但外层 base writer 默认有 8 个并发，同时多个 batch 写 SurrealDB，仍然会造成：

- 小事务数量多。
- `DELETE + INSERT` 容易与其它 batch 乱序竞争。
- retry/backoff 增加总耗时。
- 一旦唯一索引或记录冲突，事务块回滚，导致 `job_failed`。

之前已经观察到两类失败：

- `inst_relate_aabb:<refno> already exists` / `Cannot COMMIT`
- `写入失败超出重试限制`

## 提速原则

第一原则是先稳定 release 生成，再提速。原因是如果生成无法稳定成功，任何耗时数字都没有可比较价值。

因此不建议先加大并发。当前上游已经比下游快，继续提高生产并发只会让 `pending_mesh_outputs` 和 `send_wait_ms` 更严重。

推荐顺序：

1. 先做 release 性能基线。
2. 先降低上游并发和队列容量，验证稳定成功。
3. 再重构 `inst_relate_aabb` 为两阶段批量写入。
4. 最后用两次连续 release benchmark 判定是否达到 30% 提速。

## 推荐架构

推荐改成两阶段写入：

```text
生成阶段：
  生产 batch
  base 表正常写入
  mesh 正常计算
  inst_relate_aabb 只构建 rows，提交给 accumulator

收尾阶段：
  accumulator 合并同一 id，最后一条 wins
  按 touched ids 统一 DELETE
  按大块 INSERT 写入 inst_relate_aabb
  导出 Parquet 并校验产物
```

这个设计的核心收益是减少高频小事务与跨 batch 乱序冲突。`inst_relate_aabb` 的唯一性由 accumulator 在内存或 spool 阶段解决，而不是交给 SurrealDB 在高并发事务里反复冲突。

## 为什么两阶段会更快

两阶段 bulk writer 有三个直接收益：

- 减少事务数量：从每个 batch 多次 `DELETE + INSERT`，变成收尾集中处理。
- 减少冲突窗口：同一 refno 的重复记录先在本地 last-wins 去重，避免数据库唯一冲突。
- 降低背压：`inst_aabb` 阶段不再每批等待慢事务，join 队列更容易排空。

这不会牺牲最终一致性，因为 `inst_relate_aabb` 是生成结果的派生关系表，最终产物以本轮 touched refnos 的最后结果为准。

## 稳定化配置建议

第一轮建议使用稳定优先配置：

```toml
index_tree_max_concurrent_targets = 3
index_tree_batch_size = 100
batch_channel_capacity = 24
base_write_concurrency = 4
mesh_compute_concurrency = 4
inst_aabb_write_concurrency = 2
```

如果仍然出现 SurrealDB 写入冲突，优先把 `base_write_concurrency` 降到 2 做对照，而不是继续增加并发。

## 验收口径

最终验收必须同时满足：

- `cargo build --release --bin aios-database` 成功。
- `ams7997_0001` release 生成两次连续成功。
- 必需 Parquet 文件全部存在且非空：
  - `manifest_7997.json`
  - `instances.parquet`
  - `geo_instances.parquet`
  - `transforms.parquet`
  - `aabb.parquet`
- 日志无新的 `job_failed`、`Cannot COMMIT`、`already exists`、`写入失败超出重试限制`。
- 相比稳定基线，总耗时降低至少 30%。

## 后续实施路径

按 `tasks.md` 执行：

1. T001 建立 release 性能基线。
2. T002 配置稳定化试跑。
3. T003 抽象 `InstRelateAabbAccumulator`。
4. T004 接入两阶段 `inst_relate_aabb` 写入。
5. T005 写入参数配置化。
6. T006 release 验证与 30% 提速判定。
7. T007 更新文档和回归保护。
