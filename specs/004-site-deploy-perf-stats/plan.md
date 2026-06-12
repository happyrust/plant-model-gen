# Plan: 站点部署性能统计

## 架构总览

```
sidecar (aios-database CLI)                      web_server (admin 控制面)               admin UI
┌──────────────────────────────┐                 ┌─────────────────────────────┐        ┌──────────────┐
│ gen-cata-closure  ──┐        │                 │ job 完成事件钩子             │        │ 站点详情      │
│ parse (sync)      ──┼─ 聚合 ─→ metrics/<task>.json ─→ 读取+校验 → SQLite      │ ─API─→ │ 「性能统计」  │
│ generate (run_app)──┤        │   (原子写)       │ site_task_metrics(50条/站点) │        │  Tab          │
│ export            ──┘        │                 │ GET /sites/{id}/metrics     │        └──────────────┘
└──────────────────────────────┘                 └─────────────────────────────┘
```

## 数据模型

### metrics 产物文件（sidecar 写，schema_version=1）

`runtime/admin_sites/<site_id>/metrics/<task_id>.json`

```json
{
  "schema_version": 1,
  "task_id": "…", "job_kind": "parse|generate",
  "started_at": "RFC3339", "finished_at": "RFC3339", "duration_ms": 0, "success": true,
  "stages": {
    "closure":  { "seed_count": 0, "visited_count": 0, "rounds": 0, "missing_count": 0,
                  "covered_dbnums": [{"dbnum":0,"refnos":0,"total":0}], "pruning_ratio": 0.0, "duration_ms": 0 },
    "parse":    { "dbs": [{"dbnum":0,"db_type":"CATA","elements":0,"mode":"partial","duration_ms":0}],
                  "total_elements": 0, "error_count": 0, "duration_ms": 0 },
    "generate": { "inst_relate": 0, "inst_info": 0, "inst_relate_aabb": 0,
                  "mesh_generated": 0, "mesh_cache_hit": 0,
                  "boolean_success": 0, "boolean_failed": 0, "tubi_count": 0,
                  "stage_ms": {"geo_gen":0,"mesh":0,"base_write":0,"inst_aabb":0,"boolean":0,"spatial_tree":0},
                  "error_count": 0, "cache_miss": 0, "duration_ms": 0 },
    "export":   { "parquet_files": 0, "parquet_bytes": 0, "json_files": 0, "json_bytes": 0, "duration_ms": 0 }
  }
}
```

阶段可缺省（parse 任务无 generate/export 段）。

### SQLite 表（web_server，admin 库）

```sql
CREATE TABLE IF NOT EXISTS site_task_metrics (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  site_id TEXT NOT NULL,
  task_id TEXT NOT NULL UNIQUE,
  job_kind TEXT NOT NULL,            -- parse | generate
  started_at TEXT NOT NULL,
  finished_at TEXT,
  duration_ms INTEGER,
  success INTEGER NOT NULL DEFAULT 0,
  stages_json TEXT NOT NULL,         -- 上述 stages 原文
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_stm_site_time ON site_task_metrics(site_id, started_at DESC);
```

淘汰：插入后 `DELETE FROM site_task_metrics WHERE site_id=? AND id NOT IN (SELECT id … ORDER BY started_at DESC LIMIT 50)`。

## 采集点映射（复用既有结构，不新增热路径埋点）

| 指标 | 来源（已存在） | 接线方式 |
|---|---|---|
| closure.* | `CataClosureManifest`（seed/visited/rounds/missing/by_dbnum） | `gen-cata-closure` 写 manifest 后同步写 metrics 段 |
| parse.dbs[] | `sync_total_async_threaded` 各库循环 + `apply_sync_filter` 决策 | 每库解析完成时累加（内存聚合器），任务尾统一落盘 |
| parse.error_count | failed_sql 转储计数（`FAILED_SQL_DUMP_COUNT`） | 任务尾读取 |
| generate.数量 | `GenModelResult` / `ModelWriterFinishReport` / `MeshWorkerReport` / `BoolWorkerReport` | `run_app` 生成收尾处聚合 |
| generate.stage_ms | `PerfTimer`（已有 `generate_report/save_json`） | 直接复用其 JSON 输出嵌入 |
| generate.cache_miss | `cache_miss_report` snapshot | 任务尾读取 |
| export.* | `ParquetExportStats` / 导出文件 walk | 导出收尾处统计 |

实现载体：`src/perf_metrics.rs` 新增 `TaskMetricsCollector`（线程安全累加器 + `write_json(path)`），CLI 各模式入口创建，阶段完成处 `record_*`，进程退出前落盘。env `AIOS_TASK_METRICS_PATH` 由 web_server 在派发 sidecar job 时注入（沿用 CLI job 环境变量注入能力，与 `AIOS_CATA_CLOSURE_MODE` 同机制）。

## web_server 接线

- `run_sidecar_cli_job_with_site_events` 完成回调处：按 `AIOS_TASK_METRICS_PATH` 读产物 → 校验 schema_version → upsert `site_task_metrics` → 按 50 条淘汰。文件缺失只 `append_log_line` 提示。
- 新 handler：`admin_metrics_handlers.rs`（挂在 `create_admin_routes`）：
  - `GET /api/admin/sites/{id}/metrics?limit=` → 行列表 + 同 kind 前一条 delta（耗时、total_elements、inst_relate）。
  - `GET /api/admin/sites/{id}/metrics/{task_id}` → 单条完整 stages_json。

## UI（ui/admin）

- `SiteDetailView.vue` 新增 Tab「性能统计」：
  - 顶部四阶段卡片（最近一次成功任务）：闭包（visited/裁剪率）、解析（元素数/库数/耗时）、生成（inst/mesh/布尔/耗时）、导出（文件数/体积）；delta 用 ↑↓ 与百分比。
  - 历史表：时间 / 类型 badge / 耗时 / 关键数量 / 状态；行点击展开 stages 明细（JSON 树或分组列表）。
  - 复用 `ui/界面设计/admin/cata-parse-stats.pen` 的卡片+表格视觉语言。
- `types/site.ts` 新增 `SiteTaskMetrics` 类型；`sitesApi.getMetrics()`。

## 风险与对策

- **R1 sidecar 崩溃无产物** → job 完成钩子容错（metrics_missing 标记），UI 显示「本次无指标」。
- **R2 双进程并发写同名产物**（重试/僵尸进程） → 产物名含 task_id，写入原子；入库 task_id UNIQUE upsert。
- **R3 解析多库并行计数竞争** → 聚合器用原子计数/互斥，仅任务尾一次序列化。
- **R4 历史 50 条不够回溯** → limit 常量化（`METRICS_RETAIN_PER_SITE`），后续可配置化，不在本期。
