# Tasks: 7997 release 模型生成写入流水线提速（spec 004）

## T001 — 建立 release 性能基线

- 编写日志汇总脚本，解析 `generate.log` 中的：
  - `pending_mesh_outputs`
  - `base_write_ms`
  - `inst_aabb_ms`
  - `send_wait_ms`
  - `job_failed` / `Error` / `写入失败超出重试限制`
- 输出 `perf-summary.json`。
- 记录当前 7997 release 配置与运行结果。

验收：

- 脚本能对现有 `quicktest-7997-final-8081/logs/generate.log` 输出 summary。
- summary 能明确指出当前瓶颈在写库 / AABB 持久化。

## T002 — 配置稳定化试跑

- 给 `DbOption-generate.toml` 增加一组稳定优先配置：
  - `index_tree_max_concurrent_targets = 3`
  - `index_tree_batch_size = 100`
  - `batch_channel_capacity = 24`
  - `base_write_concurrency = 4`
  - `mesh_compute_concurrency = 4`
  - `inst_aabb_write_concurrency = 2`
- 使用 release binary 运行 7997。
- 汇总 perf 和 Parquet 产物。

验收：

- release 生成不出现 fatal 写入错误。
- 若失败，summary 能定位新的失败点。

## T003 — 抽象 `InstRelateAabbAccumulator`

- 新增 accumulator 类型：
  - `push_batch(aabb_rows, inst_rows, inst_ids)`
  - `dedupe_last_wins()`
  - `touched_ids()`
  - `finish()`
- 复用现有 `dedupe_inst_relate_aabb_rows` 语义。
- 加单元测试覆盖重复 id last-wins。

验收：

- 重复 `inst_relate_aabb` id 只保留最后一条。
- accumulator 不依赖 SurrealDB，可纯内存测试。

## T004 — 两阶段 `inst_relate_aabb` 写入接入

- 修改 `SurrealModelWriterBackend::persist_inst_relate_aabb`：
  - 只构建 rows 并交给 accumulator。
  - 不在每个 batch 内执行 `save_inst_relate_aabb_rows`。
- 在生成收尾阶段统一 flush：
  - bulk DELETE touched ids。
  - bulk INSERT deduped rows。
- 失败时转储 SQL 和 accumulator summary。

验收：

- 日志中不再出现逐 batch `inst_relate_aabb` 长耗时。
- 不再出现同一 id 的 `already exists`。

## T005 — 写入参数配置化

- 将以下硬编码提到 `DbOptionExt`：
  - `CHUNK_SIZE`
  - `MAX_TX_STATEMENTS`
  - `MAX_CONCURRENT_TX`
  - bulk AABB chunk size
  - bulk AABB max tx statements
  - bulk AABB max concurrent tx
- 默认值保持稳定优先。

验收：

- 旧配置缺省时行为兼容。
- 新配置能在 `DbOption-generate.toml` 中覆盖。

## T006 — release 验证与 30% 提速判定

- 运行两次 7997 release。
- 检查必需产物：
  - `manifest_7997.json`
  - `instances.parquet`
  - `geo_instances.parquet`
  - `transforms.parquet`
  - `aabb.parquet`
- 对比 T001 / T002 基线总耗时。

验收：

- 两次连续成功。
- Parquet 文件完整且非空。
- 无 fatal 写入错误。
- 总耗时降低 >= 30%。

## T007 — 文档与回归保护

- 更新 spec 004 的 perf 结果。
- 在 `CHANGELOG.md` 记录写入流水线重构。
- 如果新增 CLI/log 脚本，补使用说明。

验收：

- 后续维护者能复现 7997 release benchmark。
- spec、plan、tasks 与实际实现保持一致。
