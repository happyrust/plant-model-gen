# B — 模型生成的 SurrealDB 读路径与缓存层盘点

> 任务：盘点生成期间哪些查询打到 SurrealDB、哪些走本地缓存/文件索引，评估「内存模式」对读的收益。
> 范围：`gen_all_geos_data`（orchestrator.rs）驱动的 IndexTree 生成管线（Full / Manual / Debug / Incremental 四种 scope 共用）。
> 说明：本文所称 SurrealDB 均指 `SUL_DB`（`Surreal<Any>`，rs-core `rs_surreal/mod.rs`）；连接模式由 `DbOption.surrealdb.mode` 决定 —— `File`（进程内嵌入式 `rocksdb://`，默认）或 `Ws`（远程 `ws://ip:port`）。模型表与 PE/ATT 固定同库（`model_primary_db() == SUL_DB`）。

## 1. 读路径清单（按管线阶段）

### 1.1 层级/结构查询 —— 全部走本地 TreeIndex 文件，不打 DB

入口 `get_model_query_provider()`（query_provider.rs）返回 `TreeIndexQueryProvider`：**层级查询走 `<output>/<project>/scene_tree/{dbnum}.tree`，PE/属性委托 SurrealDB**。

| 查询 | 实现 | 数据源 |
|---|---|---|
| roots 枚举 / noun 计数（`query_by_noun_all_db`、`count_noun_all_db`、`prequery_noun_counts`） | 遍历 TreeIndex `all_refnos()` 按 noun hash 过滤 | `.tree` 文件（进程内缓存） |
| 子孙/children/ancestors（query_compat.rs 全部函数：`query_visible_geo_descendants`、`query_negative_geo_descendants`、`query_deep_visible_inst_refnos`、`query_filter_deep_children`、`collect_children_filter_ids` 等） | TreeIndex BFS | `.tree` 文件 |
| BRAN 子管件元素收集（`collect_children_elements_from_tree`、`collect_bran_cate_descendant_elements_from_tree`） | TreeIndex，构造最小 `SPdmsElement`（不含 name/status） | `.tree` 文件 |
| cata_hash 分组（`build_cata_hash_map_from_tree`，cate_processor 入口） | TreeIndex node_meta.cata_hash | `.tree` 文件；**tree 加载失败才逐 refno 回退 `get_named_attmap`（DB）** |
| refno→dbnum（`resolve_dbnum_for_refno`、db_meta_cache） | `db_meta_info.json` 内存映射 | 本地 JSON，**明确禁止回退 DB** |

TreeIndex 有全局进程内缓存（`TREE_INDEX_CACHE: DashMap<(dir,dbnum), Arc<TreeIndex>>`），每 dbnum 只做一次磁盘反序列化（64MB 大栈线程）。**结论：结构遍历这一大类读在生成期间零 DB 流量，内存模式对其无收益。**

### 1.2 属性读取 —— 生成期间最大的 SurrealDB 读流量

核心原语 `aios_core::get_named_attmap(refno)`（rs-core query.rs:1129）：

```sql
(select * from pe:<key>.refno)[0];
```

**每 refno 一次点查、无任何进程内缓存**。而所谓的批量接口 `get_attmaps_batch` / `get_pes_batch`（surreal_provider.rs:228/241）内部是 **for 循环逐个点查**，只是名字上的 batch。

调用密度（按元素）：
- LOOP 页（loop_model.rs）：每页 `get_named_attmap` future 批（buffered）+ 每 target 1 次 attmap + 负实体子孙（tree）。
- PRIM 页（prim_model.rs）：origin/tmpl/design_owner/child/ddat/顶点（SPVE 等）多次 attmap，POLYHE 类顶点逐个 attmap（`query_multi_descendants_with_self` 走 tree，但每个顶点 refno 再点查 attmap）。
- CATE 页（cata_model.rs，重灾区，grep 到 20+ 处计时点）：每元素 `get_named_attmap(ele_refno)`（有的分支 2 次）、`get_cat_refno`（SPRE→CATR 图查询，LRU 缓存）、逐 cata_hash 一次 `resolve_desi_comp`。
- BRAN/TUBI 预取（`prefetch_tubi_size_and_branch_meta`）：每 branch `get_named_attmap(branch_refno)` + `get_named_attmap(h_ref)` + `query_single_by_paths(->LSTU->CATR)` + `query_tubi_size`（快路径 1 次 SCOM attmap；慢路径整套 resolve_desi_comp）。

### 1.3 CATA 元件库解析 —— DB 读 + 两层进程内缓存

`resolve_desi_comp`（resolve.rs）每次未命中缓存时的 DB 读序列：

1. `get_named_attmap(desi_refno)`（可由调用方传入复用）
2. `get_cat_refno`（`query_single_by_paths(->SPRE / ->SPRE->CATR)`，**LRU cached(size=10000)**）
3. `normalize_catalog_scom_ref`：沿 CATR 链最多 4 次 attmap
4. `get_or_create_scom_info(scom_ref)`：**SCOM_INFO_MAP（DashMap，全局，进程生命周期）** miss 时 → SCOM attmap + `query_axis_params`（PTRE 的 `get_children_named_attmaps`）+ `query_single_by_paths(->GMRE/->GSTR)` + `query_gm_params`（`collect_descendant_full_attrs` 1..2 层一次图查询）+ NGMR 同套 + PSTR 子属性
5. `get_or_create_cata_context(desi_refno)`：又一次 DESI attmap + DESP/UNIPAR 解析（每元素，无缓存）
6. `query_iparam_from_desi`：`fn::get_ipara(...)` DB 函数，**仅 with_insulation=true 才发**（默认关，填 0）

缓存命中范围：
- `SCOM_INFO_MAP`：按 SCOM refno 全局有效（注释：6502 次调用 → ~10 次真实 DB 查询）。
- `cata_resolve_cache`（model_cache/cata_resolve_cache.rs）：按 cata_hash 缓存 `resolve_desi_comp` 最终产物（ptset+几何），**进程内 DashMap，跨多轮 gen_cata_geos 复用**；同 hash 组内只有首元素付出解析代价，其余元素只剩 attmap+transform 两个点查。
- 部分解析模式（`AIOS_CATA_CLOSURE_MODE=manifest`）下 attmap miss 会触发惰性小闭包解析回填再重试（try_lazy_cata_fallback）。

### 1.4 世界/局部变换 —— 三级缓存，miss 才打 DB

transform_cache.rs + transform_rkyv_cache.rs：

1. **内存层**：`GLOBAL_TRANSFORM_CACHE`（DashMap，(dbnum,refno)→Transform，带 pin/租约）。
2. **文件层**：`<model_cache>/transforms/transform_cache_db_{dbnum}.rkyv`，按 dbnum 整库快照，`source_version`（latest_sesno+file+ref0s）失配即失效重建。
3. **DB 层（miss 回源）**：`query_world_transforms_from_pe_transform` —— 按 200/chunk 批量 `SELECT record::id(id), world_trans.d FROM [pe_transform:...]`；仍 miss 的兜底 `aios_core::get_world_transform`（惰性计算：查 pe_transform 缓存 → `get_local_mat4`（2 次 attmap）→ 向上找最近有缓存的祖先逐层累乘 —— **每个 miss 是一串 attmap 点查**，最贵的读路径）。

两种模式：`get_world_transforms_cache_first_batch`（miss 回源 DB+计算）与 `*_cache_only_batch`（strict，miss 直接报错，离线 Generate 用）。rkyv 构建本身也是读大户（fetch_inst_relate_refnos 全表 `SELECT record::id(id) FROM pe_transform WHERE world_trans != none` + 全量 world 批查 + 逐 refno local 计算，失败有负缓存防重试风暴）。precheck 阶段（precheck_coordinator + pe_transform_refresh）会先刷新 pe_transform，orchestrator 还会用 `prime_global_transform_cache_from_pe_entries` 把刚刷出的 entries 直接灌进内存缓存，避免同进程再回源。

### 1.5 Mesh / 布尔 / 收尾阶段的 DB 读

| 读点 | SQL 形态 | 频次 |
|---|---|---|
| mesh 去重预载 `query_existing_meshed_inst_geo_ids` | **不打 DB**：EXIST_MESH_GEO_HASHES（本地 aabb 缓存文件）∩ 磁盘 glb 存在性 | 启动一次 |
| mesh 写前 existence（mesh_generate 内 inst_geo 点查/批查） | `SELECT ... FROM inst_geo:...` | 每 batch |
| 布尔 DB 回填 `query_cata_backfill_candidates` | `SELECT VALUE in.id FROM inst_relate WHERE has_cata_neg = true`（**全表谓词扫描**） | 布尔阶段一次（enable_db_backfill 时） |
| `fetch_cata_bool_tasks_from_db` | 每 50 refno：`[pe:...]->inst_relate WHERE has_cata_neg` + 每 inst_info 一次 `->geo_relate` | 回填候选数/50 + N |
| barrier 后 `reconcile_missing_neg_relations`、`filter_missing_inst_aabb` | `SELECT VALUE refno FROM inst_relate_aabb WHERE refno IN [...]` | 收尾一次/批 |
| 导出/instances（inst_query.rs `query_insts_with_batch`） | inst_relate / inst_relate_bool / geo_relate + pe 计算字段（world_trans/world_aabb） | 导出阶段按 refno 批 |

### 1.6 明确已移除/桩化的缓存层（不要再指望它们）

- foyer 缓存整体移除：`model_cache/mod.rs`、`geom_input_cache.rs`（prefetch_all_geom_inputs 等全是 no-op 桩）、`instance_cache.rs`、`cache_flush.rs`、`cata_resolve_cache_pipeline.rs` 均为桩；cate/loop/prim processor 注释明确「离线生成路径已移除，直接走 SurrealDB」。
- `model_store.rs`（model_query_response/model_query_take）已不存在于代码树（AGENTS.md 签名是陈旧的）；模型表读写就是 `project_primary_db()`==SUL_DB。
- cache_miss_report（Direct 模式）只做缺失记录，不是缓存。

## 2. 已有缓存层清单与命中范围

| 缓存 | 位置 | 键 | 生命周期/失效 | 覆盖的读 |
|---|---|---|---|---|
| TREE_INDEX_CACHE | 进程内 DashMap | (tree_dir, dbnum) | 进程；tree 文件重建后需重启 | 全部层级遍历 |
| db_meta_info.json（DbMetaManager） | 内存 | ref0→dbnum、dbnum→file/sesno | ensure_loaded 一次 | dbnum 解析、source_version |
| GLOBAL_TRANSFORM_CACHE | 进程内 DashMap | (dbnum, refno) | 批间可 clear/pin | world/local transform |
| transform rkyv 快照 | `<model_cache>/transforms/*.rkyv` | dbnum 整库 | source_version（sesno）失配重建；构建失败负缓存 | transform 冷启动 |
| SCOM_INFO_MAP | rs-core 全局 DashMap | SCOM refno | 进程 | SCOM 属性+gm/axis 参数 |
| cata_resolve_cache | 进程内 DashMap | cata_hash | 进程 | resolve_desi_comp 产物 |
| query_single_by_paths LRU | rs-core `#[cached(size=10000)]` | (refno,paths,fields) | LRU | SPRE/CATR/GMRE/LSTU 等图跳转 |
| EXIST_MESH_GEO_HASHES + aabb 缓存文件 | 内存+磁盘 | geo_hash | save_aabb_cache_to_disk | mesh 存在性/AABB |
| RecentGeoDeduper | 进程内 | geo_hash | 单次运行 | mesh 重复生成去重 |

**没有缓存的读**：`get_named_attmap`（及其伪批量）、`get_or_create_cata_context`（每元素 DESI attmap）、boolean 回填扫描、inst_relate/geo_relate 导出查询。

## 3. 生成期间真正打 SurrealDB 的热点（按代价排序）

1. **逐元素 attmap 点查**（get_named_attmap 及 get_attmaps_batch 的 for 循环）：O(元素数) 次独立小查询，CATE 元素每个 2~3 次。cata_model.rs 自带 `db_time_get_named_attmap / get_world_transform / get_cat_refno` 计时统计，说明这三项就是实测热点。
2. **transform miss 的惰性计算链**：每个 miss = attmap×2 + 祖先链逐级查询；rkyv 快照新鲜时几乎为 0，快照失效/新元素时集中爆发。
3. **CATA 解析簇**（SCOM_INFO_MAP/cata_resolve_cache miss 时）：attmap + 图跳转 + collect_descendant_full_attrs；unique SCOM/cata_hash 数量决定总量，通常远小于元素数。
4. **pe_transform 全表 id 扫描**（rkyv 构建 collect_refnos）与 **inst_relate has_cata_neg 全表谓词扫描**（布尔回填）：单次但重。
5. **TUBI/BRAN 预取**：每 branch 2 次 attmap + LSTU→CATR 图查询（部分 LRU 命中）。
6. mesh/inst_geo existence 与收尾 aabb 补查：批量化较好，量级次要。

## 4. 内存模式对读的收益评估

「内存模式」按三种理解分别评估：

**A. SUL_DB 用 `mem://`（kv-mem 嵌入式）或站点级内存实例**
- 受益面 = 上面第 3 节全部热点，尤其是逐元素 attmap 点查：嵌入式（File/mem 同为进程内 `Surreal<Any>`）没有网络往返，主要开销从 I/O 变成查询解析/执行本身。相对默认 `rocksdb://`（File 模式），kv-mem 省掉 LSM 读放大与块缓存 miss，对**点查密集**负载（attmap、pe_transform chunk 查、inst_geo existence）有实际收益，但收益上限受限于：查询仍逐条走 SurrealQL 解析 + 执行管线，**每查询固定开销（parse/plan）不因内存后端消失**。伪批量 for 循环点查的问题在内存模式下依然是 O(N) 次查询。
- 全表谓词扫描（has_cata_neg、pe_transform id 扫描）在内存后端加速最明显（纯内存扫描 vs RocksDB 迭代）。
- 结构遍历零收益（本就不走 DB）；transform/CATA 命中缓存后零收益。
- 代价：PE/ATT + 模型表同库，全库驻内存的内存占用；versioned=true（specs/022）与 mem 后端的持久化/历史保留语义冲突 —— mem 模式重启即丢，无法承载锚点/版本历史，**只适合一次性离线生成场景，不适合站点常驻服务**。

**B. Ws 远程 → File 嵌入式（不换后端，只去网络）**
- 对逐元素点查是数量级收益（每查询省一次 ws 往返 + 序列化）。当前默认已是 File 模式；仍在用 Ws 跑生成的部署，切嵌入式比换内存后端收益更大、更稳。

**C. `mem-kv-save`（SUL_MEM_DB，PE 数据副本进内存 KV）**
- 现状**只有写路径**（额外备份 PE），rs-core 中没有任何读路径消费 SUL_MEM_DB —— 对读收益为 0，除非专门把 get_named_attmap 改造成先查 SUL_MEM_DB。

**结论与建议**：
- 读侧真正的瓶颈形态是「每元素多次点查 + 每查询固定开销」，缓存体系（tree/rkyv/SCOM/cata_hash）已把可复用的重读挡掉了。内存模式能压低单次点查成本（对 File→mem 是中等收益、对 Ws→嵌入式是大收益），但**改不掉 O(N) 查询次数**。
- 性价比排序：① 真批量化 attmap（一条 `SELECT ... FROM [pe:...,pe:...]` 替代 for 循环点查，与 pe_transform 的 200/chunk 写法对齐）> ② Ws 部署改嵌入式 > ③ 离线一次性生成场景用 mem 后端（或生成前把库文件放 RAM 盘）> ④ mem-kv-save 读侧改造（需开发，收益≈批量化后的 attmap 命中内存）。
- 若做 mem 模式实验，优先用 `AIOS_CATA_P1_TRACE_REFNO` / cata_model 的 db_time_* 统计和 cache_miss_report 对比 attmap/transform 两项耗时即可定量验证。

## 附：关键文件索引

| 文件 | 角色 |
|---|---|
| src/fast_model/gen_model/query_provider.rs | 全局 Provider（TreeIndex 层级 + Surreal PE/ATT），tree 缺失自动 sync_pdms 重建 |
| src/fast_model/gen_model/query_compat.rs | 兼容层：层级查询全部 TreeIndex 化 |
| src/fast_model/gen_model/tree_index_manager.rs | .tree 加载/缓存/BFS；resolve_dbnum（cache-only，禁回退 DB） |
| src/fast_model/gen_model/query.rs | query_gm_params：collect_descendant_full_attrs 一次图查询 |
| src/fast_model/gen_model/resolve.rs | SCOM/CATA 解析读序列 + SCOM_INFO_MAP + fn::get_ipara |
| src/fast_model/gen_model/transform_cache.rs / transform_rkyv_cache.rs | 三级 transform 缓存 + pe_transform 批查 + 惰性计算兜底 |
| src/fast_model/gen_model/db_meta_cache.rs / data_interface/db_meta_manager.rs | ref0→dbnum 内存映射 |
| src/fast_model/model_cache/* | foyer 移除后的桩 + cata_resolve_cache（唯一存活） |
| src/fast_model/gen_model/boolean_backfill.rs | inst_relate/geo_relate 布尔回填读 |
| src/fast_model/gen_model/mesh_generate.rs | inst_geo existence、fetch_inst_relate_refnos（pe_transform 全表 id） |
| src/fast_model/gen_model/inst_query.rs | 导出期 inst_relate(_bool)/geo_relate 读 |
| ../rs-core/src/rs_surreal/query.rs | get_named_attmap/get_pe 点查原语；query_single_by_paths LRU |
| ../rs-core/src/query_provider/surreal_provider.rs | get_attmaps_batch/get_pes_batch = for 循环点查（伪批量） |
| ../rs-core/src/transform/mod.rs | get_transform_mat4 惰性计算 + pe_transform 回写 |
