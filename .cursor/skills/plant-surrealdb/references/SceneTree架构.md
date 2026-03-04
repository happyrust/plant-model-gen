# Scene Tree 数据库架构

本文档描述了 `gen-model-fork` 中 Scene Tree 模块的 SurrealDB 数据库架构。

> **模块位置**: `src/scene_tree/`

---

## 概述

Scene Tree 是一个用于管理模型生成状态和场景层级关系的数据库模块，用于替代传统的 `inst_relate_aabb` 表。主要功能包括：

- **场景层级管理**：维护从 WORLD 到叶子节点的完整层级结构
- **生成状态追踪**：记录每个节点的模型是否已生成
- **几何节点标识**：标记哪些节点包含几何数据

---

## 核心表概览

| 表名 | 类型 | 说明 |
|------|------|------|
| `scene_node` | 节点表 | 场景树节点，存储节点状态和属性 |
| `contains` | 关系表 | 父子层级关系（Graph Edge） |

---

## 表结构详解

### scene_node 表（场景节点）

存储场景树中每个节点的状态信息。

```sql
-- 表定义
DEFINE TABLE IF NOT EXISTS scene_node SCHEMAFULL;

-- 字段定义
DEFINE FIELD IF NOT EXISTS parent ON TABLE scene_node TYPE option<int>;           -- 父节点 refno (u64 编码)
DEFINE FIELD IF NOT EXISTS aabb ON TABLE scene_node TYPE option<record<aabb>>;    -- 包围盒引用
DEFINE FIELD IF NOT EXISTS has_geo ON TABLE scene_node TYPE bool DEFAULT false;  -- 是否为几何节点
DEFINE FIELD IF NOT EXISTS is_leaf ON TABLE scene_node TYPE bool DEFAULT false;  -- 是否为叶子节点
DEFINE FIELD IF NOT EXISTS generated ON TABLE scene_node TYPE bool DEFAULT false; -- 模型是否已生成
DEFINE FIELD IF NOT EXISTS dbno ON TABLE scene_node TYPE int;                     -- 数据库编号

-- 索引定义
DEFINE INDEX IF NOT EXISTS idx_parent ON TABLE scene_node COLUMNS parent;
DEFINE INDEX IF NOT EXISTS idx_has_geo ON TABLE scene_node COLUMNS has_geo;
DEFINE INDEX IF NOT EXISTS idx_is_leaf ON TABLE scene_node COLUMNS is_leaf;
DEFINE INDEX IF NOT EXISTS idx_dbno ON TABLE scene_node COLUMNS dbno;
DEFINE INDEX IF NOT EXISTS idx_generated ON TABLE scene_node COLUMNS generated;
DEFINE INDEX IF NOT EXISTS idx_has_geo_generated ON TABLE scene_node COLUMNS has_geo, generated;
```

**Rust 结构体**：

```rust
// ID 格式: scene_node:⟨refno_u64⟩ 例如 scene_node:⟨104679055498⟩
#[derive(Debug, Serialize, Deserialize)]
pub struct SceneNodeStatus {
    pub id: i64,           // refno 的 u64 编码
    pub has_geo: bool,     // 是否为几何节点（根据 noun 判断）
    pub generated: bool,   // 模型是否已生成
}

// 内部使用的节点数据结构
struct SceneNodeData {
    id: i64,               // refno 的 u64 编码
    parent: Option<i64>,   // 父节点 refno
    has_geo: bool,         // 是否为几何节点
    is_leaf: bool,         // 是否为叶子节点（无子节点）
    dbno: i16,             // 数据库编号
}
```

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | `i64` | 节点 ID，格式为 `scene_node:⟨refno_u64⟩` |
| `parent` | `Option<i64>` | 父节点的 refno，WORLD 节点为 `NONE` |
| `aabb` | `Option<record<aabb>>` | 包围盒引用，关联 `aabb` 表 |
| `has_geo` | `bool` | 是否为几何节点（根据 noun 类型判断） |
| `is_leaf` | `bool` | 是否为叶子节点（无任何子节点） |
| `generated` | `bool` | 模型是否已生成 |
| `dbno` | `int` | 数据库编号 |

---

### contains 表（层级关系）

存储场景树的父子层级关系，使用 SurrealDB 的图关系类型。

```sql
-- 关系表定义（Graph Edge）
DEFINE TABLE IF NOT EXISTS contains TYPE RELATION FROM scene_node TO scene_node;
```

**关系方向**：`scene_node:⟨parent⟩ -> contains:[parent_id, child_id] -> scene_node:⟨child⟩`

**ID 格式**：`contains:[parent_refno_u64, child_refno_u64]`

**创建示例**：

```sql
-- 创建父子关系
RELATE scene_node:104679055498 -> contains:[104679055498, 104679055499] -> scene_node:104679055499;
```

---

## 几何节点判断逻辑

节点是否为几何节点（`has_geo`）由其 `noun` 类型决定：

```rust
pub fn is_geo_noun(noun: &str) -> bool {
    let noun_upper = noun.to_uppercase();
    let noun_str = noun_upper.as_str();

    USE_CATE_NOUN_NAMES.contains(&noun_str)           // 使用 CATE 的节点类型
        || GNERAL_LOOP_OWNER_NOUN_NAMES.contains(&noun_str)  // 环形结构所有者
        || GNERAL_PRIM_NOUN_NAMES.contains(&noun_str)        // 通用基元类型
        || BRAN_COMPONENT_NOUN_NAMES.contains(&noun_str)     // BRAN 组件类型
        || noun_str == "BRAN"                                // 管道分支
        || noun_str == "HANG"                                // 吊点
}
```

---

## API 函数

### 初始化函数

| 函数 | 说明 |
|------|------|
| `init_schema()` | 初始化 Scene Tree 表结构（创建表、字段、索引） |
| `init_scene_tree(mdb_name, force_rebuild)` | 从 WORLD 节点开始构建整棵场景树 |
| `init_scene_tree_from_root(root_refno, force_rebuild)` | 从指定根节点构建子树 |
| `init_scene_tree_by_dbno(dbno, force_rebuild)` | 按数据库编号初始化（WORLD 为 `${dbno}_0`） |

**初始化结果**：

```rust
pub struct SceneTreeInitResult {
    pub node_count: usize,      // 节点数量
    pub relation_count: usize,  // 关系数量
    pub duration_ms: u128,      // 耗时（毫秒）
}
```

### 查询函数

| 函数 | 说明 |
|------|------|
| `query_generation_status(refnos)` | 批量查询 refnos 的生成状态 |
| `filter_ungenerated_geo_nodes(refnos)` | 从 refnos 中过滤出未生成的几何节点 |
| `query_ungenerated_leaves(root_id)` | 查询指定节点下所有未生成的几何叶子节点 |
| `query_children_ids(parent_id, limit)` | 查询指定节点的直属子节点 |
| `query_ancestor_ids(start_id, limit)` | 查询指定节点的祖先链（从直接父节点到根） |
| `query_generated_refnos(refnos)` | 查询已生成的节点（替代 inst_relate_aabb 存在性检查） |

### 更新函数

| 函数 | 说明 |
|------|------|
| `mark_as_generated(ids)` | 批量标记节点为已生成 |
| `update_scene_node_aabb(inst_aabb_map)` | 批量更新节点的 AABB 并标记为已生成 |

---

## 常用查询示例

### 1. 查询节点生成状态

```sql
-- 批量查询节点状态
SELECT VALUE { id: record::id(id), has_geo: has_geo, generated: generated } 
FROM [scene_node:104679055498, scene_node:104679055499]
```

### 2. 过滤未生成的几何节点

```sql
-- 从列表中筛选未生成的几何节点
SELECT VALUE record::id(id) 
FROM [scene_node:104679055498, scene_node:104679055499] 
WHERE has_geo = true AND generated = false
```

### 3. 查询子节点

```sql
-- 查询直属子节点
SELECT VALUE record::id(out) 
FROM contains 
WHERE in = scene_node:104679055498 
LIMIT 1000
```

### 4. 查询父节点

```sql
-- 查询父节点 refno
SELECT VALUE parent 
FROM scene_node:104679055498 
LIMIT 1
```

### 5. 标记为已生成

```sql
-- 批量更新生成状态
UPDATE [scene_node:104679055498, scene_node:104679055499] 
SET generated = true
```

### 6. 更新 AABB 并标记生成

```sql
-- 更新 AABB 引用并标记为已生成
UPDATE scene_node:104679055498 
SET aabb = aabb:⟨hash_value⟩, generated = true
```

### 7. 按 dbno 清理数据

```sql
-- 清理指定数据库编号的数据
DELETE contains WHERE in.dbno = 24383 OR out.dbno = 24383;
DELETE scene_node WHERE dbno = 24383;
```

---

## 表关系图

```
                         ┌─────────────────┐
                         │   scene_node    │
                         │  (场景树节点)   │
                         └────────┬────────┘
                                  │
                                  │ parent (向上引用)
                                  │
            ┌─────────────────────┼─────────────────────┐
            │                     │                     │
            ▼                     ▼                     ▼
     ┌─────────────┐       ┌─────────────┐       ┌─────────────┐
     │ scene_node  │       │ scene_node  │       │ scene_node  │
     │   (子节点)  │       │   (子节点)  │       │   (子节点)  │
     └──────┬──────┘       └─────────────┘       └─────────────┘
            │
            │ contains (图关系)
            ▼
     ┌─────────────┐
     │   contains  │  ───> scene_node:child
     │ (关系边)    │
     └─────────────┘

     ┌─────────────┐
     │    aabb     │  <─── scene_node.aabb 引用
     │ (包围盒)    │
     └─────────────┘
```

---

## 与其他表的关联

| 关联表 | 关联方式 | 说明 |
|--------|----------|------|
| `pe` | `scene_node.id` ↔ `pe:⟨refno⟩` | scene_node ID 与 pe 表 ID 共享相同的 refno 编码 |
| `aabb` | `scene_node.aabb` → `aabb:⟨hash⟩` | 包围盒引用 |
| `inst_relate` | 替代关系 | scene_node 替代了原有的 inst_relate_aabb 功能 |

---

## 相关代码位置

### 模块文件

| 文件 | 说明 |
|------|------|
| `src/scene_tree/mod.rs` | 模块入口，导出公共 API |
| `src/scene_tree/schema.rs` | 表结构定义（Schema） |
| `src/scene_tree/init.rs` | 初始化逻辑（BFS 构建树） |
| `src/scene_tree/query.rs` | 查询方法（状态查询、AABB 更新） |

### Web API

| 文件 | 说明 |
|------|------|
| `src/web_api/scene_tree_api.rs` | Scene Tree REST API 路由 |

### 测试

| 文件 | 说明 |
|------|------|
| `src/test/test_scene_tree.rs` | 完整功能测试 |
| `src/test/test_scene_tree_simple.rs` | 简化单元测试 |
| `src/bin/scene_tree_smoke.rs` | 冒烟测试脚本 |

---

## 性能优化

1. **复合索引**：`idx_has_geo_generated` 用于加速"未生成的几何节点"查询
2. **批量操作**：所有写入操作使用 `CHUNK_SIZE = 200` 的批量处理
3. **BFS 遍历**：树遍历限制最大深度为 20 层，避免无限递归
4. **分块查询**：大量 ID 查询时分 500/2000 个一组进行
