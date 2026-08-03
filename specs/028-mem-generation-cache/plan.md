---
status: merged-into-030
---

# Implementation Plan

> **已收编，不再实施**。见 [spec.md](spec.md) 顶部的收编说明与
> [specs/030-zone-stream-initialization/plan.md](../030-zone-stream-initialization/plan.md)。
> 本文件中仍然直接适用于 spec 030 的部分：§1 引擎配置面（`DbConnMode::Mem`）、
> §2 连接串参数泛化、§3 双连接共存（`SUL_DB` 留给工作区、回填目标用独立连接）、
> §4 回填抽象的 trait 形状（open → 按表流式搬运 → commit/abort）。

## Architecture

1. **引擎配置面**（rs-core，doc C §③ 最小改动清单）：`DbConnMode` 加 `Mem` 变体（serde rename `mem`）；`SurrealDbConfig::conn_str()` 与 `DbOption::surrealdb_conn_str()` 加 `mem://` 分支；`init_surreal` 与 `initialize_databases` 加匹配臂——等同 `File`（嵌入式、无 signin），但**不套** `#[cfg(feature="kv-rocksdb")]`、不走 RocksDB LOCK 清理与端口释放。顺手对齐两处 `File` 分支现存的 cfg 门控不一致（doc C 附带发现 §2）。

2. **连接串参数泛化**：`src/options.rs:33 rocksdb_conn_str` 是 rocksdb 专用；改为按 mode 组装，`?versioned=&retention=` 的拼法对 mem 完全同形（fork `ds.rs:582-593` 三个引擎共用同一套 `datastore_*` 解析）。开关沿用 `ModelWriterMode` / `TransformWriteBackend` 的 env + toml 双入口先例。

3. **双连接共存**：bootstrap 期需要 mem（轮转工作区）与 RocksDB（回灌目标）同时在线。`SUL_DB` 保持为轮转工作区的那一个，回灌目标用独立连接对象持有，避免全仓 `project_primary_db()` 调用点被迫带路由参数。回灌是唯一同时接触两侧的地方。

4. **回灌抽象**：新增 `GenerationOutputBackfill` trait（open → 按表流式搬运 → commit/abort），首版实现 `SurrealqlBackfill`（收尾单线程、大批 INSERT、无 DELETE 交错），二进制搬运版作为第二实现预留。命名与 `CONTEXT.md` 的「生成产物回灌」对齐。

5. **bootstrap 编排器**：新模块承载「公共依赖层预灌 → for each DESI dbnum: 解析 → 生成 → 回灌 → range 清理」的循环。复用现成零件——解析侧 `manual_db_files` / `selected_db_file_names`，生成侧 `manual_db_nums`（`gen_pipeline.rs:654`）——编排器只负责调度、清理与断点记录，不重写解析或生成。

6. **公共依赖层范围计算**：读 `db_index.sqlite` 的 dbnum→dbnum 依赖边（`db_index.rs:870`）取并集，叠加 `SYSTEM_SYNC_DB_TYPES`，CATA 子集沿用 spec 002 的闭包清单。允许滞后，漂移由生成期兜底链自愈。

7. **轮转层清理**：按 dbnum range 删除该轮 PE/ATT 与模型产物。清理范围必须与回灌范围同源计算，防止误删公共依赖层。

8. **断点续跑**：以 dbnum 为单位记录「已回灌」状态；重跑时跳过已完成 dbnum。回灌未完成的 dbnum 一律整轮重来，不做半态修复。

9. **可观测性**：每轮记录解析耗时、生成耗时、回灌耗时、回灌行数、mem 峰值占用；沿用现有 `PerfTimer` 与 `output/<project>/profile/` 落盘口径，便于与 RocksDB 直跑基线逐轮对比。

## Rollout

- 分两阶段：阶段一只落 bootstrap（本 spec）；阶段二把常驻生成缓存库（增量场景）接到同一套抽象上，届时才需要处理锚点/水位/lease 的易失性（doc C §④ 列为高危的四条冲突）。
- mem 模式是显式配置项，默认关闭。站点常驻服务不得启用——doc C 的结论是内存模式只适合一次性「解析 → 生成 → 导出」的短生命周期场景。
- 验证遵循 AGENTS.md 约束：CLI + JSON 断言、`db-data/*.surql`、`scripts/smoke/*.ps1`，不使用 `cargo test`；web 相关一律起服务后走 HTTP。

## Dependencies

- 需要先定 Q3（回灌原语形态）。若最终选二进制搬运，则新增 `surrealdb-core` 依赖或 fork SDK 层搬运 API 成为前置项，工期与风险都会显著上升。
- 需要工程真实规模数据（公共依赖层占比、单个 DESI 元素量）复核 Q4「分层常驻」相对「每轮全新实例」的优劣，以及内存预算取值。
- 与增量链路无耦合，可与 specs/026/027 的后续工作并行。
