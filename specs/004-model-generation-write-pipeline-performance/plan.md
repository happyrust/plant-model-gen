# Implementation Plan: 7997 release 模型生成写入流水线提速（spec 004）

## Approach

把提速分为两条线并行推进：

1. **稳定线**：先让 release 生成稳定完成，避免 SurrealDB 事务冲突、重复写入和队列长期满载。
2. **性能线**：在稳定成功的基础上，用可复现 benchmark 逐轮调并发和批量写入策略，达到 30% 提速。

核心设计是把 `inst_relate_aabb` 从“每个 batch 内立即 DELETE+INSERT”改为“两阶段批量写入”：

```
生成阶段:
  base batch 写基础 inst / geo / relate
  mesh stage 计算 mesh + aabb
  inst_aabb stage 只构建 rows 并写入 accumulator/spool

收尾阶段:
  合并同一 refno 的最终 row
  按 touched_refnos 统一 DELETE
  按大块 INSERT inst_relate_aabb
  写入 aabb / mesh 引用完成标记
```

## Root Cause

当前管线中 mesh 计算通常是毫秒级，base 写库和 `inst_relate_aabb` 持久化是秒级到几十秒级。`pending_mesh_outputs` 长期高位说明 join 阶段主要在等 `base_result`，而不是等 mesh 结果。

关键代码路径：

- `src/fast_model/gen_model/orchestrator.rs`
  - `run_batch_sink`
  - `run_base_writer`
  - `run_mesh_stage`
  - `run_inst_aabb_writer`
  - `process_inst_aabb_batch`
- `src/fast_model/gen_model/pdms_inst.rs`
  - `save_instance_data_with_report`
  - `save_inst_relate_aabb_rows`
  - `TransactionBatcher`
- `src/fast_model/gen_model/model_writer.rs`
  - `SurrealModelWriterBackend::write_base_batch`
  - `SurrealModelWriterBackend::persist_inst_relate_aabb`

当前 `save_instance_data_with_report` 内部已把事务并发降到 2，但外层 base writer 仍有 8 个并发；多个 batch 同时写 SurrealDB 时，整体事务压力仍然很高。

## Phase 0 — Baseline and Safety

目标：拿到可比较的 release 基线，避免之后“感觉更快”。

1. 新增或整理一个日志汇总脚本，解析 `generate.log`：
   - 总耗时。
   - `pending_mesh_outputs` max / count / 最后值。
   - `base_write_ms` p50 / p95 / max。
   - `inst_aabb_ms` p50 / p95 / max。
   - `send_wait_ms` p50 / p95 / max。
   - `job_failed` / `Error` / `写入失败超出重试限制` 计数。
2. 固化 7997 release 运行命令和产物校验命令。
3. 记录当前配置作为 baseline：
   - `index_tree_max_concurrent_targets = 6`
   - `index_tree_batch_size = 200`
   - `base_write_concurrency = 8`
   - `mesh_compute_concurrency = 4`
   - `inst_aabb_write_concurrency = 2`
   - `batch_channel_capacity = 100`

## Phase 1 — Configuration Stabilization

目标：先降低写库冲突，找到稳定成功配置。

推荐第一组参数：

```toml
index_tree_max_concurrent_targets = 3
index_tree_batch_size = 100
batch_channel_capacity = 24
base_write_concurrency = 4
mesh_compute_concurrency = 4
inst_aabb_write_concurrency = 2
```

验证：

- release 生成必须完整成功。
- Parquet 必须完整产出。
- `pending_mesh_outputs` 不再长期贴近 `batch_channel_capacity`。
- 如果 `base_write_ms` 仍高，继续降 `base_write_concurrency` 到 2 做对照。
- 如果 CPU 低且写库稳定，再把 `index_tree_max_concurrent_targets` 从 3 提到 4。

## Phase 2 — Two-Phase `inst_relate_aabb` Writer

目标：移除生成中最容易冲突和拖慢的逐批 `DELETE + INSERT`。

### New Components

- `InstRelateAabbAccumulator`
  - 接收每个 batch 的 `(aabb_rows, inst_relate_aabb_rows, inst_relate_aabb_ids)`。
  - 按 id 去重，最后一条 wins。
  - 记录 touched refnos。
  - 超过内存阈值时 spill 到 spool 文件。
- `flush_inst_relate_aabb_bulk`
  - 输入 accumulator 的最终 rows。
  - 先按 touched ids/dbnum 统一 `DELETE`。
  - 再按大块 `INSERT INTO inst_relate_aabb [...]`。
  - 批大小与事务语句数可配置。

### Orchestrator Changes

- `process_inst_aabb_batch` 不再直接调用 `save_inst_relate_aabb_rows`。
- `persist_inst_relate_aabb` 改为构建 rows 并提交到 accumulator。
- `run_inst_aabb_writer` 收尾时执行 accumulator flush，并把结果纳入 `ModelWriterFinishReport` 或新的 finish report。

### Correctness Rules

- 同一 `inst_relate_aabb:<refno>` 重复出现时，最终 row 覆盖前一 row。
- bulk DELETE 和 bulk INSERT 不得被拆成并发乱序事务。
- flush 失败必须转储 SQL 与 accumulator summary。

## Phase 3 — Configurable Transaction Tuning

目标：把硬编码调参变成可 benchmark 的配置。

新增/复用配置项：

```toml
model_write_chunk_size = 100
model_write_max_tx_statements = 4
model_write_max_concurrent_tx = 2
inst_aabb_bulk_chunk_size = 500
inst_aabb_bulk_max_tx_statements = 8
inst_aabb_bulk_max_concurrent_tx = 1
inst_aabb_spool_threshold_rows = 100000
```

默认值保持稳定优先。release benchmark 可以逐步增大 bulk chunk，而不是增大上游并发。

## Phase 4 — Benchmark and Acceptance

每轮运行都输出一个 `perf-summary.json`：

```json
{
  "site_id": "quicktest-7997-final-8081",
  "dbnum": 7997,
  "release": true,
  "success": true,
  "duration_ms": 0,
  "pending_mesh_outputs": { "max": 0, "last": 0 },
  "base_write_ms": { "p50": 0, "p95": 0, "max": 0 },
  "inst_aabb_ms": { "p50": 0, "p95": 0, "max": 0 },
  "send_wait_ms": { "p50": 0, "p95": 0, "max": 0 },
  "errors": [],
  "parquet_files": {
    "manifest_7997": true,
    "instances": true,
    "geo_instances": true,
    "transforms": true,
    "aabb": true
  }
}
```

Acceptance:

- 两次连续 release 成功。
- 必需 Parquet 文件全部存在且非空。
- 无 fatal 写入错误。
- 总耗时相对稳定基线降低 >= 30%。

## Risks

- R1：两阶段写入改变可见时序。缓解：生成期间不依赖 `inst_relate_aabb` 查询；若有依赖，保留兼容路径或延后切换。
- R2：accumulator 内存增长。缓解：spool 阈值和 summary 监控。
- R3：bulk DELETE 范围过大误删。缓解：默认按 touched ids，不按整 dbnum。
- R4：调低并发可能单轮更慢。缓解：目标是稳定后总耗时降低，benchmark 驱动取最优点。
- R5：Parquet 导出仍可能是独立瓶颈。缓解：Phase 4 单独记录导出耗时，必要时另立 spec。
