# 树查询统一 pe_owner / indextree 退役开发计划（2026-07-20）

> **M4/M5 已由** [`docs/superpowers/specs/2026-07-21-gen-pipeline-cleanup-rename-design.md`](../superpowers/specs/2026-07-21-gen-pipeline-cleanup-rename-design.md) **与 GenPipeline 清理开发计划承接**（配置硬切 `gen_pipeline_*`、运行时仅 pe_owner、模块改名、生产侧停产 `.tree`）。本文件 M0–M3 仍为历史落地记录；勿再按下方 M4/M5 任务表重复开工。

> 依据：2026-07-20 对「增量更新下 indextree 适配性」的代码审核。
> 审核结论：`.tree`（TreeIndex）只在全量解析 / 手工 `--gen-indextree` 时产出，
> 增量路径（`incremental-sesno` / `watch-incremental` / web `/api/incremental/*`）**从不重建它**，
> 进程内 `TREE_INDEX_CACHE` 也**永不失效**（`tree_index_manager.rs` 只有 get/insert）。
> 增量常态化后，indextree 作为「latest 层级」唯一来源已不满足要求；specs/023 已把带 sesno
> 的树查询迁到 versioned `pe_owner` 边，本计划把 **latest（不带 sesno）路径也统一到 pe_owner**，
> 彻底退役 `.tree` 的生产与消费逻辑，并同步修订相关文档。

## 0. 背景：indextree 在增量场景下的具体失效面（审核证据）

| # | 失效面 | 证据 |
|---|---|---|
| 1 | 增量不维护 `.tree`：新增元素查不到、删除元素仍在、移动元素父子错 | `sesno_increment.rs` 全文不写 tree；`increment_run.rs::build_tree_index_evidence` 只做存在性/mtime 证据并注明 “Do not auto-run long --gen-indextree work from watcher/default incremental paths” |
| 2 | 进程内缓存永不失效：即使手工重建 `.tree`，长驻 web_server 仍用旧索引 | `tree_index_manager.rs` L33/L149/L165 只有 get/insert，无 remove/clear |
| 3 | 增量模型生成静默漏元素：BRAN 下新增管件（如 ELBO）从旧 tree 收集不到 → 不生成 | `index_tree_mode.rs` L827-840 `collect_bran_cate_descendant_elements_from_tree`；`orchestrator.rs::filter_bran_hang_refnos` L1020 `node_meta` 查不到直接 `continue` |
| 4 | `rebuild-pe-owner` 候选枚举依赖 tree，前提「tree 与库内同源新鲜」在增量后不成立 | `version_management/cli.rs` L4280-4310 |
| 5 | 无版本维度（单快照）——该问题 specs/023 已用 versioned pe_owner 解决，但 latest 路径仍走 tree | `e3d_tree_api.rs` L784-787（带 sesno 走版本分支，否则 TreeIndex） |

数据侧能力已就绪：`pe_owner` 边 id = `pe_owner:[<owner_key>, <order>]`（`versioned_db/pe.rs` L455，
`ORDER BY id` 保同胞顺序）；full 重灌先删后插 + `pe_owner_version_meta` 可信分界；增量与 PE/ATT 同一
fingerprint/counts 提交（specs/023）；`db_meta_info.json`（ref0→dbnum 映射）增量路径已有刷新
（`sesno_increment.rs::refresh_db_meta_for_increment_files`）。

## 1. 范围与非目标

**范围**：
1. latest（不带 sesno）层级查询全部改为 pe_owner / pe 表查询（web API、模型生成管线、导出、版本管理运维命令）；
2. `.tree` 的生产（`export_tree_file` / `gen_tree_only` / `--gen-indextree`）与消费（`TreeIndexManager` 及各 helper）退役；
3. 相关文档、CLI help、smoke 脚本同步修订。

**非目标**：
- 不改带 sesno 的版本化查询（已是 pe_owner + `resolve_anchor`，specs/023 语义不动）；
- 不改锚点 / commit seam / lease / fingerprint 语义（与《2026-07-20 增量更新加固计划》正交，见排期）；
- 不删除 rs-core（aios-core）侧 `tree_query` 类型本体，本仓不再引用后由 rs-core 择期清理；
- **保留** `db_meta_info.json` 与 `resolve_dbnum_for_refno`（它基于 db_meta 而非 `.tree`，增量已维护）——仅从 `tree_index_manager` 迁出宿主。

**验证约束**（AGENTS.md）：不使用 `cargo test`；CLI 走 `--json`，web_server 起服务后 HTTP POST 验证；
SurrealQL 断言走 `db-data/*.surql` + `scripts/smoke/*.ps1`；瘦构建 `scripts/build-sync-cli.ps1` 必须保持绿。

## 2. 现状盘点：消费面清单（迁移对象，~28 文件 / ~160 处）

| 域 | 文件（引用数） | 用法 |
|---|---|---|
| Web API latest 树 | `web_api/e3d_tree_api.rs`(20)、`web_api/room_tree_api.rs`(5)、`web_api/spatial_query_api.rs`(5)、`web_server/stream_generate.rs`(2)、`scene_tree/init.rs`(4) | children/ancestors/subtree/search、offline world roots、children_count、default_name 序号 |
| 生成管线 | `gen_model/query_provider.rs`(9)、`query_compat.rs`(4)、`index_tree_mode.rs`(10)、`utilities.rs`(9, build_cata_hash_map)、`cata_model.rs`、`cata_cache_gen.rs`、`cate_processor.rs`、`neg_query.rs`、`prim_model.rs`、`orchestrator.rs`(5)、`precheck_coordinator.rs`、`pdms_inst.rs`、`room_model.rs`(7)、`tree_index_manager.rs`(本体) | noun 枚举/计数、可见/负几何子孙、BRAN 子元件收集、cata_hash 分组、dbnum 路由 |
| 导出 | `export_prepack_lod.rs`(6)、`export_dbnum_instances_parquet.rs`(7)、`export_dbnum_instances_web.rs`(3)、`export_instanced_bundle.rs`(4)、`model_exporter.rs`(3)、`spec_info.rs`(2) | visible_geo_refnos、descendants 收集、spec info |
| 版本管理/增量 | `version_management/cli.rs`（rebuild-pe-owner 候选枚举、restore-scene-tree、publish-history tree 证据）、`increment_run.rs`（TreeIndexEvidence / `--require-tree-index`）、`scene_tree_artifact.rs`、`physical_baseline_snapshot.rs`、`history_replay_plan.rs` | tree 存在性门禁、tree 工件快照/恢复 |
| 生产侧 | `versioned_db/database.rs`（`export_tree_file` ×3、`gen_tree_only`）、`versioned_db/tree_export.rs`、`data_interface/db_meta_manager.rs`（`generate_*_indextree`）、`init_project.rs`、`main.rs`（`--gen-indextree` / `--gen-all-desi-indextree`）、`cli_modes.rs`(3) | `.tree` 写出与手工重建 |
| 配置 | `options.rs` `index_tree_*` 字段(12)、`web_server/handlers.rs`(2)、`gen_model/config.rs` | 生成管线 noun 过滤/并发/批大小（语义与 tree 文件无关，仅命名） |

## 3. 前置决策（M0 输出，先定后动）

- **D1 cata_hash 通路**：`TreeNodeMeta.cata_hash` 是生成管线分组关键，pe 行目前**没有**该字段
  （解析期经 `att_map.cal_cata_hash()` 只写进 `.tree` 与 `ele_reuse_relate:[pe_key, inst_info:⟨hash⟩]` 边）。
  方案 A（推荐）：pe 行补 `cata_hash` 字段，full/增量写入路径各补一处，rebuild 工具补存量；
  方案 B：查询期从 `ele_reuse_relate` 边反查（需审计增量是否维护该边）。M0 审计后二选一。
  > **已定（2026-07-20）：方案 A**。审计结论：`sesno_increment.rs` 全文不写 `ele_reuse_relate` /
  > `cal_cata_hash`（增量新增元素无边），且 full 解析的 `ensure_ele_reuse_relate_relation_schema`
  > 首次执行 `REMOVE TABLE`（database.rs L537）——边只在 full 解析期完整，方案 B 对增量元素必然 miss。
  > 存储格式：**string**（u64 哈希可超出 Surreal int/i64 范围；与 `CataHashRefnoKV.cata_hash: String` 对齐）。
- **D2 生成管线查询策略**：交互 API 用**逐请求 SurrealQL**；生成管线（百万级节点）用
  **per-run 内存快照**——运行开始按 dbnum 批量 `SELECT id, owner, noun, cata_hash FROM pe WHERE dbnum=…`
  + `pe_owner` 邻接构建内存索引（复用/替换现 TreeIndex 内存结构），**来源永远是 DB（新鲜）**，文件退役。
- **D3 pe 表二级索引**：noun 枚举/计数（`query_by_noun_all_db` / `count_noun_all_db` / `prequery_noun_counts`）
  需要 `DEFINE INDEX idx_pe_dbnum_noun ON TABLE pe FIELDS dbnum, noun;`（现无，见 §DEFINE INDEX 盘点）。
  versioned RocksDB 下索引写放大与 count 性能需 M0 surql 实测；不达标则降级为「快照统计」。
- **D4 离线/cache-only 模式**：`query_provider` 现约定「cache-only 不回退 SurrealDB」。退役 `.tree` 后
  该模式改为**要求本地 SurrealDB（嵌入式 RocksDB）可用**，删除"无 DB 纯文件"路径；`offline_world_children`
  改查 `pe WHERE noun='WORL'` + db_meta dbnum 清单。
- **D5 存量站点回退**：pe_owner 边仅在 surreal-save 构建 + full 重灌 / `rebuild-pe-owner` 后完整。
  latest 查询统一走「pe_owner 优先、`pe.children` 字段点查回退」的双源结构（与版本路径 FR-008 同构），
  切换前用审计脚本兜底（M0）。

## M0（P0）：决策落地 + 基础设施 + 基线

### T1 pe_owner/pe latest 查询原语层 `PeOwnerTreeStore` ✅（2026-07-20）
- 新模块（建议 `src/versioned_db/pe_owner_tree.rs`），接口对齐 `TreeIndexManager` 现有签名（drop-in）：
  `query_children(_filtered)` / `query_ancestors(_filtered)` / `query_descendants(_filtered)` /
  `collect_target_refnos_pruned|grouped` / `query_noun_refnos` / `count_by_noun` / `all_refnos` /
  `query_visible_geo_refnos` / `get_node_meta` / `contains`。
- latest children SQL：`SELECT VALUE in FROM <owner_pe_key><-pe_owner ORDER BY id;`（无 VERSION）；
  边缺失回退 `pe.children`；descendants 为分层 BFS 批查（chunk 500，对齐 `exec_statements` 粒度）。
- **禁止** `pe_owner:[..]..[..]` id 区间扫（specs/023 research C3 教训在 latest 同样适用——统一图遍历）。
- 验收：新增 `scripts/smoke/pe_owner_latest_tree_smoke.ps1`——同库对比 TreeIndex 与 PeOwnerTreeStore 的
  children/ancestors/subtree/noun-count 输出，diff=0；顺序断言（ORDER BY id == pe.children 顺序）。
  > **落地修订（fork dev-3.1 实测，8030 versioned 实例全绿）**：descendants/ancestors 主路径改用
  > SurrealDB 3.1 **递归图 idiom**（用户指定，语法依据 `D:\work\plant-code\surrealdb`
  > `language-tests/tests/language/graph|idiom`）而非分层批查：
  > - descendants：`<root>.{..N+collect}<-pe_owner<-pe`（+collect BFS 邻近序去重防环，深度上限 256）
  > - ancestors：`<node>.{..N+collect}(.owner)`（owner 记录链接递归，不依赖边完整性；根自指靠 visited 终止）
  > - 剪枝/分组收集（`collect_target_refnos_*`）保留 Rust 侧逐层 BFS（中途过滤=断链语义，递归 idiom 表达不了剪枝）
  > - smoke 为 L1-L11 逐形态断言（children 序/递归/深度/回退/元信息/索引），实测全绿。

### T2 cata_hash 通路（D1）与 pe 索引（D3）✅（2026-07-20）
- 审计 `ele_reuse_relate` 增量维护现状 → 定 D1 方案；落地字段/索引 DDL 与回填工具
  （`model-version backfill-pe-cata-hash --dbnum`，幂等）；
- `DEFINE INDEX idx_pe_dbnum_noun` + surql 实测 count/枚举耗时（记入 `db-data/bench_pe_noun_index.out.md`）。
  > **落地**：D1=方案 A（见 §3 修订）。写入三路齐：full（`pe.rs::save_pes` 注入 pe 行 JSON）、
  > 增量（`sesno_increment.rs::inject_cata_hash_into_pe_json`，UPSERT CONTENT 整行替换必须注入否则
  > modify 会抹掉字段）、存量（`model-version backfill-pe-cata-hash --dbnum <D> [--compute-missing]
  > [--dry-run] [--json]`：快路径搬 `->ele_reuse_relate.out` 边、慢路径 opt-in 逐行 ATT 重算）。
  > `idx_pe_dbnum_noun` 由 `PeOwnerTreeStore::ensure_pe_dbnum_noun_index()` 幂等定义，backfill 启动时自动建。
  > 大库 count/枚举耗时基线待 t012 库实测补录（bench_pe_noun_index.out.md）。

### T3 边完整性审计 + 性能基线 ◐（审计已落地，基线待跑）
- `db-data/audit_pe_owner_vs_children.surql`：按 dbnum 抽样对比 `count(<-pe_owner)` 与 `count(pe.children)`，
  输出不一致清单（复用 M4/T6《增量加固计划》的实测计数口径）；
- 基线：`run_t012_release_bench.ps1` 跑一轮全量生成记录 wall time；`.tree` 反序列化耗时 vs 快照加载耗时对比。
  > **落地**：审计对 = `audit_pe_owner_vs_children.surql` + `scripts/smoke/pe_owner_children_audit.ps1`
  > （[1] per-dbnum 总量 [2] 抽样 parent 边/字段计数对比 [3] childless 残留脏边；FAIL 退出码 1，
  > 输出 `db-data/audit_pe_owner_vs_children.out.md`；支持 `-Dbnum` 收窄）。fixture 上已验证能正确
  > 抓出"有 children 无边"的 pe:f（FAIL 路径）。**D5 前置铁律**：站点审计不绿禁止切
  > `query_descendants` 递归主路径（混合态在缺边节点静默截断，见 pe_owner_tree.rs 模块注释）。
  > t012 性能基线未跑（与另一会话的增量加固工作共用构建目录，避免干扰，待其收敛后补）。

## M1（P0）：Web API latest 路径切换

### T4 e3d_tree_api 不带 sesno 分支 → PeOwnerTreeStore ✅（2026-07-20）
- `get_children` / `get_node` / `get_ancestors` / `get_subtree_refnos` / `search` / world roots /
  `children_count` / `default_name` 序号（保持 `{noun} {order+1}` 语义，order 来自边序）；
- 版本分支零改动；`offline_world_children` 按 D4 改 DB 查询。
- 验收：起服务 HTTP 对比（迁移前后同库响应 diff=0）；**新能力验收**：增量提交一个「新增/删除/移动」
  元素的区间后，不带 sesno 的 children 立即反映变化（现状 TreeIndex 做不到，这是本计划的核心收益）。
  > **落地（2026-07-20）**：双源开关 `AIOS_TREE_QUERY_SOURCE=pe_owner`（默认）|`tree`（一键回退，
  > M4 删除），判定收敛在 `pe_owner_tree::latest_tree_source_is_pe_owner()`。切换点：
  > - `get_children` 非 WORL 分支 → `query_children_dtos_pe_owner`（边序 children +
  >   `fetch_node_metas`/`query_children_counts` 批查，不再逐子回环）；
  > - `get_ancestors` → `PeOwnerTreeStore::query_ancestors`（owner 链接递归，根→父序）；
  > - `get_subtree_refnos` → `query_descendants` 递归 idiom（错误显式返回，不静默空集）；
  > - `query_node` 空名序号 → 边序 position；`get_visible_insts` BRAN/HANG 过滤 → 批量 meta；
  > - `offline_world_children` 转 async + `offline_world_children_pe_owner`（D4：先 parent 直子，
  >   合成 world 再按 manual/db_meta dbnum 清单查 `pe WHERE noun='WORL'` 取其 children，
  >   空则回退该库 SITE 清单）；`try_offline_world_children_count` 随之 async。
  > - `search` 本就走 noun 表（`query_noun_hierarchy`），无 tree 依赖，零改动；
  >   versioned 分支与 `resolve_dbnum_for_refno`（db_meta 驱动）未动。

### T5 其余 API 触点 ✅（2026-07-20，stream_generate 无需改）
- `spatial_query_api`（parent noun 路由 + 直接子节点）、`room_tree_api`、`stream_generate`
  （`query_visible_descendants`）、`scene_tree/init`（scene_node 构建源改 pe_owner）。
- 验收：对应 HTTP smoke + `scripts/smoke/tree_version_smoke.ps1` 扩展 latest 场景。
  > **落地（2026-07-20）**：
  > - `spatial_query_api`：parent noun 路由 → `PeOwnerTreeStore::get_noun` 点查；直接子节点
  >   分支 → `query_children`（`tree` 回退保留）；
  > - `room_tree_api`：`model_children` / `model_children_count` 转 async 双源（计数走
  >   `query_children_counts` 边计数+字段回退）；`query_room_item_children` → 边序 children +
  >   批量 meta/计数；COMP_GROUP 展开处 map 闭包改显式循环以支持 await；
  > - `scene_tree/init`：`build_tree_from_world` 双源，pe_owner 主路径为逐层批查 BFS
  >   （`children_batch` + `fetch_node_metas`，visited 防环；scene_node 无同胞顺序语义）；
  > - `stream_generate::query_visible_descendants`：**审计结论=本就 DB 驱动**（逐节点
  >   `aios_core::get_children_refnos` 点查 `pe.children`，含 `!deleted` 过滤），无 tree 依赖，
  >   不改（避免动 deleted 语义）；仅存的 `resolve_dbnum_for_refno` 按约保留。
  > **验证**：`cargo check --lib` 与 `--features web_server` 全绿（另一会话并发改动下反复漂移，
  > 编辑均已基于最新内容重放）；`pe_owner_latest_tree_smoke.ps1` 复跑全绿（8030）。
  > **未验证项（留站点环境）**：起 web_server 的 HTTP 前后对比 diff=0、
  > 「增量提交后 latest children 立即可见」端到端场景（fixture 库无完整 web_server 配置；
  > 双源开关可在站点上 A/B 对拍）。`tree_version_smoke.ps1` latest 场景扩展并入该轮一起做。

## M2（P0→P1）：模型生成管线切换（依赖 T1/T2）

### T6 快照 Provider 替换 ✅（2026-07-20）
- `query_provider::init_provider`：`TreeIndexQueryProvider` → `PeOwnerSnapshotProvider`（D2 快照，
  per-run 按需加载 dbnum，保留 64MB 大栈线程仅在构建期需要时使用或直接删除该 workaround）；
- `query_compat`（visible/neg descendants、deep children、ancestors）、`neg_query::query_descendants_map_by_dbnum`、
  `utilities::build_cata_hash_map_from_tree*`（改从快照/pe.cata_hash）、`index_tree_mode` BRAN 子元件收集
  （`collect_bran_cate_descendant_elements_from_tree` → 快照 BFS）、`orchestrator::filter_bran_hang_refnos`、
  `cate_processor` / `cata_cache_gen` / `prim_model` / `precheck_coordinator` / `room_model` / `pdms_inst` 触点。
- `prequery_noun_counts` / noun 枚举走 T2 索引或快照统计。
- 验收：
  1. t012 全量生成 bench 对比 M0 基线（预算：总耗时回退 ≤10%，超标走 D2 快照调优或分块并行加载）；
  2. **增量缺陷修复验收**：既有 BRAN 增量新增 ELBO → 增量生成产物含新管件 inst（现状会漏，修 §0-3）；
  3. `cache_miss_report` 无新增 miss 类别；`--dry-run-gen` 收集 refno 集与迁移前一致。
  > **落地（2026-07-20）**：
  > - 基础设施两件：`versioned_db/pe_owner_snapshot.rs`（per-dbnum 快照，cursor 分页
  >   `WHERE dbnum AND id > <last> ORDER BY id`（fork 实测）批量拉 id/owner/noun/cata_hash/children，
  >   页大小默认 2000、`AIOS_PE_SNAPSHOT_PAGE_SIZE` 可调；BFS/prune/include_self/max_depth/ancestors
  >   语义**逐行对齐 rs-core `TreeIndex`** 并加 visited 防环；`invalidate_pe_snapshots()` 于
  >   `gen_all_geos_data` 入口强制调用——快照是 run 级缓存，不复刻"永不失效"缺陷）+
  >   `gen_model/hier_view.rs`（`HierView` Snapshot|Tree 双源桥接，方法面向 TreeIndexManager
  >   签名对齐，加载后全同步查询，sync 闭包场景可用 `preload_pe_snapshots` + `try_get_cached_pe_snapshot`）。
  > - Provider：`PeOwnerSnapshotProvider`（层级走快照、其余委托 SurrealQueryProvider，与
  >   TreeIndexQueryProvider 委托结构同构）；`query_by_noun_all_db`/`count_noun_all_db`/
  >   `query_noun_page_all_db` 改 **async** 双源（processor/prepack 调用点同步 .await）；
  >   `query_multi_descendants_with_self` 双源（覆盖 pdms_inst 清理、pdms_inst_surreal、
  >   instanced_bundle 展开、index_tree_mode collect_all_descendants、cli_modes 验证路径）。
  > - 消费面：query_compat 四个私有 helper 双源（query_descendants_bfs / query_children_filtered /
  >   get_node_meta_dual / query_filter_ancestors）；`neg_query::query_descendants_map_by_dbnum_dual`
  >   新增（prim_model 856 切换）；utilities cata_hash 分组走 pe.cata_hash 字段，元件类 noun
  >   （USE_CATE∪BRAN_COMPONENT）缺字段回退 attmap 计算并记 cache_miss_report（kind=
  >   `pe_cata_hash_missing`）；BRAN 子元件收集双源收敛在 tree_index_manager 两个 collect fn 内部
  >   （cata_model 6878 / cata_cache_gen 702 / index_tree_mode 830 三处调用点零改动自动覆盖，修 §0-3）；
  >   orchestrator::filter_bran_hang_refnos meta miss 记 `hier_meta_missing` 不再静默；
  >   room_model 三处（可见几何收集 / 空间聚合节点表转 async / 房间面板 preload+同步闭包）；
  >   precheck 在 pe_owner 源跳过 .tree 检查；cate_processor/loop 等经 provider/快照间接覆盖。
  > - cli_modes：tubi cache BRAN/HANG 枚举切 HierView；"缺 .tree 自动 gen_tree_only 解析"仅
  >   tree 回退源保留。

### T7 导出路径切换 ✅（2026-07-20）
- `export_prepack_lod` / `export_dbnum_instances_{parquet,web,v3}` / `export_instanced_bundle` /
  `model_exporter` / `spec_info`（`build_spec_info_parquet` 的 tree_dir 入参改快照）。
  > **落地（2026-07-20）**：`model_exporter::collect_export_refnos` 双源（pe_owner 免 .tree 存在性
  > 门槛）；`spec_info` 内部 `SpecHierSource` 枚举双源（tree_dir 入参仅回退路径使用，签名不变）；
  > parquet 导出的 `tree_owner_refno`/`resolve_spec_value_with_ancestors`/`append_tubing_rows_for_owner`/
  > `append_owner_chain_rows` 参数 `&TreeIndexManager` → `&HierView`，入口 `HierView::load(vec![dbnum])`；
  > web 导出全库枚举双源；instanced_bundle BRAN/HANG 过滤切 HierView；prepack 全库 refno 枚举切
  > HierView（root 模式经 query_compat 已双源）；v3 导出经共用路径覆盖（自身无 tree 触点）。
  > **验证（2026-07-20）**：`cargo check --lib`、`--lib --features web_server`、`--bin aios-database`
  > （默认特性）、`--bin aios-database --no-default-features --features sync-cli` 全绿；
  > `pe_owner_latest_tree_smoke.ps1` 复跑全绿；快照 cursor 分页 SQL（首页 + `id > <last>` 续页）
  > 在 8030 fixture 实测正确续接。
  > **未验证项（留站点环境，与 T3 基线一并补）**：t012 全量生成 bench ≤10% 预算、
  > 「BRAN 增量新增 ELBO 立即生成」端到端、`--dry-run-gen` 迁移前后 refno 集对拍、
  > 大库（百万行）快照加载耗时实测。
- 验收：同库导出产物对比（refno 集合、manifest、instances 数量 diff=0）。

## M3（P1）：版本管理 / 增量域清理

### T8 增量入口证据与门禁改造 ✅（2026-07-20）
- `increment_run.rs`：删除 `TreeIndexEvidence` 与 `require_tree_index`（含 `main.rs` watch
  `--require-tree-index` flag），替换为 **pe_owner 完整性证据**：`pe_owner_version_meta.maintained_since_sesno`
  存在 + T3 审计抽查通过；degraded 语义（patch_only/quarantined）保留但判据换源。
- `version_management/cli.rs`：`rebuild-pe-owner` 候选枚举改 `SELECT id FROM pe WHERE dbnum=… START … LIMIT …`
  分页（去掉「tree 与库内同源新鲜」前提，修 §0-4）；`restore-scene-tree` 命令与 `scene_tree_artifact.rs`
  退役（保留 `db_meta_info.json` 的快照/恢复子集）；`publish-history` / `physical_baseline_snapshot` /
  `history_replay_plan` 的 scene_tree 证据项同步降级为 db_meta_info.json 检查。
- 验收：`incremental-sesno --json` / `watch-incremental --once --json` 回归；rebuild 在无 `.tree` 站点可运行。
  > **落地（2026-07-20）**：
  > - **证据换源**：`TreeIndexEvidence` → `PeOwnerEvidence`（`increment_run.rs::build_pe_owner_evidence`，
  >   async）：per-dbnum 读 `pe_owner_version_meta.maintained_since_sesno` + 在线抽查（≤200 个有子
  >   parent 对比 `count(<-pe_owner)` vs `len(children)`，口径对齐 audit surql [2] 段，LET+RETURN 一次
  >   请求完成，8030 实测形态 OK）；证据本身咨询性——单库探测失败记 not_ready 带 error 不终止运行，
  >   仅 strict flag 升级为快速失败。summary 字段 `tree_index` → `pe_owner_evidence`
  >   （manifest_version=`incremental_pe_owner_evidence:v1`，ready/mode[strict_required|ready|degraded_allowed]/
  >   required/sample_limit/checked_dbnums/not_ready_dbnums/dbnums[]/recommendation，recommendation 指向
  >   rebuild-pe-owner + 审计脚本）。
  > - **flag 更名**：`--require-tree-index` → `--require-pe-owner-ready`（incremental-sesno 与
  >   watch-incremental 两处，直接删除不留别名）；`IncrementRunOptions.require_tree_index` →
  >   `require_pe_owner_ready`；`WatchIncrementalOptions` 同步；watch summary 打印行改
  >   `pe_owner_evidence: ready/mode/not_ready_dbnums`；web `incremental_update_handlers` 构造点同步。
  > - **rebuild-pe-owner 重建**（并发会话瘦身 cli.rs 时被整体删除，自 stash `pre-sync-20260720-145423`
  >   找回并按 T8 改造）：候选枚举改 **pe 表 cursor 分页**（`WHERE dbnum AND id > <last> ORDER BY id
  >   LIMIT 500`，与 pe_owner_snapshot 同形态；弃 START 偏移——无序 START/LIMIT 页序不稳会漏读/重读），
  >   彻底去掉 `.tree` 依赖（修 §0-4，无 `.tree` 站点可运行）；T021 实测教训全部保留（verify-and-skip
  >   只重写不一致 owner、删段/插段分批请求、慢路径幂等冲突判定、幽灵 owner 清理）；幽灵清理加
  >   dbnum 归属过滤（候选集只含本 dbnum，非本库 owner 不算幽灵，经 db_meta ref0→dbnum 判定）；
  >   `dbnum_info_table` 无 sesno 拒绝写 meta 语义不变；main.rs ensure_surreal_connected 门已含。
  > - **顺带修正**：M0 `backfill-pe-cata-hash` 的枚举分页同步从 START 偏移改为 cursor 分页（同一
  >   稳定性理由）。
  > - **退役目标核实**：`restore-scene-tree`/`scene_tree_artifact.rs`/`physical_baseline_snapshot.rs`/
  >   `history_replay_plan.rs`/`model_release`/`ducklake_store` 已由 **spec 024 批次**（并发会话，
  >   CHANGELOG 2026-07-20 Spec 024 条目）整体退役——文件仍在磁盘但已无 mod 声明、无编译引用，
  >   本批只核实无残留（rg 确认），文件删除归 024 批次收尾。
  > **验证（2026-07-20）**：三面 `cargo check`（--lib / --features web_server / sync-cli 瘦特性）全绿；
  > 8030 fixture 实测 rebuild 全语句序列（cursor 首页+续页终止、边 bulk 读投影、children 批查、
  > 先删后插修复 pe:f 缺边、meta UPSERT、evidence 抽查 probe sampled=4/mismatched=0）；
  > `pe_owner_children_audit.ps1` 修复前 FAIL(1) → 修复后 PASS(0) 闭环；
  > `pe_owner_latest_tree_smoke.ps1` 复跑全绿。
  > **未验证项（留站点环境）**：真实站点 `incremental-sesno --json` / `watch-incremental --once --json`
  > 端到端回归（本地无完整工程配置）；大库 rebuild 耗时（cursor 分页 + verify-and-skip 的写放大为零，
  > 预期优于旧实现）。

## M4（P1→P2）：生产侧退役与代码删除

### T9 停产 `.tree` 与 CLI 清理
- `versioned_db/database.rs` 三处 `export_tree_file` 停写、`tree_export.rs` 删除；
  `gen_tree_only` 模式**保留轻量扫描但只产出 `db_meta_info.json`**（重命名为 `gen_db_meta_only`，
  `init_project` 第 1 步、`db_meta_manager::generate_*_indextree`、`main.rs --gen-indextree/--gen-all-desi-indextree`、
  `cli_modes.rs` 的 tree 缺失自动解析随之改造/删除）；
- `tree_index_manager.rs` 删除：`resolve_dbnum_for_refno` 迁至 `db_meta_manager`（全部调用点改引用，
  该方法本就是 db_meta 驱动）；`TREE_INDEX_CACHE`、`load_index_with_large_stack`（Windows 栈溢出
  workaround）随之消失；
- `options.rs` `index_tree_*` 字段**保留字段名**（toml 兼容），注释改为「生成管线 noun 过滤/并发配置」，
  `get_index_tree_*` 方法改名加 deprecated 别名；
- 运维指引：站点 `scene_tree/` 目录仅剩 `db_meta_info.json`，旧 `.tree` 文件可删（写入 ops-notes）。
- 验收：`cargo check --lib` / 瘦构建 / full features 构建全绿；`rg 'TreeIndex|indextree|\.tree'` 在 src/
  仅剩 rs-core 类型 re-export（若有）与历史文档。

## M5（P2）：文档与规范修订

### T10 文档对齐（与代码同批合入）
- **AGENTS.md**：specs/023 段「可信分界回退 pe.children」补 latest 路径同构说明；删除
  `--gen-indextree` 相关 Tools/todo 引述；「Auto-generated signatures」段跑 `node gen-context.js` 再生成
  （CLAUDE.md 同步）；
- **CONTEXT.md**：层级查询数据源章节改写（latest + versioned 统一 pe_owner，`.tree` 已退役）；
- **specs/023**：spec.md FR-005（「不传 sesno 走 TreeIndex 零改动」）追加修订记录标注本计划取代；
  plan.md/quickstart.md/tasks.md 增补 latest 切换任务归档；contracts/tree-version-api.md 补 latest 行为；
- **specs/022 ops-notes**：删除/改写 scene_tree 工件恢复、`--require-tree-index` 运维段；新增
  「pe_owner 完整性审计 + rebuild-pe-owner」运维口径；
- **CHANGELOG**：行为变化（latest 树实时性、`--gen-indextree` 移除、离线模式要求本地 DB、
  scene_tree 目录内容变化）；
- **scripts/smoke**：`tree_version_smoke.ps1` 场景改写，新增 `pe_owner_latest_tree_smoke.ps1`（T1）、
  审计脚本（T3）入册 quickstart。

## 排期与依赖

| 里程碑 | 任务 | 规模 | 依赖 | 优先级 |
|---|---|---|---|---|
| M0 | T1 查询原语层 | M | 无 | P0 |
| M0 | T2 cata_hash + pe 索引 | M（含回填工具） | D1 审计 | P0 |
| M0 | T3 审计 + 基线 | S | 无 | P0 |
| M1 | T4 e3d_tree_api latest | M | T1 | P0 |
| M1 | T5 其余 API | S-M | T1 | P0 |
| M2 | T6 生成管线快照 | L | T1/T2/T3 基线 | P0-P1 |
| M2 | T7 导出路径 | M | T6 | P1 |
| M3 | T8 增量域清理 | M | T1；**排在《2026-07-20 增量加固计划》M1/M2 合入之后**（同文件 `increment_run.rs`/`cli.rs`，避免冲突；T3 审计口径与其 T6 实测计数共用） | P1 |
| M4 | T9 生产侧退役 | M-L | T4-T8 全部完成（消费面清零才能停产） | P1-P2 |
| M5 | T10 文档 | S-M | 各里程碑收尾 | P2 |

建议节奏：M0 先行独立合入（不动现有路径，纯新增）；M1+M2 双源并行期（PeOwnerTreeStore 上线、
TreeIndex 保留为对照/回退），对比脚本全绿后进 M3/M4 删除；M5 随删除批次收尾。

## 全局验证清单（每批次合入前）

1. `powershell -File scripts/build-sync-cli.ps1` 瘦构建 + `cargo check --lib` + full features 构建；
2. `pe_owner_latest_tree_smoke.ps1`：双源对比 diff=0（M1/M2 期间每次必跑）；
3. 增量端到端：全量解析 → 增量（含新增/删除/移动 + BRAN 新增管件）→ latest 树 HTTP 立即可见、
   增量生成产物含新元素、`model-version history *` 不回归；
4. t012 bench：全量生成耗时对比 M0 基线（≤10% 回退预算）；
5. surql 审计：`audit_pe_owner_vs_children` 抽查、锚点链/`version_commit_state` 不回归（复用 022 断言）；
6. web：`/api/e3d/*`、`/api/spatial/*`、room tree、stream_generate HTTP 场景。

## 风险与回退

- **性能回退（最大风险）**：DB 快照加载/noun 统计慢于 `.tree` 反序列化。缓解：T3 基线先行、D3 索引、
  快照按 dbnum 并行分块加载；回退：M1/M2 双源期 provider 可一键切回 TreeIndex（feature/env 开关
  `AIOS_TREE_QUERY_SOURCE=tree|pe_owner`，M4 删除该开关）。
- **存量站点边缺失/不完整**：切换前 T3 审计必须绿；不绿站点先 `rebuild-pe-owner`（T8 改造后不依赖 tree）。
- **cata_hash 回填遗漏**：D1 回填工具幂等可重跑；生成期 miss 记入 `cache_miss_report` 显式暴露，不静默。
- **离线模式行为变化（D4）**：无本地 SurrealDB 的纯文件流程不再支持——CHANGELOG/ops-notes 显著标注；
  受影响用户先跑一次 full 解析建库。
- **顺序语义**：`ORDER BY id`（边序）与 `pe.children` 顺序一致性纳入 T1 smoke 断言；发现不一致按
  `pe.children` 为权威并记录 rebuild。
- **与增量加固计划的文件冲突**：`increment_run.rs` / `version_management/cli.rs` / `sesno_increment.rs`
  两计划都触碰——严格按排期表依赖顺序串行合入。
