# Tasks

> 按序执行；每个 Task 后跑 `cargo check -q`（不跑 test）。web_server 改动完成后必须跑
> `scripts/guard/web_server_parse_boundary_guard.ps1` 并用运行服务 + HTTP 实测验证。

## sidecar 采集

- [x] **T101 `TaskMetricsCollector` 基础设施**
  - `src/perf_metrics.rs`：schema_version=1 的产物结构（serde）、线程安全累加器、`write_json(path)` 原子落盘；
    env `AIOS_TASK_METRICS_PATH` 读取入口（无值时全程 no-op，零开销）。
  - 实现补充：每次 `record_*` 即 flush（中断也保留已完成阶段）；启动时 merge-on-load 已有产物
    （闭包 job 与解析 job 两进程共用同一 task 产物）；`AIOS_TASK_METRICS_KIND` 可显式指定 job_kind。
- [x] **T102 闭包阶段接线**
  - `gen-cata-closure` 写 manifest 后填 `closure` 段（seed/visited/rounds/missing/by_dbnum/耗时）。
  - 偏差：裁剪率不在闭包段记录（闭包 pass 不读 CATA 文件全量数），由解析段 `dbs[].total_in_file` 派生。
- [x] **T103 解析阶段接线**
  - `sync_total_async_threaded(_with_callback)` / `parse_single_db_file`：每库完成记 `dbnum/db_type/elements/mode/duration_ms`（mode 来自 `apply_sync_filter` 决策：full|partial|skipped）；任务尾记 `error_count`（failed_sql 计数）并落盘。
- [x] **T104 生成阶段接线**
  - 偏差（更稳的等价口径）：数量不走 `GenModelResult`（其只含 success 标志）——`lib.rs` 生成收尾以
    Surreal `count()` 落 inst_relate/inst_info/inst_relate_aabb/tubi_relate；mesh 新建/命中取
    orchestrator 批量屏障汇总点；布尔 success/failed 取 boolean_bridge 报告；分段耗时取
    `index_tree_generation` 的 PerfTimer 全部分段；cache_miss 取全局报告 bucket 计数和。
- [x] **T105 导出阶段接线**
  - 导出收尾 walk `output/<project>/parquet` 目录统计 parquet/json 文件数与字节数 + 导出总耗时。

## web_server 入库与 API

- [x] **T106 metrics 产物入库**
  - 派发 parse/generate sidecar job 时注入 `AIOS_TASK_METRICS_PATH=runtime/admin_sites/<site_id>/metrics/<task_id>.json`；
    job 完成回调读取产物 → `site_task_metrics` upsert（task_id UNIQUE）→ 每站点按 `started_at` 保留 50 条；缺产物记日志不阻塞。
  - 实现位置：新模块 `src/web_server/site_task_metrics.rs`（建表内联 `ensure_metrics_schema`，admin SQLite）。
- [x] **T107 REST API**
  - 偏差：handler 与入库同在 `site_task_metrics.rs`（未单开 `admin_metrics_handlers.rs`）；
    `GET /api/admin/sites/{id}/metrics?limit=`（含同 kind 前一条 delta）与 `GET .../metrics/{task_id}`，
    merge 进 `create_admin_routes`（admin 鉴权层内）。
- [x] **T108 站点详情「性能统计」Tab**
  - `SiteDetailView.vue` 新 Tab（URL `?tab=metrics`）：四阶段卡片 + 历史任务表（行点击展开 stages JSON）+ 空态；
    `types/site.ts` / `api/sites.ts` 增类型与 `metrics()`；`npx vite build` 已重建静态产物。

## 验收

- [ ] **T109 端到端验收**
  - 对测试站点跑一次完整部署（parse + generate）：核对产物 JSON、SQLite 行、API 数值与 sidecar 日志 summary 三方一致（抽查 inst_relate / visited / total_elements）；
    第二次部署后核对 delta；boundary guard PASS（已过）；浏览器实测 Tab 渲染与空态。
  - 进度：代码侧 `cargo check` 双 bin 绿 + guard PASS；待 release 重建与重新部署后完成数值核对
    （需等当前运行中的部署作业结束以释放可执行文件锁）。
- [x] **T110 文档收尾**
  - CHANGELOG 已记录（2026-06-12「站点部署性能统计」条目）；本文件与 spec.md 已回填实现偏差。
