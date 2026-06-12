# Feature Specification: aabb/vec3 增量持久化（消除每批全量重写，spec 006）

## User Need

spec 004/005 落地后，7997 release 模型生成"还是这么慢"：2026-06-12 16:20 实测总耗时
**127 分钟**，其中 `categorize_and_inst_relate` 阶段占 99.25%（7,568 秒）。需要把模型
生成总耗时压回分钟级，且不牺牲 spec 005 已建立的幂等写入语义。

## Evidence（2026-06-12 实测，quicktest-7997-8080 站点）

基线产物：
- perf：`output/AvevaMarineSample/profile/perf_gen_model_index_tree_dbnum_7997_20260612_182741.json`
  （total_ms=7,625,607；categorize_and_inst_relate=7,568,397ms / 99.25%）
- 日志：`runtime/admin_sites/quicktest-7997-8080/logs/generate.log`（2,415 条 `[batch_perf]`）

瓶颈定位（占墙钟 99.9%）：
- `inst_aabb` writer 阶段（`worker_pool=2`）`inst_aabb_ms` **累计 15,117 秒（252 分钟）**，
  除以 2 workers = 7,558 秒，与 perf 阶段墙钟 7,568 秒 **99.9% 吻合** —— 该阶段就是全部瓶颈。
- 每批成本固定 ~5.5 秒（p50=5,519ms，p10=4,920，p90=7,908，max=62,252），
  **与批内行数无关**：写 13 行的批次 ~5s，写 1,092 行的批次同样 5.2s。
- 根因：`model_writer.rs::persist_mesh_results`（L305-306）每批把**全局累积**的
  `mesh_aabb_map` / `mesh_pts_map`（orchestrator.rs L1224-1226 创建、全程共享、
  BRAN/CATE 阶段早期即填到数万条，最终 aabb=37,689 行）整体传给
  `save_aabb_to_surreal`（300 行/chunk）与 `save_pts_to_surreal`（100 行/chunk），
  每批发出 100+ 条 `INSERT IGNORE` WS 往返 → **O(N²) 回归**。
  该形态由 7696cf04（ModelWriter trait 重构）把"跑完存一次"变成"每批存全量"引入；
  spec 005 的 INSERT IGNORE 使其不再报错，只是默默浪费时间。
- 对照：base 写库累计 2,866 秒 / 8 workers ≈ 6 分钟墙钟，不是瓶颈。

次要根因（同一条"还是慢"证据链）：
- 生成开始即报 `The table 'pe_transform' does not exist`
  （`transform_rkyv_cache.rs:233`、`mesh_generate.rs:2252`、rs-core `pe_transform.rs:50`），
  transform rkyv 缓存构建失败全程回退 DB 逐条查询；BRAN 阶段
  `cata_time.get_world_transform=14,146ms`（27.4s 的 cata_gen 内）。
- `precheck_coordinator.rs::check_pe_transform`（L202-221）只做内存 cache 初始化，
  **不探测 SurrealDB `pe_transform` 表覆盖**，precheck 显示 ✅ 但运行期持续报错。
- `pe_transform` 刷新被安排在生成**结束后**的 Parquet 导出前
  （144,577 节点 / 4 分 13 秒），对生成本身没有帮助。

## 决策记录（grill-me 已确认，2026-06-12）

| # | 分支 | 决策 |
|---|---|---|
| Q1 | spec 定位 | **新开 spec 006**（独立新根因）；范围 = ①aabb/pts 增量化（主）+ ②pe_transform 提前到 precheck 刷新（次）；db_type 日志归属修复**不纳入**（另行小修）；004 加注记指向本 spec |
| Q2 | 安全网语义 | **方案 A**：收尾一次全量 `INSERT IGNORE` 补写（默认开，env `AIOS_SKIP_FINAL_AABB_SWEEP` 可关）。补齐 ≠ 清理/预查，不违反 spec 005 不变量；兜住"mesh 文件在、DB 行不在"的跨运行状态漂移 |
| Q3 | 验收标准 | 同站点同配置重跑：总耗时 <20min；`inst_aabb_ms` p50<200ms / p95<1s；aabb/vec3 行数 ≥ 基线；Parquet 5 件套齐全且 aabb.parquet ≥37,689 行；收尾补写一次 <30s；零 pe_transform 报错；CATE `get_world_transform` <2s；零 `already exists`/`Cannot COMMIT` |

## 不变量（继承并扩展 spec 005 写入契约）

1. 写入层**永不** DELETE、永不预查旧数据；aabb/vec3 增量写与收尾补写均为 `INSERT IGNORE`。
2. 旧数据清理唯一权威仍是 `pre_cleanup_for_regen`（仅 `--regen-model` 触发）。
3. 每批 `persist_mesh_results` 只写**本批 delta**：由 `batch.mesh_results` 的
   `aabb_hash` / `pts_hashes` 还原，不引入跨批共享的 dirty-set 状态。
4. 收尾补写是**补齐**语义（幂等、一次性、可关闭），不是写入正确性的前提；
   增量路径自身必须完整覆盖本轮新生成的全部 aabb/pts。

## Scope

- `persist_mesh_results` 增量化：从 `mesh_results` 构造本批 (aabb_hash→Aabb) 与
  (pts_hash→json) 局部视图，只写 delta；`inst_geo` mesh 回写 UPDATE 部分不变。
- 收尾安全网：`ModelWriterBackend` 增加收尾补写钩子，Surreal 后端在
  sink 完成后（orchestrator.rs L1322 finish 处）全量 `INSERT IGNORE` 补写一次，
  打印行数与耗时；`AIOS_SKIP_FINAL_AABB_SWEEP` 可关；DrainOnly 后端 no-op。
- precheck `check_pe_transform` 真实化：探测目标 dbnum 在 `pe_transform` 表的覆盖，
  未覆盖则调用既有 `refresh_pe_transform_for_dbnums` 刷新并计入
  `pe_transform_refreshed` 统计。
- （加固）transform rkyv 构建失败负缓存：同 dbnum 失败后本次运行内不再反复重建
  （基线日志 1 秒内同库重试 6+ 次并刷错误日志）。

## Non-Goals

- 不改 `save_aabb_to_surreal` / `save_pts_to_surreal` 的其他调用方
  （`cata_model.rs` tubi、`manifold_bool.rs`、`mesh_generate.rs` 传入的本就是局部 map）。
- 不调 `inst_aabb` worker_pool / 批大小等并发参数（spec 004 的配置化领域；
  增量化后 2 workers 预期绰绰有余，实测后再议）。
- 不动 Parquet 导出前的 pe_transform 刷新（导出正确性职责，与 precheck 刷新互补）。
- 不修 `db_type is X` 日志归属问题（解析端可读性小修，单独处理）。
- 不写 cargo test / 不编译 test 目标（仓库规则）；验证走 release 构建 + 站点实测。

## Requirements

1. `persist_mesh_results` 每批只写本批 delta：批内 `mesh_results` 的
   `aabb_hash`（Some 者）与 `pts_hashes` 所对应的行；缓存命中且 `aabb_hash=None`
   的 mesh 本批不写（前提：行已存在，由收尾补写兜底漂移）。
2. 增量写沿用现行 chunked `INSERT IGNORE`（aabb 300/chunk、vec3 100/chunk），
   失败仍走 `init_save_database_error` 转储。
3. 收尾补写在 ModelWriter finish 后、boolean 阶段前执行一次：全量
   `mesh_aabb_map` / `mesh_pts_map` `INSERT IGNORE`；日志含行数与耗时；
   `AIOS_SKIP_FINAL_AABB_SWEEP=1` 时跳过并打印跳过原因。
4. precheck 必须真实探测 `pe_transform` 覆盖：表缺失或目标 dbnum 未覆盖时
   同步刷新（沿用 `refresh_pe_transform_for_dbnums`），刷新条数计入摘要；
   覆盖完好时探测成本 ≤ 1 次轻量查询/dbnum。
5. 生成全程日志不得出现 `The table 'pe_transform' does not exist`。
6. `[batch_perf]` 指标含义不变（`inst_aabb_ms` 仍计量该批 writer_backend 阶段耗时），
   保证与基线可直接对比。

## Acceptance Criteria（基线 = 2026-06-12 16:20 运行）

- `cargo build --release --bin aios-database` 成功。
- 同站点 `quicktest-7997-8080` 同配置（batch_size=200 / concurrency=6 /
  gen_mesh=true / apply_boolean=true）重跑：
  1. perf json 总耗时 **< 20 分钟**（基线 127 分钟）。
  2. `[batch_perf]` 统计：`inst_aabb_ms` **p50 < 200ms、p95 < 1s**（基线 p50=5,519ms）。
  3. SurrealDB `aabb` / `vec3` 表行数 **≥ 基线**；Parquet 5 件套
     （manifest_7997.json / instances / geo_instances / transforms / aabb）齐全，
     `aabb.parquet` **≥ 37,689 行**。
  4. 收尾补写日志恰好一次，耗时 **< 30s**。
  5. 生成日志零 `pe_transform' does not exist`；precheck 摘要显示覆盖探测结果；
     BRAN 阶段 `cata_time.get_world_transform` **< 2s**（基线 14.1s）。
  6. 零 `already exists` / `Cannot COMMIT`（继承 spec 005）。
- 行数/分布统计方式：日志 grep + aios-database CLI + SurrealDB count 查询（json 输出）。
