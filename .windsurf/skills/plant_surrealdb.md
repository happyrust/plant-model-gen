---
description: 面向 plant/rs-core/aios-core/gen_model-dev 的 SurrealDB/SurrealQL 查询与性能实践技能。用于：编写/优化/排错 SurrealQL（pe、pe_owner、inst_relate、geo_relate、inst_geo、neg_relate、tubi_relate、scene_node/contains 等），在 Rust 侧用 SUL_DB/SurrealQueryExt/SurrealValue 做强类型查询封装，以及在层级查询场景下选择 TreeIndex/SceneTree 以获得数量级加速。
---

# Plant SurrealDB（plant 数据模型查询）

## 用法提要（先选路由）

- 先判定你要查的是什么：层级/状态/几何/管道/属性；再决定用 TreeIndex/SceneTree 还是直接 SurrealQL。


## 决策树

- 要"层级"（子节点/子孙/祖先）
  - 优先：TreeIndex（.tree 内存索引，性能通常高两个数量级）
  - 备选：SurrealDB 递归/图遍历（仅在 TreeIndex 不覆盖的关系上使用）
- 要"生成状态/叶子未生成"（gen_model 场景）
  - 优先：SceneTree（`scene_node` + `contains`）
- 要"几何实例链路"（pe -> inst_relate -> inst_info -> geo_relate -> inst_geo）
  - 用 SurrealQL + Rust 强类型结果；必要时用 `LET $refnos = ...` 做批量
- 要"管道直段"（`tubi_relate`）
  - 用复合 ID 的 ID Range 查询（避免全表扫描）
- 要"数据库端函数/批量收集"
  - 直接调用 `fn::*`（见 references）

## SurrealQL 关键约定（少走弯路）

- **Record ID 形制**（务必先确认 ID 形态再写查询）
  - `pe:⟨ref0_ref1⟩`（ref0=RefU64 高32位；ref1=低32位；ref0 不是 dbnum）
  - `tubi_relate:[pe:⟨bran_refno⟩, index]`（复合 ID；其中 bran_refno 取值为 ref0_ref1，下划线连接；id[0] 为 BRAN/HANG 的 pe_key）
  - `scene_node:⟨refno_u64⟩`、`contains:[parent_u64, child_u64]`
  - 若需 `dbnum`：须用 `DbMetaManager` 由 `ref0 -> dbnum` 映射取得（勿把 ref0 当 dbnum）
- **复合 ID 取值**：优先 `id[0]`/`id[1]`，少用 `record::id(id)`（额外解析开销）
- **范围查询**：对 `tubi_relate` 一律用 `table:[start]..[end]` 的 ID Range
- **只取值**：能用 `SELECT VALUE ...` 就别 `SELECT *`
- **IDs 数组查询**：若已得 record id 数组（如一批 `pe:⟨...⟩`），一律用 `FROM ids` 点查；勿用 `WHERE ... IN ids` 触发表扫描
  - 例：`SELECT * FROM [pe:⟨21491_10000⟩, pe:⟨21491_10001⟩];`
  - 例：`LET $ids = [pe:⟨21491_10000⟩, pe:⟨21491_10001⟩]; SELECT * FROM $ids;`
- **children 顺序**：`pe.children` 为有序数组，其顺序即先后顺序；查询 children 时不要再 `ORDER BY`/额外排序（无益且增开销，亦可能破坏顺序）
- **关系表写入（INSERT）**：写 relation table 时，优先按官方语法使用 `INSERT RELATION INTO <table> ...`（见：SurrealQL `INSERT` 文档）
  - ✅ 单条：`INSERT RELATION INTO geo_relate { in: inst_info:..., id: 'stable_id', out: inst_geo:..., ... };`
  - ✅ 批量：`INSERT RELATION INTO geo_relate [ { in: ..., id: 'a', out: ... }, { in: ..., id: 'b', out: ... } ];`
  - ⚠️ 说明：`INSERT RELATION <table>:<id> CONTENT { ... }` 属于文档允许的写法；但在我们当前 SurrealDB 版本/解析器组合下，曾出现对 `CONTENT` 分支的兼容性问题（同一 SQL 在不同版本表现不一致）。为避免线上/联调环境差异，统一采用 `INSERT RELATION INTO ...` 路径。
- **replace / 复写注意**：`inst_geo` 若用 `INSERT IGNORE`，则同 id 的旧记录不会被覆盖（包括 `unit_flag`/`param`）
  - ✅ 需要强制重建/切换 unit_flag 时：先点删再插入：`DELETE [inst_geo:123, inst_geo:456]; INSERT IGNORE INTO inst_geo [{...},{...}];`
  - ✅ 典型排错：`--regen-model` 后仍读到旧 `unit_flag=false`，先查 `SELECT id, unit_flag FROM inst_geo:123;` 再确认是否被 IGNORE 了

## Rust 侧查询约定

- **结果类型**：必须用 `SurrealValue`（禁止 `serde_json::Value`）
- **执行入口**：统一走 `SurrealQueryExt`
  - 单语句：`SUL_DB.query_take(sql, 0).await?`
  - 多语句：`let mut resp = SUL_DB.query_response(sql).await?; resp.take::<T>(i)?;`
- **ID 字段**：优先 `RefnoEnum` / `RefU64`；可选用 `Option<RefnoEnum>`

## 常用查询配方（按需套用）

- **查 pe 基本信息**
  - 按 noun/dbnum 过滤，避免扫全表；deleted/逻辑删除按项目约定带上过滤条件
- **查层级**
  - 子节点/子孙/祖先：优先走 TreeIndex API（性能、稳定性更好）
  - 若必须用 SurrealDB：用 `->pe_owner`/`<-pe_owner` 或递归路径（见 references）
- **查几何实例链路**
  - 先批量收集 refnos，再 `WHERE ir.in IN $refnos`（减少往返与重复查询）
  - 导出几何时注意 `geo_type`/`visible` 等过滤条件（见 references）
- **查 tubi_relate（管道直段）**
  - 用 ID Range：`tubi_relate:[pe:⟨bran_refno⟩, 0]..[pe:⟨bran_refno⟩, ..]`（其中 bran_refno 取值为 ref0_ref1，下划线连接）

## 排错与性能检查清单

- 查询慢：先检查是否误用递归/全表扫描；层级优先 TreeIndex/SceneTree；tubi_relate 必须 ID Range
- 结果空：先确认 record id 形制与 refno 编码；再核对 deleted/visible/geo_type 等过滤条件
- 类型反序列化失败：检查结构体字段类型与 `serde(alias="id")` 等映射；确保 derive 了 `SurrealValue`

## 参考资料（按需加载）

- `references/数据库查询总结.md`：总览（架构、语法、Record ID、图遍历、递归、RELATE、TreeIndex、批量优化、Rust API）
- `references/数据库架构.md`：表结构详解（pe/inst_* / geo_relate / tubi_relate 等）
- `references/常用查询方法.md`：Rust 侧查询封装与常用函数入口
- `references/SurrealDB函数参考.md`：`fn::*` 自定义函数一览与用法
- `references/tubi_relate查询指南.md`：tubi_relate 复合 ID、ID Range、写入 RELATE
- `references/SceneTree架构.md`：scene_tree（scene_node/contains）表与 API

## 代码定位（需要时再做）

当你要把 SurrealQL 落到 Rust 封装/修复性能时：用 `ace-tool.search_context` 在对应仓库内找这些关键词：

- `init_model_tables`（表结构初始化）
- `SurrealQueryExt` / `query_take` / `query_response`
- `collect_children_filter_ids` / `collect_descendant_filter_ids` / `query_filter_ancestors`
- `tubi_relate` / `inst_relate` / `geo_relate` / `neg_relate`
- `scene_tree` / `scene_node` / `contains`
