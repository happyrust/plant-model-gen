# 房间计算 SurrealDB 优化分析

> 基于 plant-surrealdb skill 对房间计算相关 SurrealQL 与 Rust 查询的优化建议。

---

## 1. spatial_query_api 字段名错误（高优先级）

**位置**：[`plant-model-gen/src/web_api/spatial_query_api.rs`](plant-model-gen/src/web_api/spatial_query_api.rs) L275、L284

**问题**：`room_panel_relate` 和 `room_relate` 使用 `owner` 字段查询，但 SurrealDB 关系表标准字段为 `in` / `out`。

```sql
-- 当前（可能错误）
SELECT refno FROM room_panel_relate WHERE owner = {}
SELECT refno FROM room_relate WHERE owner = {}
```

**关系表结构**：
- `room_panel_relate`：`out`=房间(FRMW)，`in`=面板(PANE)
- `room_relate`：`in`=面板(PANE)，`out`=构件(EQUI/BRAN...)

**建议**：
- FRMW 的子（通过 room_panel_relate）：`WHERE out = {}`（out 为房间）
- PANE 的子（通过 room_relate）：`WHERE in = {}`（in 为面板，查 out 即构件）

需确认项目中 `owner` 是否为 `out` 的别名或自定义字段；若否，应改为 `out` / `in`。

---

## 2. fn::room_code 层级查询可考虑 TreeIndex

**位置**：`rs-core/resource/surreal/fn_query_room_code_hh.surql`、`fn_query_room_code.surql`

**问题**：`fn::room_code` 通过 `$pe<-pe_owner.in.id`、`$pe<-pe_owner.in<-pe_owner.in.id` 等递归遍历层级，属于 SurrealDB 递归图遍历。

**plant-surrealdb 约定**：层级查询优先 TreeIndex，性能通常高两个数量级。

**限制**：`fn::room_code` 为 SurrealDB 端函数，无法直接调用 TreeIndex。

**建议**：
- **方案 A**：在 Rust 侧（房间计算或材料表生成时）预计算 room_code，写入 pe 或单独表，fn::room_code 改为查缓存
- **方案 B**：保持现状，若 fn::room_code 调用频率不高（如材料表批量），可接受
- **方案 C**：若需在 SurrealDB 内频繁调用，考虑在导入时预建 room_code 字段

---

## 3. query_room_panels_by_keywords 层级遍历

**位置**：`rs-core/src/room/query_v2.rs` 的 `query_room_panels_by_keywords`

**当前 SQL**：
```sql
array::flatten([REFNO<-pe_owner<-pe, REFNO<-pe_owner<-pe<-pe_owner<-pe])[?noun='PANE']
```

**问题**：通过 pe_owner 递归遍历获取 PANE 子节点，层级深时可能较慢。

**建议**：
- 若已有 TreeIndex，可在 Rust 侧用 `collect_descendant_filter_ids(room_refno, &["PANE"], None)` 替代
- 需在 `build_room_panels_relate` 调用前确保 TreeIndex 已加载

---

## 4. 关系表写入：RELATE vs INSERT RELATION

**位置**：`room_model.rs` 的 `create_room_panel_relations_batch`、`save_room_relate`

**当前**：使用 `RELATE ... -> ... -> ... SET` 语法

**plant-surrealdb 约定**：关系表写入优先 `INSERT RELATION INTO <table> ...`，避免 CONTENT 分支兼容性差异。

**建议**：当前 RELATE 语法为 SurrealDB 标准写法，若运行稳定可暂不修改。若遇版本兼容问题，可改为：
```sql
INSERT RELATION INTO room_panel_relate [ { in: panel:..., out: room:..., room_num: '...' }, ... ];
```

---

## 5. 索引与查询优化

**已有**：`room_relate`、`room_panel_relate` 均有 `(in, out) UNIQUE` 索引（`define_room_index`）。

**建议**：
- 若常按 `room_num` 过滤，可加 `DEFINE INDEX idx_room_relate_room_num ON TABLE room_relate COLUMNS room_num`
- 若常按 `out` 查 room_relate（构件→房间），现有 `(in, out)` 索引对 `WHERE out = ?` 可能不覆盖，可评估 `COLUMNS out` 索引

---

## 6. 查询结果类型

**plant-surrealdb 约定**：Rust 侧查询必须用 `SurrealValue`，禁止 `serde_json::Value`。

**检查**：房间相关 `query_take` / `query_response` 已使用强类型（如 `Vec<RoomInfo>`、`RecordId`），符合约定。

---

## 7. 批量查询

**plant-surrealdb 约定**：若已得 record id 数组，用 `FROM [ids]` 点查，勿用 `WHERE ... IN ids` 触发表扫描。

**当前**：`room_panel_relate` 批量写入使用 `relate room->room_panel_relate->[panel1, panel2, ...]`，符合批量语义。

**材料表 room_code**：`($pe<-room_relate.room_num)[0]` 为单条点查，合理。

---

## 8. 优化建议汇总

| 优先级 | 项目 | 建议 |
|--------|------|------|
| 高 | spatial_query_api owner 字段 | 确认 room_panel_relate/room_relate 的 owner 是否为 out/in，必要时改为 in/out |
| 中 | fn::room_code 层级 | 评估预计算 room_code 缓存，减少 pe_owner 递归 |
| 中 | query_room_panels_by_keywords | 评估 TreeIndex 替代 pe_owner 递归 |
| 低 | room_num 索引 | 若按 room_num 过滤频繁，可加索引 |
| 低 | RELATE 语法 | 若遇兼容问题再考虑 INSERT RELATION |

---

## 9. 参考

- plant-surrealdb skill
- `rs-core/CLAUDE.md` 数据库架构
- `docs/房间计算流程分析.md`
