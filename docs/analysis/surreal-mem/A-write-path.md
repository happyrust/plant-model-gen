# A. 模型生成的 SurrealDB 写路径与写入瓶颈盘点

> 任务：盘点 `gen_all_geos_data` 模型生成期间往 SurrealDB 写了什么、怎么写、量级多大、哪些写可延后。
> 只读代码分析（未跑生成）；行号基于当前工作区源码。
> 入口链：`orchestrator.rs::gen_all_geos_data` → 四阶段流水线（sink / base_writer / mesh_stage / inst_aabb_writer）→ `SurrealModelWriterBackend`（`model_writer.rs`）→ `pdms_inst.rs` / `mesh_generate.rs` / `utils.rs`；旁路：BRAN_TUBI（`cata_model.rs`）、precheck（`pe_transform_refresh.rs`）、布尔（`manifold_bool.rs`）。

---

## 1. 写入表清单（按写入阶段分组）

### 1.1 主批次写（base_writer 阶段，`save_instance_data_with_report`, pdms_inst.rs:816）

| 表 | 语句形态 | 记录 id 形态 | 说明 |
|---|---|---|---|
| `inst_geo` | `INSERT IGNORE INTO inst_geo [..100 行..]` | `inst_geo:<geo_hash>`（u64 哈希） | 几何参数，按 geo_hash 全局去重 |
| `geo_relate` | `INSERT RELATION IGNORE INTO geo_relate [..100..]` | `geo_relate:[ref0,ref1,sesno,geo_index,inst_id_hash]` | inst_info→inst_geo 边，带 trans/pts/geo_type/visible |
| `inst_info` | `INSERT IGNORE INTO inst_info [..100..]` | 字符串 inst_key | 实例载荷（含完整 ptset JSON，行较肥） |
| `inst_relate` | `DELETE [ids]; INSERT RELATION INTO inst_relate [..100..]`（replace 对） | `inst_relate:[ref0,ref1,sesno]` | pe→inst_info 主桥；in 上有 UNIQUE 索引 |
| `neg_relate` | `INSERT RELATION IGNORE INTO neg_relate [..100..]` | 8 元数组 id（target+carrier+idx） | 负实体切割边；in/out UNIQUE 索引 |
| `ngmr_relate` | `INSERT RELATION IGNORE INTO ngmr_relate [..100..]` | 同上 | NGMR 交叉负实体边 |
| `aabb` | `INSERT IGNORE INTO aabb [..100..]` | `aabb:<hash>` | 共享值表（本阶段来自 tubi.aabb + 元素级解析 AABB） |
| `trans` | `INSERT IGNORE INTO trans [..100..]` | `trans:<hash>` | 共享 transform 值表（含 identity=0） |
| `vec3` | `INSERT IGNORE INTO vec3 [..100..]` | `vec3:<hash>` | 关键点值表（geo_param.key_points） |
| `refno_relations` | 仅 `replace_exist=true` 时写（pdms_inst.rs:1570）；流水线 `write_base_batch` 固定传 false（model_writer.rs:273-284），**主管线实际不写** | | |

注意：base 阶段 `write_inst_relate_aabb=false`，`inst_relate_aabb` 行被缓冲后丢弃，不落库；但 `aabb` 值表在 base 阶段就写。

### 1.2 mesh 结果回写（inst_aabb_writer 阶段，`persist_mesh_results` / `persist_inst_relate_aabb`, model_writer.rs:297-403）

| 表 | 语句形态 | 说明 |
|---|---|---|
| `vec3` / `aabb` | `INSERT IGNORE`（delta-only，spec 006 T301） | 每批只写本批 mesh 产生的增量，防 O(N²) 回归 |
| `inst_geo`（UPDATE） | 整批拼接成**一条**多语句 query：`update inst_geo:<h> set meshed=true, aabb=aabb:<h>, pts=[..];`（mesh_generate.rs:286-308） | 两步写：INSERT 后再 UPDATE meshed |
| `aabb` + `inst_relate_aabb` | `INSERT IGNORE INTO aabb`；`DELETE [ids]; INSERT INTO inst_relate_aabb [..100..]`（pdms_inst.rs:1856-1911） | refno→aabb 投影，替换式写；房间计算/SQLite 空间索引的数据源 |
| 收尾 `final_sweep`（`finalize_mesh_entities`, model_writer.rs:406-426） | 全量 `INSERT IGNORE` 整个进程级 aabb/pts map 一次 | spec 006 T302 安全网，幂等 |

### 1.3 BRAN/HANG 管段旁路（不走 ModelWriter，`cata_model.rs` 直写）

| 表 | 语句形态 | 说明 |
|---|---|---|
| `tubi_relate`（先删） | 每 BRAN 一条 `LET $ids = SELECT VALUE id FROM tubi_relate:[..range..]; DELETE $ids;`（pdms_inst.rs:265-285） | 写前按 branch range 清旧直段 |
| `trans`/`aabb`/`vec3` | `save_transforms_to_surreal`/`save_aabb_to_surreal`/`save_pts_to_surreal`（utils.rs），**必须先于 RELATE**（cata_model.rs:6438-6460 注释：RELATE 先执行会隐式建 d=NONE 空记录） | chunk 100/300/100 |
| `tubi_relate` | 所有 `relate pe->tubi_relate:[branch,idx]->pe set geo=..,aabb=..,world_trans=..,dt=fn::ses_date(..)` 语句 `join("")` 后**一条 query 全量提交**（cata_model.rs:6462-6492） | 单点大 SQL；语句级错误显式 check |
| `tubi_info` | `INSERT IGNORE INTO tubi_info [..200..]`（pdms_inst.rs:2163-2190），BRAN 阶段 5 串行调用（index_tree_mode.rs:996） | 幂等，不预查 |

### 1.4 布尔阶段（`run_bool_worker_from_tasks` / `run_boolean_worker`, manifold_bool.rs）

| 表 | 语句形态 | 说明 |
|---|---|---|
| `inst_geo`（新建 booled 记录） | `create inst_geo:<mesh_id> set meshed=true, aabb=..` | booled mesh 的 GLB 落盘 + DB 登记 |
| `geo_relate` | `DELETE <relation_id>; INSERT RELATION ...`（geo_type='CatePos'） + `UPDATE` 原始边为 Compound/visible=false | 每 cata 任务一条拼接 update_sql，`apply_cata_update_sql` 单 query 提交 |
| `inst_relate_cata_bool` | `LET $inst_info=..; DELETE <id>; INSERT RELATION ...`（utils.rs:135-166） | 每 refno 一条独立 query（成功和失败都写状态） |
| `inst_relate_bool` | `UPSERT <id> CONTENT {..}`（utils.rs:108-132） | 实例级布尔状态，每 refno 一条 |
| `aabb` + `inst_relate_booled_aabb` | `UPSERT`（utils.rs:169-234，chunk 200） | booled 后包围盒 |

### 1.5 入口前置写（生成开始前）

| 写 | 位置 | 说明 |
|---|---|---|
| schema/表初始化 | `init_model_tables()`（aios_core，orchestrator.rs:923）；`DEFINE TABLE IF NOT EXISTS ses`（utils.rs:34-36） | DDL，一次性 |
| `pe_transform` | precheck 未覆盖时全量刷新：BFS 遍历 pe 树，**每 100 条 flush** `save_pe_transform_entries`（pe_transform_refresh.rs:16,171,364-402） | 空库首跑最重的前置写；读侧逐节点查 local mat（30s 超时/节点） |
| 增量清理 `pre_cleanup_for_regen` | 点删 `inst_relate/inst_relate_aabb/inst_relate_bool/inst_relate_cata_bool/refno_relations` + range 删 `neg_relate/ngmr_relate/geo_relate/tubi_relate` + 按 hash 删 `inst_geo`（pdms_inst.rs:287-305,618-761） | chunk 200；并发 WS=4 / 嵌入式=16 |

### 1.6 不写 SurrealDB 的旁路输出

- mesh GLB/manifold 文件写磁盘（`assets/meshes/lod_*` / `manifold/`），不进 DB。
- `failed_sql` 诊断转储：写死限 20 个 `.surql` 文件（pdms_inst.rs:38-119）。
- Parquet 流式写默认关闭（`AIOS_ENABLE_PARQUET_STREAM_WRITER`，orchestrator.rs:1218）。
- `DrainOnly` 后端只统计不写（压测基线）。

---

## 2. 批量 / 事务 / 并发模式

### 2.1 流水线拓扑（orchestrator.rs:1311-1356）

```
生产端(LOOP/CATE/PRIM/BRAN 分页生成, 页=index_tree_batch_size 默认 100 refno)
  → flume::bounded(batch_channel_capacity=100) ShapeInstancesData
  → run_batch_sink（单任务：编号、收集 touched_refnos、累积 BooleanTask，复制发两路）
     ├→ base_writer_sender  → flume::bounded(100) → run_base_writer   worker×base_write_concurrency(默认8) + Semaphore(8)
     └→ mesh_stage_sender   → flume::bounded(100) → run_mesh_stage    worker×mesh_compute_concurrency(默认4) + Semaphore(4)
                                   ↓ BatchMeshOutput flume::bounded(100)
run_inst_aabb_writer：BatchStageJoiner 按 batch_id 汇合 (base 完成信号 × mesh 输出)
  → joined flume::unbounded → worker×inst_aabb_write_concurrency(默认2) + Semaphore(2)
  → completion flume::unbounded → 主线程屏障统计
```

- 每个 batch 被写两次通道（base + mesh 并行），`inst_aabb` 阶段必须等**同一 batch 的 base 写完 + mesh 算完**才执行（Joiner，orchestrator.rs:322-380）。
- batch barrier 后串行执行：`final_sweep` → `reconcile_missing_neg_relations` → boolean bridge → web bundle 导出 → SQLite 空间索引刷新。

### 2.2 TransactionBatcher（pdms_inst.rs:1913-2150，核心写原语）

- 常量：`CHUNK_SIZE=100`（单条 INSERT 拼 100 行）、`MAX_TX_STATEMENTS=4`（一个事务块最多 4 条语句 ≈ 400 行）、`MAX_CONCURRENT_TX=2`（每个 batcher 最多 2 个在飞事务）。
- 事务块 `BEGIN TRANSACTION; ...; COMMIT TRANSACTION;`，逐 statement `take()` 显式检查语句级错误（防静默吞错）。
- 重试：`Transaction conflict / Resource busy` 指数退避 50ms→2s，最多 8 次；`inst_relate_aabb`/`neg_relate` 唯一索引脏冲突各有一次「重建索引再重试」的自愈路径（pdms_inst.rs:2040-2060）。
- 超限失败 → `.surql` 转储（每次运行最多 20 个）。
- **每张表一个独立 batcher，且在 `save_instance_data_with_report` 内串行 finish**：geo(inst_geo+geo_relate) → neg → ngmr → inst_relate/inst_info（交错 push，先后 finish）→ aabb → (inst_relate_aabb) → trans → vec3。单 batch 内是「表间串行、表内 ≤2 并发事务」。

### 2.3 并发参数与串行点汇总

| 参数 | 默认 | 位置 |
|---|---|---|
| `index_tree_batch_size` | 100（下限钳制 100） | options.rs:460-467 |
| `batch_channel_capacity` | 100 | options.rs:63-65 |
| `base_write_concurrency` | 8 | options.rs:67-69 |
| `mesh_compute_concurrency` | 4 | options.rs:71-73 |
| `inst_aabb_write_concurrency` | **2** | options.rs:75-77 |
| batcher 内并发事务 | 2；单事务 ≤4 语句、单语句 ≤100 行 | pdms_inst.rs:833-838 |
| 布尔 worker 并发 | `num_cpus.clamp(2,12)`，可被 `AIOS_BOOL_WORKER_CONCURRENCY` 覆盖 | manifold_bool.rs:1897-1916 |
| pre_cleanup 并发 | WS=4 / 嵌入式=16，chunk 200 | pdms_inst.rs:645-651 |
| pe_transform 刷新 | 单线程 BFS，100 条/flush | pe_transform_refresh.rs:16 |

**显式串行点（潜在瓶颈）**：

1. `run_batch_sink` 单任务复制两路 + 累积 bool task —— 通道满时产生背压（有 `send_wait_ms` 日志可观测）。
2. `inst_aabb` 阶段并发只有 2，且要等 base+mesh 汇合，是尾部收敛点（`[batch_stage] stage=join waiting=...` 日志暴露谁在等谁）。
3. BRAN_TUBI 的 `tubi_relates.join("")` 一次性提交（cata_model.rs:6462）：一个 BRAN 页的全部直段 RELATE 拼一条 query，无 chunk、无事务分片；语句里还内嵌 `fn::ses_date()` 函数求值。
4. `persist_mesh_results` 的 `update_sql` 同样是整批拼一条 query（model_writer.rs:343-366）。
5. `InstRelatePrecomputed::build`：每 batch 写前先同步批量**读** pe(dbnum,sesno) + ses(date)（chunk 500，pdms_inst.rs:2612-2691）——写路径上的读放大。
6. `save_tubi_info_batch` 在 BRAN 阶段 5 串行 await（index_tree_mode.rs:990-1002）。
7. barrier 后 `reconcile_missing_neg_relations`、boolean bridge、SQLite 索引刷新全部串行。
8. 布尔阶段对每个 refno 独立发 `inst_relate_cata_bool`/`inst_relate_bool` 小 query（无批量聚合），任务多时是高频小写。

### 2.4 幂等语义

- 值表（aabb/trans/vec3/inst_geo/inst_info/tubi_info）：`INSERT IGNORE`，内容寻址（hash id），天然幂等，重复写只浪费带宽。
- 边表 inst_relate / inst_relate_aabb：**替换式**（`DELETE [ids]` + `INSERT`，同事务），保证同 refno 只留最新。
- neg/ngmr/geo_relate/tubi_relate：确定性数组 id + `INSERT RELATION IGNORE`（tubi 为先 range 删再 RELATE）。
- 旧数据清理统一收敛到入口 `pre_cleanup_for_regen`（增量/regen），写路径本身不再逐批 DELETE 扫描（pdms_inst.rs:878-881 注释）。

---

## 3. 各阶段写入量级与耗时特征

### 3.1 量级模型（行数与元素数的关系）

以「本次生成触达的元素数 E、几何实例数 G（每元素 1~n 个）、唯一几何 hash U（跨元素强去重）、管段数 T」估算：

| 表 | 行数量级 | 备注 |
|---|---|---|
| `inst_info` / `inst_relate` | ≈ E 各一行 | inst_info 行含 ptset JSON，单行最肥 |
| `geo_relate` | ≈ G（同 inst_key 多载体时 ×载体数） | 通常是**行数最大**的边表 |
| `inst_geo` | ≈ U（deduper 预载已 mesh hash 后进一步降） | 复用型元件库场景 U ≪ G |
| `aabb` / `trans` / `vec3` | ≈ 唯一 hash 数 | vec3 = 每几何 key_points 数×去重 |
| `inst_relate_aabb` | ≈ E（能解析出 AABB 的） | 替换式，每批 delta + 收尾不重写 |
| `tubi_relate` / `tubi_info` | ≈ T / ≈ 管件组合数 | 仅 BRAN/HANG |
| `neg_relate` / `ngmr_relate` | 远小于 G | 有负实体的载体才有 |
| `pe_transform` | ≈ dbnum 全树节点数（仅未覆盖时） | 一次性，量最大可到几十万节点 |

每 batch（100 refno 页）在 base 阶段的写请求数上界大致为：`Σ各表 ceil(rows/100) 条 INSERT 语句 / 4 per 事务`，即**十几到几十个事务 RPC**，外加 precompute 的 2 组批量读。

### 3.2 耗时特征（代码内建观测点）

- 每 batch 打印 `[batch_perf] batch=N base_wait_ms/base_write_ms/mesh_wait_ms/mesh_ms/inst_aabb_wait_ms/inst_aabb_ms/total_ms`（orchestrator.rs:672-686）——判定瓶颈在写库、mesh 计算还是 aabb 收敛的第一手数据。
- 通道背压：`stage=sink target=base_writer send_wait_ms>0` 说明写库消费不动；`stage=join waiting=base_result` 说明 base 写是尾巴。
- 阶段耗时：PerfTimer 分段 + `output/<project>/profile/perf_gen_model_index_tree_*.json/csv`（orchestrator.rs:1814-1872）。
- 失败观测：`failed_sql` 转储目录 + `cache_miss_report.json`。
- 历史病灶（已修，作为回归警戒线）：
  - mesh_persist 每批全量重写全局 aabb/pts map → O(N²)，基线每批固定 ~5.5s（model_writer.rs:320-322 注释，spec 006 T301 改 delta）。
  - 空库首批 tubi_relate 因 `ses` 表不存在整条 RELATE panic（orchestrator.rs:840-848，已用 ensure_surreal_init 兜底）。
  - 高并发大事务触发 session 丢失 / `Transaction conflict: Resource busy`（因此 MAX_TX=4、并发 2 的保守取值，pdms_inst.rs:832-838）。
  - WS 连接下 clone db handle 丢 ns/db 选择导致整块事务回滚（pdms_inst.rs:2004-2009 注释）。
- 定性排序（依据结构与注释）：**全量首跑** pe_transform 刷新与 mesh CSG 是大头，写库瓶颈集中在 base 阶段 inst_info/geo_relate 大 JSON 行与 inst_relate 的 DELETE+INSERT 对；**增量**时 pre_cleanup（range 删 + 图遍历）与布尔阶段高频小写占比上升；**管道密集库** tubi_relate 单条大 query 是显著长尾。

---

## 4. 哪些写必须同步、哪些可延后

### 4.1 必须同步（有序依赖 / 后续阶段消费）

1. **值表先于边表**：`trans/aabb/vec3` 必须先于引用它们的 `geo_relate`/`tubi_relate` RELATE 落库，否则 SurrealDB 隐式创建 `d=NONE` 空记录且后续 `INSERT IGNORE` 补不进去（cata_model.rs:6438-6442、pdms_inst.rs:1389-1391 明确注释）。同理 `aabb` 先于 `inst_relate_aabb`。
2. **base 批写先于同批 inst_aabb 阶段与布尔**：布尔 backfill/worker 从 `geo_relate`/`inst_relate.has_cata_neg` 反查任务；`reconcile_missing_neg_relate` 依赖 geo_relate 已在库。流水线用 Joiner + barrier 硬保证。
3. **pre_cleanup 先于一切写**：否则 INSERT IGNORE 撞旧行导致"代码已改库里仍旧值"假象（pdms_inst.rs:358-361）。
4. **pe_transform 覆盖先于生成**（precheck）：否则生成期回退 DB 逐条查 transform，CATE 阶段基线劣化 14s+（precheck_coordinator.rs:201-203）。
5. `inst_relate` 的 DELETE+INSERT 必须同事务成对（替换语义，防半态）。

### 4.2 可延后 / 可异步落盘（幂等，且生成主链路不回读）

1. **`inst_relate_aabb`**：已有 `AIOS_SKIP_INST_RELATE_AABB` 全跳开关（orchestrator.rs:716）；消费方是房间计算/SQLite 空间索引/导出，均在 barrier 之后——可整体挪到收尾一次性写。
2. **mesh 结果回写 `UPDATE inst_geo SET meshed=...`**：defer 模式已证明可与 INSERT 合并为单次写（`to_insert_fields`，pdms_inst.rs:2895-2903），消除两步写；纯状态位，晚到只影响下次去重预载。
3. **`final_sweep` 全量 aabb/pts**：本身就是收尾兜底，`INSERT IGNORE` 幂等，可跳过（`AIOS_SKIP_FINAL_AABB_SWEEP`）或后台执行。
4. **`tubi_info`**：`INSERT IGNORE` 幂等、无表内依赖，仅导出链路消费，可后置/并行。
5. **布尔状态表** `inst_relate_bool`/`inst_relate_cata_bool`/`inst_relate_booled_aabb`：worker 去重与导出用，UPSERT 幂等；`SqlBoolWriter` 已支持整段写 `.surql` 延后导入。
6. **`refno_relations`**：主管线本来就不写（仅 replace_exist 兜底路径）。
7. **整体零写模式**：`save_instance_data_to_sql_file` + mesh worker `sql_writer` + `SqlBoolWriter` 组成完整「SQL 落文件、事后 `import_sql_file` 批量导入」通道（pdms_inst.rs:2746、mesh_generate.rs:1232-1249）——证明除 §4.1 的顺序约束外，**全部模型写都可延后**，代价是延后期间 DB 不可查最新模型。
8. 诊断类（failed_sql 转储、cache_miss_report、perf json）：纯文件写，天然异步。

### 4.3 优化线索（供后续内存/写优化任务引用）

- `inst_aabb_write_concurrency=2` 与批内「表间串行 batcher」是 base_write_ms 之外最先该动的旋钮。
- tubi_relate / mesh update_sql 的"单条巨型 query"应按 CHUNK 分片并进 TransactionBatcher 同一原语。
- 布尔阶段逐 refno 小 query 可聚合成批（与 SqlBoolWriter 的批量形态对齐）。
- `InstRelatePrecomputed` 的 per-batch pe/ses 批量读可上移为 run 级缓存（同一 refno 不会跨 batch 重复出现，但 ses 表读可全局一次）。
