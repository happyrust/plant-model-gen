# Scene Tree 开发方案

## 1. 概述

### 1.1 背景

当前模型流式加载时，前端需要调用 `query_insts` 从 SurrealDB 查询模型数据，增加了网络往返开销。通过引入 Scene Tree，可以：

1. 后端直接返回渲染所需的模型数据
2. 提供高效的层级查询（查询任意节点下的所有几何叶子节点）
3. 支持 AABB 的层级聚合更新

### 1.2 目标

- 构建与 E3D Tree 一致的场景树结构
- 支持 O(1) 的 pe ↔ scene_node 双向转换
- 支持高效的递归查询（SurrealDB Graph 特性）
- 支持 AABB 的增量更新

---

## 2. 数据结构设计

### 2.1 Scene Node 表

```sql
DEFINE TABLE scene_node SCHEMAFULL;

-- 主键：refno 的 u64 编码值（通过 parse::uint64 转换）
-- 例如 scene_node:104679055498 对应 pe:⟨24383_73962⟩

-- 父节点 refno (u64 编码)
DEFINE FIELD parent ON TABLE scene_node TYPE option<int>;

-- 包围盒引用
DEFINE FIELD aabb ON TABLE scene_node TYPE option<record<aabb>>;

-- 是否为几何节点（根据 noun 类型判断）
DEFINE FIELD has_geo ON TABLE scene_node TYPE bool DEFAULT false;

-- 是否为叶子节点（叶子节点定义：无任何子节点）
-- 注意：has_geo=true 并不代表叶子节点
DEFINE FIELD is_leaf ON TABLE scene_node TYPE bool DEFAULT false;

-- 模型是否已生成
DEFINE FIELD generated ON TABLE scene_node TYPE bool DEFAULT false;

-- 数据库编号
DEFINE FIELD dbno ON TABLE scene_node TYPE int;

-- 索引
DEFINE INDEX idx_parent ON TABLE scene_node COLUMNS parent;
DEFINE INDEX idx_has_geo ON TABLE scene_node COLUMNS has_geo;
DEFINE INDEX idx_is_leaf ON TABLE scene_node COLUMNS is_leaf;
DEFINE INDEX idx_dbno ON TABLE scene_node COLUMNS dbno;
DEFINE INDEX idx_generated ON TABLE scene_node COLUMNS generated;
DEFINE INDEX idx_has_geo_generated ON TABLE scene_node COLUMNS has_geo, generated;
```

### 2.2 父子关系表（Graph Edge）

```sql
-- 使用 SurrealDB 的 RELATION 类型
DEFINE TABLE contains TYPE RELATION FROM scene_node TO scene_node;
```

### 2.3 与 PE 表的关联

通过 `parse::uint64` 模块实现 O(1) 双向转换：

| 方向 | 函数 | 示例 |
|-----|------|-----|
| pe → scene_node | `parse::uint64::pair_to_u64` | `"24383_73962"` → `104679055498` |
| scene_node → pe | `parse::uint64::u64_to_pair_str` | `104679055498` → `"24383_73962"` |

---

## 3. 层级结构

### 3.1 E3D 层级映射

```
WORLD (根节点，每个 MDB 一个)
  └── SITE (站点)
        └── ZONE (区域)
              ├── PIPE (管道系统)
              │     └── BRAN [has_geo=true]
              │           └── ELBO [has_geo=true]
              ├── EQUI (设备)
              │     └── NOZZ [has_geo=true]
              └── STRU (结构)
                    └── PANE [has_geo=true]
```

### 3.2 几何节点判断

根据 `aios_core::pdms_types` 中的常量判断 `has_geo`：

| 类别 | 常量 | 示例类型 |
|-----|------|---------|
| Cate | `USE_CATE_NOUN_NAMES` | NOZZ, VALV, ELBO, TEE, FLAN... |
| Loop | `GNERAL_LOOP_OWNER_NOUN_NAMES` | BRAN, HANG, GENSEC, SCTN... |
| Prim | `GNERAL_PRIM_NOUN_NAMES` | BOX, CYL, CONE, SPHER, TORUS... |

```rust
fn is_geo_noun(noun: &str) -> bool {
    let noun_upper = noun.to_uppercase();
    let noun_str = noun_upper.as_str();

    USE_CATE_NOUN_NAMES.contains(&noun_str)
        || GNERAL_LOOP_OWNER_NOUN_NAMES.contains(&noun_str)
        || GNERAL_PRIM_NOUN_NAMES.contains(&noun_str)
}
```

---

## 4. 初始化流程

### 4.1 流程图

```
┌────────────────────────────────────────┐
│ 1. 获取 MDB 的 WORLD 节点              │
│    aios_core::mdb::get_world_refno()   │
└────────────────────────────────────────┘
                │
                ▼
┌────────────────────────────────────────┐
│ 2. 创建 WORLD 作为根节点               │
│    parent = NULL                       │
└────────────────────────────────────────┘
                │
                ▼
┌────────────────────────────────────────┐
│ 3. 从 WORLD 递归遍历子节点 (BFS)       │
│    get_children_refnos(refno)          │
└────────────────────────────────────────┘
                │
                ▼
┌────────────────────────────────────────┐
│ 4. 判断 has_geo (根据 noun 类型)       │
│    is_geo_noun(noun)                   │
└────────────────────────────────────────┘
                │
                ▼
┌────────────────────────────────────────┐
│ 5. 批量写入 scene_node + contains      │
└────────────────────────────────────────┘
```

### 4.2 初始化入口

```rust
/// 初始化 Scene Tree（从 WORLD 开始）
pub async fn init_scene_tree(mdb_name: &str, force_rebuild: bool) -> Result<SceneTreeInitResult> {
    // 1. 获取 WORLD 节点
    let world = aios_core::mdb::get_world_refno(mdb_name.to_string()).await?;
    let world_refno = world.refno();

    // 2. 可选重建：清理旧数据
    // force_rebuild=true 时，先清理 scene_node/contains，确保幂等与数据一致

    // 2. 从 WORLD 开始构建整棵树
    let (nodes, relations) = build_tree_from_world(world_refno).await?;

    // 3. 批量写入
    batch_insert_nodes(&nodes).await?;
    batch_insert_relations(&relations).await?;

    Ok(SceneTreeInitResult {
        node_count: nodes.len(),
        relation_count: relations.len(),
    })
}
```

### 4.3 递归构建树

```rust
async fn build_tree_from_world(
    world_refno: RefnoEnum,
) -> Result<(Vec<SceneNode>, Vec<(i64, i64)>)> {
    let mut nodes = Vec::new();
    let mut relations = Vec::new();
    let mut queue = VecDeque::new();

    queue.push_back((world_refno, None::<i64>));

    while let Some((refno, parent_id)) = queue.pop_front() {
        // 1. 获取节点信息
        let pe = aios_core::get_pe(refno).await?;
        let refno_i64 = refno.to_i64();
        let dbno = refno.dbno() as i16;

        // 2. 判断是否为几何节点
        let has_geo = is_geo_noun(&pe.noun);

        // 3. 收集节点
        nodes.push(SceneNode {
            id: refno_i64,
            parent: parent_id,
            has_geo,
            dbno,
            aabb: None, // 初始化时为空
        });

        // 4. 收集关系
        if let Some(pid) = parent_id {
            relations.push((pid, refno_i64));
        }

        // 5. 获取子节点
        let children = aios_core::get_children_refnos(refno).await?;
        for child in children {
            queue.push_back((child, Some(refno_i64)));
        }
    }

    Ok((nodes, relations))
}
```

---

## 5. AABB 更新机制

### 5.1 更新时机

当叶子节点的模型生成完成后，需要更新 AABB：

1. 更新叶子节点自身的 AABB
2. 向上递归更新所有祖先节点的 AABB（聚合子节点）

### 5.2 更新流程

```rust
/// 更新节点 AABB 并向上传播
pub async fn update_aabb_recursive(
    refno_i64: i64,
    aabb_id: &str,
) -> Result<()> {
    // 1. 更新当前节点
    let sql = format!(
        "UPDATE scene_node:{} SET aabb = aabb:⟨{}⟩",
        refno_i64, aabb_id
    );
    SUL_DB.query(sql).await?;

    // 2. 获取祖先链并更新
    let ancestors = get_ancestors(refno_i64).await?;
    for ancestor_id in ancestors {
        update_parent_aabb(ancestor_id).await?;
    }

    Ok(())
}
```

---

## 6. 查询示例

### 6.1 查询所有几何叶子节点

```sql
-- 查询 EQUI(104679055498) 下所有几何叶子节点（叶子=无子节点）
SELECT * FROM scene_node:104679055498.{1..20}.->contains->scene_node
WHERE has_geo = true AND is_leaf = true
```

### 6.2 查询直接子节点

```sql
SELECT * FROM scene_node:104679055498->contains->scene_node
```

### 6.3 查询祖先链

```sql
-- 注意：SurrealDB v3 对 v2 的图递归语法不兼容，本项目已在 Rust 侧做 BFS（最多 20 层）来收集子孙节点。
-- 如需递归查询，请优先使用后端接口（/api/scene-tree/{refno}/leaves）或复用 query_ungenerated_leaves 的实现。
--
-- 下面示例为“查询一跳子节点”：
SELECT VALUE meta::id(out) FROM contains WHERE in = scene_node:104679055498;
```

### 6.4 PE 与 Scene Node 互转

```sql
-- pe ID → scene_node
LET $id = parse::uint64::pair_to_u64("24383_73962");
SELECT * FROM scene_node:$id;

-- scene_node → pe ID
LET $pe_id = parse::uint64::u64_to_pair_str(104679055498);
SELECT * FROM pe:⟨$pe_id⟩;
```

---

## 7. API 设计

### 7.1 HTTP 接口

| 端点 | 方法 | 说明 |
|-----|------|-----|
| `/api/scene-tree/init` | POST | 初始化 Scene Tree |
| `/api/scene-tree/init/{dbno}` | POST | 初始化指定 dbno |
| `/api/scene-tree/init-by-root/{refno}` | POST | 从指定 root refno 构建子树（推荐用于测试/按需构建） |
| `/api/scene-tree/{refno}/leaves` | GET | 查询几何叶子节点 |
| `/api/scene-tree/{refno}/children` | GET | 查询子节点 |
| `/api/scene-tree/{refno}/ancestors` | GET | 查询祖先链 |

> 说明：这里的 `dbno/refno` 指 `refno` 字符串左侧数字（u64 高 32 位），不等同于 `pe_1112` 这类分表后缀的 `dbnum` 概念；如果你的数据来源是 `pe_1112`，更推荐先从该表选一个根 `refno`，再调用 `/api/scene-tree/init-by-root/{refno}`。

### 7.2 请求/响应示例

**初始化请求**：
```json
POST /api/scene-tree/init
{
  "mdb_name": "ALL",
  "force_rebuild": false
}
```

**初始化响应**：
```json
{
  "success": true,
  "node_count": 50000,
  "relation_count": 49999,
  "duration_ms": 3500
}
```

**查询叶子节点响应**：
```json
{
  "success": true,
  "refno": "24383_73962",
  "leaves": [104679055500, 104679055501, 104679055502],
  "count": 3
}
```

---

## 8. 模块结构

```
src/scene_tree/
├── mod.rs           // 模块入口
├── schema.rs        // 表结构定义
├── init.rs          // 初始化逻辑
├── query.rs         // 查询方法
├── aabb.rs          // AABB 更新逻辑
└── api.rs           // HTTP API
```

---

## 9. 实施计划

### 9.1 阶段一：基础结构

- [ ] 创建 `scene_tree` 模块
- [ ] 定义 SurrealDB 表结构
- [ ] 实现 `is_geo_noun` 判断函数

### 9.2 阶段二：初始化

- [ ] 实现 `init_scene_tree` 入口函数
- [ ] 实现 BFS 递归构建
- [ ] 实现批量写入 scene_node 和 contains

### 9.3 阶段三：查询

- [ ] 实现查询几何叶子节点
- [ ] 实现查询子节点/祖先链
- [ ] 集成 `parse::uint64` 转换

### 9.4 阶段四：AABB 更新

- [ ] 实现叶子节点 AABB 更新
- [ ] 实现祖先节点 AABB 聚合
- [ ] 集成到模型生成流程

### 9.5 阶段五：API

- [ ] 实现 HTTP API 端点
- [ ] 集成到 web_server 路由

---

## 10. 生成状态查询实现

### 10.1 Rust 后端实现

#### 数据结构定义

```rust
use surrealdb::types::{self as surrealdb_types, SurrealValue};

#[derive(Debug, Serialize, Deserialize, SurrealValue)]
pub struct SceneNodeStatus {
    pub id: i64,
    pub has_geo: bool,
    pub generated: bool,
}
```

#### 批量查询生成状态

```rust
/// 批量查询 refnos 的生成状态
pub async fn query_generation_status(
    refnos: &[RefnoEnum],
) -> anyhow::Result<Vec<SceneNodeStatus>> {
    if refnos.is_empty() {
        return Ok(vec![]);
    }

    let ids: Vec<i64> = refnos.iter().map(|r| r.to_i64()).collect();
    let id_list = ids
        .iter()
        .map(|id| format!("scene_node:{}", id))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT VALUE {{ id: meta::id(id), has_geo: has_geo, generated: generated }} FROM [{}]",
        id_list
    );

    let result: Vec<SceneNodeStatus> = SUL_DB.query_take(&sql, 0).await?;
    Ok(result)
}
```

#### 过滤未生成的几何节点

```rust
/// 从 refnos 中过滤出未生成的几何节点
pub async fn filter_ungenerated_geo_nodes(
    refnos: &[RefnoEnum],
) -> anyhow::Result<Vec<i64>> {
    if refnos.is_empty() {
        return Ok(vec![]);
    }

    let ids: Vec<i64> = refnos.iter().map(|r| r.to_i64()).collect();
    let id_list = ids
        .iter()
        .map(|id| format!("scene_node:{}", id))
        .collect::<Vec<_>>()
        .join(",");

    // 使用复合索引 idx_has_geo_generated 优化查询
    let sql = format!(
        "SELECT VALUE meta::id(id) FROM [{}] WHERE has_geo = true AND generated = false",
        id_list
    );

    let result: Vec<i64> = SUL_DB.query_take(&sql, 0).await?;
    Ok(result)
}
```

#### 批量标记为已生成

```rust
/// 批量标记节点为已生成
pub async fn mark_as_generated(ids: &[i64]) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }

    let id_list = ids
        .iter()
        .map(|id| format!("scene_node:{}", id))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "UPDATE [{}] SET generated = true",
        id_list
    );

    SUL_DB.query_response(&sql).await?;
    Ok(())
}
```

#### 查询节点下所有未生成的几何叶子

```rust
/// 查询指定节点下所有未生成的几何叶子节点
pub async fn query_ungenerated_leaves(root_id: i64) -> anyhow::Result<Vec<i64>> {
    // 使用 Graph 递归查询，深度 1..20
    let sql = format!(
        r#"SELECT VALUE meta::id(id)
           FROM scene_node:{}->contains->(1..20)->scene_node
           WHERE has_geo = true AND is_leaf = true AND generated = false"#,
        root_id
    );

    let result: Vec<i64> = SUL_DB.query_take(&sql, 0).await?;
    Ok(result)
}
```

### 10.2 前端 TypeScript 实现

#### 类型定义

```typescript
// types/sceneTree.ts

/** Scene Node 状态 */
export interface SceneNodeStatus {
  id: number        // u64 编码的 refno
  has_geo: boolean  // 是否为几何节点
  generated: boolean // 模型是否已生成
}

/** refno 转换工具 */
export const RefnoUtils = {
  /** 字符串 refno 转 u64 */
  toU64(refno: string): bigint {
    const [dbno, ref] = refno.split('_').map(Number)
    return (BigInt(dbno) << 32n) | BigInt(ref)
  },

  /** u64 转字符串 refno */
  fromU64(id: bigint): string {
    const dbno = Number(id >> 32n)
    const ref = Number(id & 0xFFFFFFFFn)
    return `${dbno}_${ref}`
  }
}
```

#### API 调用

```typescript
// api/sceneTree.ts

const API_BASE = import.meta.env.VITE_API_BASE_URL

/** 初始化 Scene Tree */
export async function initSceneTree(mdbName: string = 'ALL'): Promise<{
  success: boolean
  node_count: number
  relation_count: number
  duration_ms: number
}> {
  const resp = await fetch(`${API_BASE}/api/scene-tree/init`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ mdb_name: mdbName, force_rebuild: false })
  })
  return resp.json()
}

/** 查询几何叶子节点 */
export async function queryGeoLeaves(refno: string): Promise<{
  success: boolean
  leaves: number[]
  count: number
}> {
  const resp = await fetch(`${API_BASE}/api/scene-tree/${refno}/leaves`)
  return resp.json()
}
```

#### 流式生成集成

```typescript
// composables/useSceneTreeGenerate.ts

import { ref } from 'vue'
import { RefnoUtils } from '@/types/sceneTree'

export function useSceneTreeGenerate() {
  const generating = ref(false)
  const progress = ref(0)

  /**
   * 流式生成模型（跳过已生成的）
   */
  async function generateWithSceneTree(
    refnos: string[],
    onBatchComplete?: (generatedRefnos: string[]) => void
  ) {
    generating.value = true
    progress.value = 0

    try {
      // 后端会自动：
      // 1. 查询 scene_node 过滤已生成的
      // 2. 只生成 has_geo=true && generated=false 的节点
      // 3. 生成完成后标记 generated=true
      const response = await fetch(`${API_BASE}/api/model/stream-generate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ refnos, skipGenerated: true })
      })

      // 处理 SSE 事件...
    } finally {
      generating.value = false
    }
  }

  return { generating, progress, generateWithSceneTree }
}
```

---

## 11. 废弃 inst_relate_aabb 迁移方案

### 11.1 背景

当前 `inst_relate_aabb` 表有两个主要用途：
1. 存储实例 AABB（包围盒数据）
2. 判断模型是否已生成（通过记录是否存在）

引入 `scene_node` 后，这两个功能可以统一：
- `aabb` 字段：存储包围盒引用
- `generated` 字段：标记是否已生成

### 11.2 功能对比

| 功能 | inst_relate_aabb | scene_node |
|-----|------------------|------------|
| 存储 AABB | `aabb: record<aabb>` | `aabb: option<record<aabb>>` |
| 判断已生成 | 记录存在即已生成 | `generated = true` |
| 层级查询 | 不支持 | 支持 Graph 递归查询 |
| 批量查询子孙 | 需要多次查询 | 单次 Graph 查询 |

### 11.3 需要修改的文件

| 文件 | 修改内容 |
|-----|---------|
| `src/fast_model/utils.rs` | `save_inst_relate_aabb` → `update_scene_node_aabb` |
| `src/fast_model/mesh_generate.rs` | 替换所有 `inst_relate_aabb` 查询 |
| `src/fast_model/gen_model/orchestrator.rs` | 更新 AABB 写入逻辑 |
| `src/data_interface/surreal_schema.sql` | 删除 `inst_relate_aabb` 表定义 |

### 11.4 新增函数

#### 更新 scene_node 的 AABB

```rust
/// 批量更新 scene_node 的 AABB 并标记为已生成
pub async fn update_scene_node_aabb(
    inst_aabb_map: &DashMap<RefnoEnum, String>,
) -> anyhow::Result<()> {
    if inst_aabb_map.is_empty() {
        return Ok(());
    }

    let keys: Vec<_> = inst_aabb_map.iter().map(|e| *e.key()).collect();

    for chunk in keys.chunks(200) {
        let mut sql = String::new();
        for refno in chunk {
            let Some(aabb_hash) = inst_aabb_map.get(refno) else { continue };
            let id = refno.to_i64();
            sql.push_str(&format!(
                "UPDATE scene_node:{} SET aabb = aabb:⟨{}⟩, generated = true;",
                id, aabb_hash.value()
            ));
        }
        if !sql.is_empty() {
            SUL_DB.query_response(&sql).await?;
        }
    }
    Ok(())
}
```

#### 查询已生成的节点

```rust
/// 查询已生成的节点（替代 inst_relate_aabb 存在性检查）
pub async fn query_generated_refnos(
    refnos: &[RefnoEnum],
) -> anyhow::Result<Vec<RefnoEnum>> {
    if refnos.is_empty() {
        return Ok(vec![]);
    }

    let id_list = refnos
        .iter()
        .map(|r| format!("scene_node:{}", r.to_i64()))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT VALUE meta::id(id) FROM [{}] WHERE generated = true",
        id_list
    );

    let ids: Vec<i64> = SUL_DB.query_take(&sql, 0).await?;
    Ok(ids.into_iter().map(RefnoEnum::from_i64).collect())
}
```

### 11.5 迁移步骤

1. **实现 scene_tree 模块** - 完成初始化和基础查询
2. **添加新函数** - `update_scene_node_aabb`、`query_generated_refnos`
3. **替换调用点** - 逐步替换 `inst_relate_aabb` 相关调用
4. **验证功能** - 确保模型生成流程正常
5. **删除旧表** - 移除 `inst_relate_aabb` 表定义和相关代码
