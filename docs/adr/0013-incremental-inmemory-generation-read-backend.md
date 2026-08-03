---
status: proposed
---

# 增量模型生成的内存生成读后端（源文件按需装载，脱离常驻 SurrealDB 读）

## Context

ADR-0012 把 kv-mem 严格限定在**初始化全量模型生成**：投影内容只有层级（`pe` + `pe_owner`），
承载体是**内存版 SurrealDB 站点**（`Surreal::new::<Mem>`，为复用 SurrealQL 图查询），
且明确「增量生成、欠账追赶、历史版本生成、受控修复保持原路径」。

当前增量模型生成（spec 026 / `version_management/increment_run.rs`）的读路径是：

- 先把本轮 PE/ATT **落库**到持久 SurrealDB（`run_increment` 在 `--no-persist` 且
  `--generate-model` 时直接 bail：`incremental model generation requires persisted PE/ATT data`）；
- 再由 `SurrealVersionedReadSession`（`generation_read/surreal.rs`）查常驻库读取，
  每条查询带 `VERSION` 后缀 = MVCC as-of「生成读取时刻」保证一致切面。

spec 025（本模块 `generation_read/`）已经把生成输入抽象成**存储无关读取契约**
`VersionedReadSession`（`ElementRead` / `AttributeRead` / `HierarchyRead` /
`CatalogGraphRead` / `TransformRead` 五个能力），并有 boundary test 强制生成管线
（orchestrator/gen_pipeline/loop_processor/prim_processor/cate_processor/cata_model）
不得直接触碰 SurrealDB / SQL / `project_primary_db` / `query_provider`。但目前
`factory.rs::open_generation_read_session_with_spec` **硬编码**唯一后端 `SurrealVersionedReadBackend`。

用户诉求：增量更新生成模型时**不依赖常驻数据库**，把需要更新的节点（子树 + 其依赖的 CATA 闭包）
数据加载到内存（kv-mem），在内存生成模型，再写回 RocksDB。

两个已具备的地基：

1. spec 002 已实现 refno 级**源文件部分解析**原语（`data_interface/cata_closure.rs`）：
   `collect_design_subtree_outbound`（按设计根 refno 沿 `children` 部分解析子树 + owner 祖先链）、
   `CataClosureResolver`（子树出向引用 → CATA 引用闭包 BFS，可 `with_retain_attmaps` 保留完整属性表）、
   `parse_db_refnos`（by-refno 部分解析）。这套完全在源 db 文件上工作，不碰常驻库。
2. 「RocksDB」在本系统即 SurrealDB 的存储引擎（`options.rs::rocksdb_conn_str`，
   `versioned=true` 带 MVCC）；写回 RocksDB 当前 = 写回 SurrealDB。

唯一的技术缺口在 transform：world 变换矩阵目前**预计算并存在 `pe_transform` 表**里，
读侧（`TransformRead::load_transforms`）只是取；计算算法已存在
（`pe_transform_refresh.rs::compute_world_mat_from_owner_chain` 沿 owner 链累积，
底层 `aios_core::get_world_transform`），但其输入取自全局 SurrealDB 上下文。

## Decision

### 边界与启用

- 本 ADR 扩展 ADR-0012 的适用边界：把「按需内存生成读」从**初始化全量**扩展到**增量 scope**，
  并把承载体从「内存版 SurrealDB + 仅层级投影」改为「纯内存快照 + 完整生成读输入
  （层级 / 属性 / CATA / transform）」。ADR-0012 的初始化全量路径保持有效，不被本 ADR 替代。
- 新增第二个 `VersionedReadSession` 后端 `InMemoryReadSession`，数据源为 spec 002 的源文件
  部分解析产物，构建为进程内不可变快照；生成期只读该快照，不发起任何常驻库查询。
- 后端选择显式化：`GenerationReadBackendKind` 增加 `InMemory` 变体，`factory` 增加分支；
  遵守 spec 025 FR-012「显式选择、无自动 fallback」。

### 生成算法与管线零改动

- 严格遵守 spec 025 FR-002（生成只依赖五个领域接口）与 spec 026 FR-011（增量追赶**复用同一条
  Incremental scope 管线**，含 `pre_cleanup_for_regen_versioned` 与 delete 桶清理，
  **不引入第二套生成路径**）。本 ADR 只是「同一管线换一个读后端」，不改生成算法。

### 数据装载（kv-mem）

- 装载器复用 spec 002：`collect_design_subtree_outbound(locator, roots, include_owner_chain=true)`
  产出设计子树全部元素 + owner 祖先链 + CATA 种子；`CataClosureResolver.with_retain_attmaps(true)`
  产出 CATA 闭包节点 + 完整属性表 + children。
- 增量 scope 的根来自 `IncrGeoUpdateLog` 五桶变更 refno（与 spec 026 一致），
  按目标 sesno 从源 db 文件部分解析（`get_element_at_session`）。
- 一致性不依赖 SurrealDB MVCC：内存快照即「按目标 sesno 解析出的目标态」，`read_at=None`；
  内存后端可跳过 `ensure_hierarchy_coverage` 的全量 `pe_owner` 审计门槛（层级直接来自源文件）。

### world transform 内存化（唯一新增计算）

- 把 world 计算从「输入取自全局库」改造为「输入取自内存快照」：owner 祖先链已在快照内
  （`include_owner_chain`），local 矩阵由快照内属性（POS/ORI 等）计算，world 沿 owner 链累积。
- 抽出不依赖全局 SurrealDB 上下文的纯计算函数（复用 `compute_world_mat_from_owner_chain` 的算法），
  作为 `InMemoryReadSession::load_transforms` 的实现。此为正确性关键校验点。

### 写回保持 SurrealDB(rocksdb 引擎)

- 遵守 spec 025 FR-010（读写后端独立）：`ModelWriterMode::Surreal` 不变，模型仍写 SurrealDB
  versioned 表，保留 spec 024 的 `sesno_version_anchor` 锚点 / `VERSION AT` 历史 / `model-diff`。
- **不裸写 RocksDB**：绕过 SurrealDB 直接写 RocksDB 会丢失模型版本 MVCC 能力并与 spec 024 冲突，
  本 ADR 明确不采纳。

### 增量放开「强制 persist」与锚点语义

- 放开 `--no-persist` + `--generate-model` 的互斥：PE/ATT 是否落库变为可选（模型写回与数据落库解耦）。
- 与 spec 026 FR-010 对齐：`--no-persist` / 非 Surreal writer 语义下 **不发 `model_gen` 锚点、
  不消费欠账行**（因为没有对应的已提交数据版本）。要推进模型生成水位，仍需常规 persist 增量。
- 因此本后端的默认定位是「读侧加速 / 脱库试算与校验」；作为**权威**模型水位推进路径时，
  PE/ATT 落库仍是前提（此时内存后端只替换读取来源，锚点与欠账语义不变）。

### shadow 校验门

- 内存后端与 SurrealDB 后端在同一 scope 上逐元素比对（对标 spec 025 SC-001/SC-002 与
  `verify-cata-closure`）：`inst_relate` / `geo_relate` 指纹、world transform、成员完整性一致，
  方可从 shadow 切换到正式启用。

## Consequences

- 增量生成可在「不落库、不依赖常驻库」前提下产出模型并写回 SurrealDB(rocksdb)，
  「先落库 → 再查库」被解耦；试算 / 单元素调试 / 校验成本大幅下降。
- transform 从「读库预算值」升级为「内存现算」，是唯一新增计算，也是最主要的正确性风险与校验重点。
- PE/ATT 不落库时，`pe_owner` 完整性证据、`e3d_tree_api` 等依赖持久库的查询能力不随该次增量更新；
  该模式不推进模型生成水位（不发锚点），语义在 spec 028 明确。
- 与 spec 024 无冲突：写回仍走 SurrealDB versioned 单一真相源。
- 与 spec 025/026 无冲突：复用五能力契约与同一 Incremental scope 管线，只新增一个读后端。
- 残留风险：world transform 一致性；按名引用边（spec 002 R2 残余，DTAB/CATREF）需运行期惰性兜底
  （但惰性兜底当前落库到 SurrealDB，脱库模式下需改为落内存快照）；内存预算随 scope 规模增长。
