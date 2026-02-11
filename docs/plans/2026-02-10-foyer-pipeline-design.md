# SurrealDB -> Foyer Cache -> Model -> Mesh 流水线重构设计（方案B，Key 驱动）

日期：2026-02-10

## 0. 摘要
本设计将现有“逐 refno 串行查询 + 即时计算”的模型生成流程，改造为“生产者批量预取并写入 Foyer Cache -> 发送批次 Key -> 消费者按 Key 只读 Cache 生成模型/实例 -> Mesh Worker 只读 Cache 生成网格”的流水线。

核心要点：
- 以 **Foyer Cache 作为中枢** 保存可复用的中间数据；阶段间 **只传 Key，不传 payload**。
- **SurrealDB 查询与写入 Foyer Cache 并行**（跨 batch 重叠 + 同 batch 多查询并行 + 写入 spawn_blocking）。
- **CATE 必须缓存 `resolve_desi_comp` 产物**，并以 `cata_hash` 为缓存粒度（同组 design_refno 共享）。Generator 专注从 Foyer 取数据，避免在生成阶段直查 SurrealDB。
- **所有 foyer cache payload 统一迁移为 rkyv**；旧的 JSON payload 读到即视为 miss（方案1：不做兼容解码），由上游重建并 writeback。

## 1. 背景与问题
参考既有方案文档 `C:\Users\Administrator\.windsurf\plans\pipeline-refactor-877355.md`，当前瓶颈主要为：
- 网络往返：每 refno 多次独立 SurrealDB 查询，累积为海量 RTT。
- IO/CPU 耦合：查询与计算交替执行，导致 CPU 等 IO、IO 等 CPU。
- 预取收益不足：已有 `geom_input_cache`（LOOP/PRIM）但预取仍以逐 refno await 为主，难以批量与并行。

仓库现状（用于对齐改造点）：
- 已存在 LOOP/PRIM 输入缓存：`src/fast_model/foyer_cache/geom_input_cache.rs`
- orchestrator 已支持 mesh 从 cache 路径执行：`src/fast_model/gen_model/orchestrator.rs`
- 已存在批量 SurrealQL 的 SQL 试验片段：`history.txt`

## 2. 目标与非目标
### 2.1 目标
- G1：将“查询 -> 写 cache”与“模型生成计算”解耦并并行，形成稳定流水。
- G2：中间数据落 Foyer Cache，可复用、可断点续跑、可调试复现。
- G3：模型生成尽量 cache-only（阶段性允许 fallback），mesh 生成尽量 cache-only。
- G4：可观测：cache hit/miss、fallback 次数、writeback 条数、每 dbnum/batch 耗时。

### 2.2 非目标（当前阶段不做）
- N1：不追求 per-refno 细粒度事件流（避免调度复杂度）。
- N2：不强行一次到位把所有查询都改为批量 SQL；先跑通流水线，再分项替换。
- N3：不提供旧 JSON cache 的向后兼容解码（读到即 miss），以减少迁移期复杂度。

## 3. 总体架构（方案B：Key 驱动 + Foyer 中枢）

### 3.1 组件
- Prefetcher（IO 密集）：
  - 批量查询 SurrealDB（同 batch 多语句/多查询并行）
  - 组装 Batch payload（LOOP/PRIM：GeomInputBatch；CATE：CateQueryBatch）
  - 写入 Foyer Cache（后台序列化/写盘）
  - 写入完成后发送 `ReadyBatchKey`

- Model Generator（CPU/内存密集）：
  - 接收 `ReadyBatchKey`
  - 从 Foyer Cache 读取对应 batch
  - 调用既有 from_cache 入口生成模型/实例数据

- Mesh Worker（主要 CPU/IO）：
  - 优先从 cache 路径运行（`crate::fast_model::foyer_cache::mesh::run_mesh_worker`）
  - 一期按 dbnum 完成触发（减少总墙钟时间）

### 3.2 数据流（简图）

```
SurrealDB
  |  (batch queries)
  v
Prefetcher -------------------(spawn_blocking serde + write)---->
  |                                                          Foyer Cache
  | (after write ok)                                              |
  v                                                              |
ReadyBatchKey (flume::bounded)                                   |
  |                                                              |
  v                                                              v
Model Generator (cache-only read) ---> Instance/Geo cache ---> Mesh Worker (cache-only read)
```

## 4. 缓存与 Key 协议设计

### 4.1 Key：ReadyBatchKey
建议新增模块：`src/fast_model/gen_model/pipeline_keys.rs`

```rust
pub enum BatchKind {
    LoopPrimInput,
    CateQueryInput,
}

pub struct ReadyBatchKey {
    pub dbnum: u32,
    pub kind: BatchKind,
    pub batch_id: String,
}
```

原则：
- 通道里只传 key；payload 必须落 cache。
- key 仅在“写 cache 成功”后发送，避免消费者读空。

### 4.2 LOOP/PRIM：沿用 GeomInputBatch
现有结构位于 `src/fast_model/foyer_cache/geom_input_cache.rs`：
- `GeomInputBatch { dbnum, batch_id, created_at, loop_inputs, prim_inputs }`
- index 文件：`geom_input_cache_index.json`

改造点：
- 新增异步写入 `insert_batch_async`：
  - `serde_json::to_vec` 放到 `spawn_blocking`
  - index 更新可批量/异步
- 消费侧提供逐 batch 读取并处理（避免一次性 get_all_* 全量扫描）。

### 4.3 CATE：新增 CateQueryBatch（选项1）
新增：`src/fast_model/foyer_cache/cate_query_cache.rs`

内容范围（仅“查询中间数据”）：
- per-refno：attmap、world_transform、owner_refno/owner_type、generic_type
- CATE 解析所需的标识：cata_hash、cat_refno（或可推导字段）
- pos/neg 映射（若可预取）
- 其它轻量字段：visible、noun/type 等

不包含：
- `resolve_desi_comp` 的产物（该产物将由单独的 `cata_resolve_cache` 管理，按 `cata_hash` 缓存）。

### 4.4 CATE：新增 CataResolveCache（按 cata_hash 缓存 resolve 产物）
新增：`src/fast_model/foyer_cache/cata_resolve_cache.rs`

内容范围（`resolve_desi_comp` 的可复用产物，面向后续“实例生成/mesh 生成”）：
- `ptset_map`（`axis_items`）：`BTreeMap<i32, CateAxisParam>` 等价结构
- `prepared_shapes`：将 `resolve_desi_comp -> CateCsgShape -> EleInstGeo` 的“单位复用/scale 归一”逻辑固化为可缓存的 shape 列表：
  - `geo_hash`、`unit_flag`、`geo_param`（unit 参数）、`local_transform`（t/r/s 数组）、`visible/is_tubi/is_ngmr`、`pts`
- `has_solid`：该 `cata_hash` 是否包含 Pos 实体（用于 `EleGeosInfo.is_solid`）

Key 设计：
- key = `(dbnum, cata_hash)`
- 说明：同一 `cata_hash` 可能对应多个 design_refno；只需选择一个“代表 refno”执行 `resolve_desi_comp`，其产物即可复用到同组其余元素。

### 4.5 CATE fallback + writeback（M3a 决策）
M3a 允许：
- cache miss 时回查 SurrealDB（prefetch 阶段）
- 默认 writeback：将回查得到的 `cate_query_cache` + `cata_resolve_cache` 写回 Foyer

建议开关（命名可按项目习惯再对齐）：
- `AIOS_CATE_QUERY_CACHE=1`
- `AIOS_CATE_QUERY_CACHE_WRITEBACK=1`（默认开）
- `AIOS_CATE_QUERY_CACHE_ONLY=1`（M3b 再默认开）
 - `AIOS_CACHE_CODEC=rkyv`（默认 rkyv；读到非 rkyv payload 视为 miss）

## 5. 并行与限流设计

### 5.1 同 batch 并行（多查询并发）
对一个 batch 的查询拆分为有限条“可批量”的 SurrealQL，使用 `tokio::try_join!` 并发。

### 5.2 跨 batch 并行（写入与下一批查询重叠）
- Prefetcher 查完 batch#1 后，立即启动后台写入任务（JoinSet 管理）。
- 同时开始 batch#2 查询。

### 5.3 Backpressure
- `flume::bounded(N)`：限制已写入但未消费的 key 数量。
- `Semaphore`：限制同时在飞的“写入任务”和“DB 查询任务”并发，避免压垮 SurrealDB 或磁盘。

## 6. SurrealDB 批量查询策略（渐进替换）

### 6.1 先跑通，再优化
M1 阶段允许仍复用现有逐 refno 查询（确保行为不变），重点是流水线与 cache 中枢。

M2 开始按项替换为批量 SurrealQL：
- 优先使用 `FROM [pe:⟨...⟩, ...]` 或 `LET $ids = [...]` 的方式（见 `history.txt` 的验证片段）。
- 多语句用 `SUL_DB.query_response` 合并一次 RTT。

### 6.2 注意事项
- tubi_relate 必须使用 ID Range（项目已有样例：`rs-core/src/rs_surreal/inst.rs`）避免全表扫描。

## 7. 分阶段实施（里程碑）

### M1：LOOP/PRIM Key 流水线 + 并行写 cache
- 写 cache 后发 key
- Generator 按 key 读 cache 生成
- 加入 JoinSet 收敛写入任务，确保退出前落盘完成

### M2：LOOP/PRIM 批量 SurrealQL
- attmap/transform/loops+height/neg 等逐项替换为批量
- 引入 query_response 合并多语句

### M3a：CATE 查询中间数据缓存（允许 fallback + 默认 writeback）
- 新 CateQueryCacheManager
- miss 回查并回写，统计 hit/miss/fallback

### M4：模型 -> mesh 按 dbnum 流水触发
- dbnum 完成即触发 mesh worker（cache-only 路径优先）

### M3b（可选后续）：CATE 严格 cache-only（resolve 产物 + query 数据均必须命中）
- 彻底关闭 DB fallback（`*_CACHE_ONLY=1`）
- 生成阶段不再允许调用 `resolve_desi_comp`

## 8. Task List（按模块拆分）

1) 新增 Key 协议与 runner
- `src/fast_model/gen_model/pipeline_keys.rs`
- `src/fast_model/gen_model/pipeline_runner.rs`

2) 改造 LOOP/PRIM cache
- `src/fast_model/foyer_cache/geom_input_cache.rs`：insert_batch_async + 逐 batch 消费入口

3) 新增 CATE query cache（M3a）
- `src/fast_model/foyer_cache/cate_query_cache.rs`
- CATE processor：prefetch + 读取 + miss fallback + writeback

4) 批量查询逐项替换（M2）
- 参考 `history.txt` 的 SQL 形态
- 能合并的尽量用 `query_response`

5) Mesh 流水（M4）
- `src/fast_model/gen_model/orchestrator.rs`：按 dbnum 完成触发 cache mesh worker

## 9. 可观测与验收

### 9.1 指标
- loop/prim/cate：cache hit/miss
- cate：fallback 次数、writeback 条数
- 每 dbnum / 每 batch：查询耗时、写入耗时、生成耗时

### 9.2 验收
- 正确性：同一 refnos，允许 fallback 时生成结果一致（数量、关键 hash/统计口径一致）。
- 收敛性：重复运行同一数据集，CATE fallback 次数下降（因 writeback）。
- 可控性：切 `*_CACHE_ONLY=1` 能明确暴露缺失（可诊断），而非静默吞错。

## 10. 风险与回滚
- 缓存版本漂移：建议 batch payload 增加 `schema_version`（或 batch_id 前缀），不匹配则跳过并重建。
- 后台写入丢数据：必须 await JoinSet/flush。
- 并发压垮 SurrealDB：Semaphore 限流，必要时按 dbnum 限流。
- 回滚：保留旧路径开关（禁用 pipeline 或禁用 cache-only），快速切回稳定模式。

## 11. 已确认决策（冻结）
- CATE：必须缓存 `resolve_desi_comp` 产物，按 `cata_hash` 粒度保存到 `cata_resolve_cache`。
- 所有 foyer cache payload：统一使用 rkyv；读到旧 JSON payload 一律视为 miss（方案1），由上游重建并 writeback。
- M3a：允许 DB fallback（仅在 prefetch 阶段）。
- M3a：默认 writeback（fallback 后写回 Foyer Cache）。

