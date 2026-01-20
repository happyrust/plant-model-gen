# export_dbnum_instances_json Hash 化重构方案

将 `export-dbnum-instances-json` 导出格式改为 hash 引用形式，使用数据库已有 hash，同步修改前端，不保留向后兼容。

---

## 确认事项

- [x] Hash 来源：使用数据库已有 hash
- [x] 前端改动：同步修改 plant3d-web
- [x] 向后兼容：不需要

---

## 一、现状分析

### 1.1 当前输出格式（V2）

```json
{
  "version": 2,
  "generated_at": "...",
  "groups": [{
    "owner_refno": "17496_xxx",
    "owner_aabb": { "min": [x,y,z], "max": [x,y,z] },  // 展开的 AABB
    "children": [{
      "refno": "17496_xxx",
      "aabb": { "min": [x,y,z], "max": [x,y,z] },      // 展开的 AABB
      "refno_transform": [16个f32],                    // 展开的 4x4 矩阵
      "geo_instances": [{
        "geo_hash": "...",
        "geo_transform": [16个f32]                     // 展开的 4x4 矩阵
      }]
    }],
    "tubings": [{
      "aabb": { "min": [x,y,z], "max": [x,y,z] },
      "matrix": [16个f32]
    }]
  }],
  "instances": [...]
}
```

**问题**：
- 相同的 transform / aabb 数据重复存储
- 文件体积大，加载慢
- 未利用数据库已有的 hash 去重机制

### 1.2 数据库现有 Hash 机制

| 表名 | ID 格式 | 存储内容 |
|------|---------|----------|
| `trans` | `trans:⟨hash⟩` | `{ d: "JSON序列化Transform" }` |
| `aabb` | `aabb:⟨hash⟩` | `{ d: "JSON序列化Aabb" }` |

- Hash 由 `gen_bytes_hash(&T)` 生成（u64）
- `tubi_relate` 已使用：`world_trans: trans:⟨hash⟩`, `aabb: aabb:⟨hash⟩`
- `inst_relate_aabb` 已使用：`out: aabb:⟨hash⟩`

---

## 二、目标格式（V3）

```json
{
  "version": 3,
  "generated_at": "...",
  
  "trans_table": {
    "123456789": [1,0,0,0, 0,1,0,0, 0,0,1,0, x,y,z,1],
    "987654321": [...]
  },
  
  "aabb_table": {
    "111222333": { "min": [x,y,z], "max": [x,y,z] },
    "444555666": { "min": [...], "max": [...] }
  },
  
  "groups": [{
    "owner_refno": "17496_xxx",
    "owner_aabb_hash": "111222333",           // 引用 aabb_table
    "children": [{
      "refno": "17496_xxx",
      "aabb_hash": "111222333",               // 引用 aabb_table (可能与 owner 相同)
      "trans_hash": "123456789",              // 引用 trans_table (refno_transform)
      "geo_instances": [{
        "geo_hash": "...",
        "trans_hash": "987654321"             // 引用 trans_table (geo_transform)
      }]
    }],
    "tubings": [{
      "aabb_hash": "444555666",
      "trans_hash": "123456789"
    }]
  }],
  "instances": [...]
}
```

**优点**：
- 相同矩阵/AABB 只存一份，文件体积大幅减少
- 前端构建 lookup Map 后 O(1) 查找
- 与数据库 hash 机制一致，便于增量更新

---

## 三、实现步骤

### 3.1 修改查询逻辑

**文件**: `src/fast_model/export_model/export_prepack_lod.rs`

1. 修改 `TubiQueryResult` 结构体，添加 hash 字段：
   ```rust
   struct TubiQueryResult {
       // 现有字段...
       pub world_aabb_hash: Option<String>,   // record::id(aabb) as world_aabb_hash
       pub world_trans_hash: Option<String>,  // record::id(world_trans) as world_trans_hash
   }
   ```

2. 修改 tubi_relate 查询 SQL：
   ```sql
   SELECT
       id[0] as refno,
       id[1] as index,
       in as leave,
       record::id(aabb) as world_aabb_hash,      -- 新增
       record::id(world_trans) as world_trans_hash, -- 新增
       aabb.d as world_aabb,
       world_trans.d as world_trans,
       record::id(geo) as geo_hash,
       spec_value
   FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..]
   ```

3. 修改 inst_relate 聚合查询，获取 aabb hash：
   ```sql
   in.map(|$x| record::id($x->inst_relate_aabb[0].out)) as child_aabb_hashes
   ```

### 3.2 新增 Hash 收集器

```rust
struct HashCollector {
    trans_map: HashMap<String, Vec<f32>>,   // hash -> matrix[16]
    aabb_map: HashMap<String, PlantAabb>,   // hash -> aabb
}

impl HashCollector {
    fn insert_trans(&mut self, hash: &str, matrix: &DMat4) { ... }
    fn insert_aabb(&mut self, hash: &str, aabb: &PlantAabb) { ... }
    fn into_tables(self) -> (serde_json::Value, serde_json::Value) { ... }
}
```

### 3.3 修改 JSON 构建逻辑

将 `export_dbnum_instances_json` 函数中的直接展开改为 hash 引用：

```rust
// 旧：展开矩阵
"refno_transform": world_transform_vec,

// 新：hash 引用
"trans_hash": trans_hash_str,
```

### 3.4 查询 geo_relate 的 trans hash

对于 `geo_instances` 中的 `geo_transform`，需从 `geo_relate.trans` 获取 hash：

```rust
// aios_core::query_insts 需要扩展，返回 trans_hash
struct ModelHashInst {
    pub geo_hash: String,
    pub transform: PlantTransform,
    pub trans_hash: Option<String>,  // 新增
}
```

或者在导出时重新计算 hash：
```rust
let trans_hash = gen_bytes_hash(&inst.transform);
```

---

## 四、代码改动清单

| 文件 | 改动内容 |
|------|----------|
| `src/fast_model/export_model/export_prepack_lod.rs` | 主要修改：查询 SQL、JSON 构建、新增 HashCollector |
| `aios_core` (可选) | 扩展 `query_insts` 返回 trans_hash |
| 前端 (plant3d-web) | 适配 V3 格式，构建 lookup table |

### 4.1 export_prepack_lod.rs 具体改动

1. **新增结构体** `HashCollector`
2. **修改函数** `export_dbnum_instances_json`:
   - 查询时提取 hash
   - 构建 `trans_table` / `aabb_table`
   - 输出时使用 `trans_hash` / `aabb_hash` 代替展开值
3. **修改 SQL** 查询语句

---

## 五、兼容性考虑

1. **版本号升级**: `"version": 2` → `"version": 3`
2. **前端适配**: 根据 version 字段选择解析逻辑
3. **渐进迁移**: 可保留 V2 导出函数，新增 V3 导出函数

---

## 六、预期收益

| 指标 | V2 | V3 (预估) |
|------|-----|-----------|
| 文件体积 | 100% | ~40-60% |
| 重复矩阵存储 | 多次 | 1次 |
| 前端解析 | 直接读取 | 需要 lookup |

---

## 七、详细实现步骤

### 7.1 后端改动 (gen_model-dev)

**文件**: `src/fast_model/export_model/export_prepack_lod.rs`

#### Step 1: 修改查询结构体

```rust
// TubiQueryResult 新增 hash 字段
struct TubiQueryResult {
    pub world_aabb_hash: Option<String>,   // 新增
    pub world_trans_hash: Option<String>,  // 新增
    // ... 保留原有字段
}

// GroupedOwnerResultWithDbnum 新增 child_aabb_hashes
struct GroupedOwnerResultWithDbnum {
    pub child_aabb_hashes: Vec<Option<String>>,  // 新增
    // ... 保留原有字段
}
```

#### Step 2: 修改 SQL 查询

```sql
-- tubi_relate 查询
SELECT
    record::id(aabb) as world_aabb_hash,
    record::id(world_trans) as world_trans_hash,
    aabb.d as world_aabb,
    world_trans.d as world_trans,
    ...

-- inst_relate 聚合查询
in.map(|$x| record::id($x->inst_relate_aabb[0].out)) as child_aabb_hashes
```

#### Step 3: 新增 HashCollector

```rust
struct HashCollector {
    trans_map: HashMap<String, Vec<f32>>,
    aabb_map: HashMap<String, serde_json::Value>,
}
```

#### Step 4: 修改 JSON 构建

- 收集所有 trans/aabb 到 HashCollector
- 输出 `trans_table` / `aabb_table` 到 JSON 顶层
- 实例使用 `trans_hash` / `aabb_hash` 引用

### 7.2 前端改动 (plant3d-web)

**文件**: `src/utils/instances/instanceManifest.ts`

#### Step 1: 更新类型定义

```typescript
type InstanceManifestV3 = {
  version: 3
  generated_at: string
  trans_table: Record<string, number[]>
  aabb_table: Record<string, { min: number[]; max: number[] }>
  groups: GroupV3[]
  instances: FlatInstanceV3[]
}

type ChildV3 = {
  refno: string
  aabb_hash?: string | null
  trans_hash?: string | null
  geo_instances: { geo_hash: string; trans_hash?: string }[]
}
```

#### Step 2: 修改 parseManifest 函数

```typescript
if (manifest.version === 3) {
  const transTable = manifest.trans_table || {}
  const aabbTable = manifest.aabb_table || {}
  
  // 通过 hash 查找实际数据
  const refnoTransform = transTable[child.trans_hash] || IDENTITY_MATRIX
  const aabb = aabbTable[child.aabb_hash] || null
}
```

---

## 八、改动文件清单

| 项目 | 文件 | 改动类型 |
|------|------|----------|
| gen_model-dev | `src/fast_model/export_model/export_prepack_lod.rs` | 主要修改 |
| plant3d-web | `src/utils/instances/instanceManifest.ts` | 主要修改 |
| plant3d-web | `src/composables/useDbnoInstancesDtxLoader.ts` | 适配修改 |
