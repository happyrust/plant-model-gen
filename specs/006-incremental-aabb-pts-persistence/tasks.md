# Tasks

> 按序执行；每个 Task 后跑 `cargo check -q`（不跑 test）。
> 验收走 `cargo build --release` + quicktest-7997-8080 站点同配置重跑实测。
> 基线 = 2026-06-12 16:20 运行（总耗时 127min / inst_aabb_ms p50=5,519ms / aabb 37,689 行）。

## 写入层增量化（主项）

- [x] **T301 `persist_mesh_results` 增量化**
  - `model_writer.rs` L281-337：删除全量 `save_pts_to_surreal(mesh_pts_map)` /
    `save_aabb_to_surreal(mesh_aabb_map)`；从本批 `mesh_results` 收集
    `aabb_hash`（Some 者）与 `pts_hashes`，在全局 map 点查取值构造局部 DashMap，
    传给原有 save_*；`inst_geo` mesh 回写 UPDATE 段不动。
- [x] **T302 收尾安全网 `finalize_mesh_entities`**（grill-me Q2=A）
  - trait `ModelWriterBackend` 新增 `finalize_mesh_entities`（默认 no-op skipped）；
    Surreal 后端实现全量 `INSERT IGNORE` 补写并返回行数；DrainOnly 走默认。
  - orchestrator.rs L1322 finish 后调用；打印
    `[gen_model] final_sweep aabb=N pts=M elapsed_ms=T`；
    `AIOS_SKIP_FINAL_AABB_SWEEP=1` 跳过并打印原因。

## pe_transform 提前刷新（次项）

- [x] **T303 precheck 覆盖探测真实化**
  - `precheck_coordinator.rs::check_pe_transform`：按 dbnum 轻量探测
    `pe_transform` 覆盖；未覆盖 → `refresh_pe_transform_for_dbnums`；
    刷新条数计入 `stats.pe_transform_refreshed`，摘要如实显示。
- [x] **T304 transform rkyv 构建失败负缓存**（加固）
  - `transform_rkyv_cache.rs`：同 dbnum 构建失败后本次运行内不再重复构建
    与刷错误日志，直接走 DB 回退（消除基线 1 秒内同库 6+ 次重试刷屏）。

## 文档收尾

- [x] **T305 spec 004 注记**
  - `specs/004-model-generation-write-pipeline-performance/spec.md` Decisions
    表加注：实测剩余瓶颈（persist_mesh_results 每批全量重写 aabb/vec3，
    占墙钟 99.9%）归属 `specs/006-incremental-aabb-pts-persistence/`。
- [x] **T306 CHANGELOG**
  - 记录：aabb/vec3 每批增量写 + 收尾一次性补写；precheck pe_transform
    覆盖探测与提前刷新；rkyv 失败负缓存。

## 验收（spec Acceptance Criteria，基线对比）

- [x] **T307 release 构建**
  - `cargo build --release --bin aios-database` 成功。
- [x] **T308 站点重跑性能验收**（2026-06-12 22:08 运行，全项通过）
  - perf：`categorize_and_inst_relate` **137,044ms（2.28min）** vs 基线 7,568,397ms（126.1min），**降幅 98.2% / 55×**；
    端到端（含导出前 pe_transform 刷新与 Parquet 导出）≈ 8.6min < 20min ✓
  - `[batch_perf]`（n=2415）：`inst_aabb_ms` 总和 136.2s（基线 15,117s，111×）；
    **p50=31ms**（基线 5,519ms）✓、**p95=134ms** ✓、max=4,359ms
  - `final_sweep aabb=1910 pts=137212 status=Executed elapsed_ms=3914` 恰好一次 ✓
  - 日志零 `does not exist`、零 `already exists`、零 `Cannot COMMIT` ✓
  - `cata_time.get_world_transform` = **0ms**（基线 14,146ms）✓
  - perf json：`profile/perf_gen_model_index_tree_dbnum_7997_20260612_220837.json`
  - 备注：验收过程中额外发现并修复 `ensure_surreal_init` 对已繁忙连接重复
    connect+signin 的客户端死锁（utils.rs，存活探针跳过重复 init）。
- [x] **T309 数据完整性比对**（CLI+json count）
  - SurrealDB：aabb=86,738、vec3=186,524、inst_relate_aabb=51,820、
    tubi_relate=4,950（重建），全部 ≥ 基线 ✓
  - Parquet 5 件套齐全：instances 40,199 行 / geo_instances 48,288 行 /
    transforms 49,165 行 / **aabb.parquet 37,802 行 ≥ 基线 37,689** ✓ /
    manifest_7997.json ✓
