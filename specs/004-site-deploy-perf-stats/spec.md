# Feature Specification: 站点部署性能统计（任务级指标采集与展示）

## User Need

站点部署目前没有成体系的性能统计：只有 `last_parse_duration_ms`（仅解析、仅最近一次）、即时资源快照与日志文本。解析了多少元素/几个库、生成了多少实例/网格/布尔、各阶段耗时、历史趋势，全部散落在 sidecar 日志里，无法在 admin UI 查看，也无法跨任务对比。需要任务级（每次 parse / generate 作业一条）的结构化性能指标：采集、持久化、API、UI 展示与历史对比。

## 决策记录（grill-me 已确认，2026-06-12）

| 分支 | 决策 |
|---|---|
| 统计范围 | **四阶段全覆盖**：CATA 闭包、解析、模型生成、导出 |
| 持久化 | **admin SQLite 新表 `site_task_metrics`**（一任务一行，阶段明细 JSON 列） |
| 采集方式 | **sidecar 各阶段落 JSON 产物文件**，job 完成时 web_server 读取入库（不解析日志，守 sidecar 边界） |
| UI 呈现 | 站点详情新增**「性能统计」Tab**（四阶段卡片 + 历史趋势表） |
| 历史保留 | 每站点保留**最近 50 次**，按任务时间淘汰；API 返回与上一次的 delta |

## Scope

- sidecar（aios-database CLI）侧：闭包 / 解析 / 生成 / 导出四阶段的指标采集与 `metrics.json` 产物落盘。
- web_server（admin 控制面）侧：job 完成钩子读取产物 → `site_task_metrics` 入库 → REST API。
- admin UI：站点详情「性能统计」Tab。
- 历史保留与跨任务 delta 对比。

## 指标清单（按阶段）

### 1. CATA 闭包（stage = `closure`）
- `seed_count` / `visited_count` / `rounds` / `missing_count`（直接来自 `CataClosureManifest`）
- `covered_dbnums`（manifest 覆盖的库及各库 refno 数）、`pruning_ratio`（visited / 覆盖库全量元素数）
- `duration_ms`

### 2. 解析（stage = `parse`）
- 各库：`dbnum` / `db_type` / `element_count`（落库 pe 数）/ `mode`（full | partial | skipped）/ `duration_ms`
- 汇总：`total_elements` / `db_count` / `duration_ms` / `error_count`（写入失败转储计数）

### 3. 模型生成（stage = `generate`）
- 数量：`inst_relate` / `inst_info` / `inst_relate_aabb` / `mesh_generated` / `mesh_cache_hit` / `boolean_tasks(success/failed)` / `tubi_count`
- 耗时：`total_ms` 与分段（`geo_gen_ms` / `mesh_ms` / `base_write_ms` / `inst_aabb_ms` / `boolean_ms` / `spatial_tree_ms`，来自 PerfTimer 分段）
- `error_count`（failed_sql 转储数）+ `cache_miss`（cache_miss_report 汇总）

### 4. 导出（stage = `export`）
- `parquet_files` / `parquet_bytes` / `json_files` / `json_bytes` / `duration_ms`

## Requirements

1. **采集不引入新瓶颈**：指标在既有统计结构（`PerfTimer`、`MeshWorkerReport`、`BoolWorkerReport`、`GenModelResult`、`ParquetExportStats`、`CataClosureManifest`）基础上聚合，不新增热路径埋点；阶段结束时一次性写 `runtime/admin_sites/<site_id>/metrics/<task_id>.json`（原子写 tmp+rename）。
2. **sidecar 边界**：web_server 不解析日志、不读 E3D 数据；只在 job 完成事件里读取 metrics 产物文件入库。产物缺失时记 `metrics_missing=true`，不阻塞任务状态流转。
3. **持久化**：`site_task_metrics(site_id, task_id, job_kind, started_at, finished_at, duration_ms, success, stages_json, created_at)`；建表走既有 `ensure_schema/ensure_column_exists` 模式；每站点超过 50 条按 `started_at` 淘汰。
4. **API**：
   - `GET /api/admin/sites/{id}/metrics?limit=N` → 最近 N 条任务指标（默认 10，含 delta：与上一条同 kind 任务的耗时/数量差值）。
   - `GET /api/admin/sites/{id}/metrics/{task_id}` → 单任务完整阶段明细。
5. **UI**：站点详情新增「性能统计」Tab：最近一次任务的四阶段卡片（数量+耗时+与上次 delta 箭头）、历史任务表（时间/类型/耗时/元素数/实例数/状态）；无数据时显示引导空态。
6. **兼容**：旧任务无 metrics 产物 → API 返回空列表；`last_parse_duration_ms` 既有字段保留不动。

## Non-Goals

- 不做跨站点全局聚合报表（先做单站点维度）。
- 不做实时进行中任务的流式指标（只统计已完成任务；进行中沿用现有日志/SSE）。
- 不引入 Prometheus / OpenTelemetry 等外部观测栈。
- 不跑 Rust test / 不编译 test 目标（仓库规则）；验证走 HTTP 实测。

## 实现偏差记录（T110 回填，2026-06-12）

| 计划 | 实现 | 原因 |
|---|---|---|
| closure 段含 `pruning_ratio` 与各库 total | closure 段只记 `covered_dbnums[{dbnum,refnos}]`；解析段每库新增 `total_in_file`，裁剪率由 `elements/total_in_file` 派生 | 闭包 pass 不读 CATA 文件全量元素数，避免额外 IO |
| generate 数量聚合 `GenModelResult` 等报告结构 | `GenModelResult` 仅含 success；数量改为生成收尾 Surreal `count()`（inst_relate/inst_info/inst_relate_aabb/tubi_relate），mesh/布尔计数在 orchestrator 汇总点埋点 | 与验收口径（库内实数）天然一致，避免穿透多层管线传报告 |
| web_server 侧 `admin_metrics_handlers.rs` | 入库+API 合并为 `site_task_metrics.rs` 单模块 | 入库与查询共享表结构与 schema 函数 |
| 产物仅任务尾落盘 | 每次 `record_*` 即原子 flush；进程启动 merge-on-load | 闭包与解析是两个 CLI 进程共用一个 task 产物；中断也保留已完成阶段 |
| `job_kind` 由 sidecar 推断 | web_server 注入 `AIOS_TASK_METRICS_KIND` 显式指定（缺省仍按阶段推断） | 消除 parse 任务里含 generate 段时的歧义 |

## Acceptance Criteria

- 跑一次完整部署（parse + generate）后：`metrics/` 目录出现两个任务产物 JSON；`site_task_metrics` 各入库一行；`GET .../metrics` 返回闭包/解析/生成/导出四阶段数据，数值与 sidecar 日志尾部 summary 一致（抽查 inst_relate 数、闭包 visited、解析元素数三项）。
- 第二次部署后 API 返回 delta 字段（耗时与数量差值）。
- UI「性能统计」Tab 正确渲染卡片与历史表；无 metrics 的旧站点显示空态不报错。
- 每站点指标行数不超过 50（插入第 51 条后最旧一条被淘汰）。
- `scripts/guard/web_server_parse_boundary_guard.ps1` PASS（web_server 未新增解析域逻辑）。
