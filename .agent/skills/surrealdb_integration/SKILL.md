---
name: SurrealDB Integration
description: Guide and best practices for using SurrealDB 3.0 with rs-core/gen-model-fork. Covers database schema, query patterns, TreeIndex, and performance optimization.
---

# SurrealDB 3.0 Integration Skills for rs-core/gen-model-fork

这是针对 rs-core (aios_core) 项目的 SurrealDB 数据库查询和架构知识库。

---

## 核心原则

### 1. 查询优先级
- **层级查询**：优先使用 TreeIndex（`collect_*` 系列函数），性能提升 10-100 倍
- **属性关系**：使用 SurrealDB 图遍历（`->GMRE`, `->LSTU->CATR` 等）
- **批量查询**：使用数据库端函数（`fn::*`）和 `array::map` 模式，避免循环查询

### 2. 类型安全规范
- **必须使用** `SurrealValue` trait，禁止使用 `serde_json::Value`
- ID 字段使用 `RefnoEnum` 或 `RefU64`（已兼容自动转换）
- 时间戳使用 `surrealdb::types::Datetime`
- 使用 `#[serde(alias = "id")]` 处理字段别名

### 3. 性能优化原则
- 批量查询优于循环查询（性能提升 N 倍）
- 使用 ID Range 替代 WHERE 条件（索引查询）
- 限制递归深度（如 `Some("1..5")`），避免无限递归
- 使用 `array::distinct()` 去重（SurrealDB 不支持 `SELECT DISTINCT`）
- 直接访问字段，避免 `record::id()` 函数调用
- **避免过度防御**：TreeIndex 已保证类型，批量查询时无需再用 noun 过滤（主键查询最快）

---

## 1. Core Architecture & Setup

The project uses a custom fork of `surrealdb` (likely v3.0 compatible via `happydpc/surrealdb`).

-   **Global Client**: Accessed via `aios_core::SUL_DB`.
-   **Trait**: `aios_core::SurrealQueryExt` extends the client with helper methods (`query_take`, `query_response`).
-   **Data Types**:
    -   **`RefnoEnum`**: Primary key type, automatically converts to/from SurrealDB Record IDs (e.g., `pe:⟨123_456⟩`).
    -   **`SurrealValue`**: Derivable trait for structs mapping to DB results. **Do not use `serde_json::Value`**.

## 2. Basic Query Syntax (Surql 3.0)

### 2.1 SELECT & filtering
```sql
-- Select specific fields
SELECT refno, noun FROM pe WHERE deleted = false;

-- Select raw value (array of IDs)
SELECT VALUE id FROM pe WHERE noun IN ['BOX', 'CYLI'];

-- Select from specific records
SELECT * FROM [pe:⟨123_456⟩, pe:⟨789_000⟩];
```

### 2.2 Record IDs (Critical)
The project uses specific ID formats. Always use underscore `_` for complex refnos, not slash `/`.

-   **Format**: `table:⟨id_content⟩` (e.g., `pe:⟨12345_67890⟩`)
-   **Rust Helper**: `refno.to_pe_key()` generates the formatted ID string.
-   **Range Query**: `table:[start]..[end]` is preferred over `WHERE record::id() > ...`.

### 2.3 Graph Traversal (`->` / `<-`)
SurrealDB's graph navigation is heavily used.

```sql
-- Outgoing: "Has relation to..."
SELECT * FROM pe:⟨123⟩->inst_relate;

-- Incoming: "Is referenced by..."
SELECT VALUE in FROM pe:⟨123⟩<-pe_owner; -- Get children (entities that own this as parent)
```

## 3. Advanced Features (SurrealDB 3.0)

### 3.1 Recursive Path Queries (`@.{range}.field`)
This is the modern way to query deep hierarchies (descendants).

**Syntax**: `@.{RANGE + OPTIONS}.FIELD`

-   **Range**:
    -   `..` (Infinite)
    -   `3` (Exactly 3 levels)
    -   `1..5` (Levels 1 to 5)
-   **Options**:
    -   `collect` (Gather results into a flat array)
    -   `inclusive` (Include the starting node)

**Example**: Get all descendants recursively:
```sql
SELECT VALUE array::flatten(@.{..+collect}.children) FROM ONLY $root;
```

### 3.2 Custom Database Functions (`fn::`)
The project defines server-side functions to optimize performance (avoiding network roundtrips).

| Function | Purpose |
| :--- | :--- |
| `fn::collect_descendant_ids_by_types($root, $types, $inclusive, $range)` | Get descendant IDs recursively. |
| `fn::visible_geo_descendants($root, $inclusive, $range)` | Get descendants that have visible geometry. |
| `fn::ancestor($pe)` | Get the ancestor of a node. |
| `fn::collect_descendants_filter_inst(...)` | Filter descendants based on existing relations. |

**Usage in Rust**:
```rust
let sql = format!("fn::collect_descendant_ids_by_types({}, ['BOX'], true, '..')", pe_key);
```

## 4. Rust Integration Patterns

### 4.1 Basic Query Execution
```rust
use aios_core::{SUL_DB, SurrealQueryExt};

// Return a generic Vector of structs
let result: Vec<MyStruct> = SUL_DB.query_take(&sql, 0).await?;

// MyStruct definition
#[derive(Serialize, Deserialize, SurrealValue)]
struct MyStruct {
    id: RefnoEnum,
    name: String
}
```

### 4.2 Generic Recursive Helpers (Recommended)
Use `collect_descendant_with_expr` for flexibility.

```rust
// fetch ID only
let ids: Vec<RefnoEnum> = collect_descendant_with_expr(
    &[root_refno], 
    &["BOX", "CYLI"], // Filter types
    Some("1..5"),     // Depth range
    "VALUE id"        // Select expression
).await?;

// fetch Full Attributes
let attrs: Vec<NamedAttrMap> = collect_descendant_with_expr(
    &[root_refno], 
    &[], 
    None, 
    "VALUE id.refno.*" // Fetch nested object
).await?;
```

### 4.3 Batching Strategy (Performance)
**Never** loop over IDs to query individually. Use `array::map` inside the query.

**Bad (Loop in Rust)**:
```rust
for refno in refnos {
    SUL_DB.query("..."); // N network calls
}
```

**Good (Batch in SQL)**:
```rust
let sql = format!(
    "array::map([{}], |$id| fn::some_function($id))", 
    refnos.join(",")
);
// 1 network call
```

## 5. Migration / Version 3.0 Checklist
1.  **Strict Typing**: Ensure `SurrealValue` is used, not `serde_json::Value`.
2.  **Recursive Syntax**: deprecate old manual graph traversals in favor of `@.{..}` where possible.
3.  **Range Queries**: Use ID ranges `table:[min]..[max]` for scanning specific partitions (like `dbnum`).

## 6. Cheatsheet: Common Tasks

| Task | Pattern |
| :--- | :--- |
| **Get Children** | `collect_children_filter_ids(refno, &types)` |
| **Get Ancestors** | `query_filter_ancestors(refno, &types)` |
| **Get Visible Geometry** | `query_visible_geo_descendants(refno, ...)` |
| **Check Existence** | `WHERE count(SELECT ... LIMIT 1) > 0` (Don't use `count()` on full set) |
| **Deduplicate** | `array::distinct(...)` (No `SELECT DISTINCT`) |

---

## 数据库架构速查

### 核心表结构

```
pe (元素主表) - 统一存储所有工程元素
├─ pe_owner (层级关系) - child -> parent 父子关系
├─ inst_relate (几何实例) → inst_info → geo_relate → inst_geo
├─ inst_relate_aabb (包围盒关系)
├─ tubi_relate (管道直段) - 复合 ID: [bran_pe, index]
├─ neg_relate (负实体关系)
├─ ngmr_relate (NGMR 负实体)
└─ tag_name_mapping (位号映射)
```

### PE 表（核心存储表）
- **ID 格式**: `pe:⟨dbnum_refno⟩` 例如 `pe:⟨21491_10000⟩`
- **关键字段**: id (RefnoEnum), noun (元素类型), name, owner, children, deleted, sesno, dbnum

### pe_owner 关系表（层级关系）
- **关系方向**: `child (in) -[pe_owner]-> parent (out)`
- **⚠️ 推荐**: 层级查询使用 TreeIndex，性能提升 100 倍

### geo_relate 表的 geo_type 字段
| geo_type | 含义 | 是否导出 |
|----------|------|----------|
| `Pos` | 原始几何（未布尔运算） | ✅ 导出 |
| `DesiPos` | 设计位置 | ✅ 导出 |
| `CatePos` | 布尔运算后的结果 | ✅ 导出 |
| `Compound` | 组合几何体（包含负实体引用） | ❌ 不导出 |
| `CateNeg` | 负实体 | ❌ 不导出 |
| `CataCrossNeg` | 交叉负实体 | ❌ 不导出 |

**导出条件**: `geo_type IN ['Pos', 'DesiPos', 'CatePos']`

---

## TreeIndex 使用指南

### 何时使用 TreeIndex
- ✅ **层级查询**（子节点、子孙节点、祖先节点）
- ❌ **属性关系**（GMRE、GSTR、LSTU、CATR 等）- 仍需使用 SurrealDB

### 性能对比
| 场景 | SurrealDB 递归 | TreeIndex | 性能提升 |
|------|---------------|-----------|----------|
| 查询 1000 个节点的子孙（10 层） | ~500ms | ~5ms | **100 倍** |
| 查询单层子节点（100 个） | ~50ms | ~0.5ms | **100 倍** |
| 查询祖先（5 层） | ~30ms | ~0.3ms | **100 倍** |
| 批量查询（10 个根节点） | ~5s | ~50ms | **100 倍** |

### 迁移建议
| 旧方式（SurrealDB） | 新方式（TreeIndex） | 性能提升 |
|-------------------|-------------------|----------|
| `SELECT VALUE in FROM pe:⟨refno⟩<-pe_owner` | `collect_children_filter_ids(refno, &[])` | **100 倍** |
| `SELECT VALUE array::flatten(@.{..+collect}.children)` | `collect_descendant_filter_ids(&[refno], &[], None)` | **100 倍** |
| `SELECT VALUE out FROM pe:⟨refno⟩->pe_owner` | `query_filter_ancestors(refno, &[])` | **100 倍** |

---

## 快速决策树

### 我需要查询层级关系？
- **是** → 使用 TreeIndex（`collect_children_filter_ids`, `collect_descendant_filter_ids`, `query_filter_ancestors`）

### 我需要查询属性关系？
- **是** → 使用 SurrealDB 图遍历（`query_single_by_paths`）

### 我需要批量查询多个节点？
- **是** → 使用 `array::map` + 数据库端函数（`fn::*`）

### 我需要查询几何实例？
- **是** → 使用封装函数: `query_insts`, `query_tubi_insts_by_brans`, `query_insts_by_zone`

---

## 代码位置参考

### 核心模块（rs-core）
- **查询扩展**: `src/rs_surreal/query_ext.rs` - SurrealQueryExt trait
- **实例查询**: `src/rs_surreal/inst.rs` - query_insts, query_tubi_insts_by_brans
- **层级查询**: `src/rs_surreal/graph.rs` - collect_descendant_*, collect_children_*
- **PE 查询**: `src/rs_surreal/query.rs` - get_pe, get_named_attmap
- **结构体定义**: `src/rs_surreal/inst_structs.rs` - GeomInstQuery, TubiInstQuery
- **数据库函数定义**: `resource/surreal/common.surql`

## 7. File References
-   **Query Helpers**: `rs-core/src/rs_surreal/query_ext.rs`
-   **Graph Logic**: `rs-core/src/rs_surreal/graph.rs`
-   **DB Functions**: `rs-core/resource/surreal/common.surql`
