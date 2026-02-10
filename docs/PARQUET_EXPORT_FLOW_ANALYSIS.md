# 模型数据导出 Parquet 完整流程分析

> 文档版本: 1.0  
> 创建时间: 2025-12-02  
> 分析对象: aios-database 项目中的 Parquet 导出功能

## 概述

本文档详细分析了项目中将模型数据导出为 Parquet 格式的整个流程，包括数据查询、处理和存储等各个环节。

## 目录

- [1. 导出类型总览](#1-导出类型总览)
- [2. 场景树导出 (Scene Tree)](#2-场景树导出-scene-tree)
- [3. 模型数据导出 (Model Data)](#3-模型数据导出-model-data)
- [4. 实例数据导出 (Instance Data)](#4-实例数据导出-instance-data)
- [5. 完整导出 (Prepack LOD + Parquet)](#5-完整导出-prepack-lod--parquet)
- [6. 命令行使用](#6-命令行使用)
- [7. 数据流转图](#7-数据流转图)
- [8. 技术栈](#8-技术栈)

---

## 1. 导出类型总览

项目支持多种 Parquet 导出类型，各有不同的应用场景：

| 导出类型 | 文件名格式 | 主要用途 | 入口函数 |
|---------|-----------|---------|---------|
| **场景树** | `scene_tree_{dbnum}.parquet` | 前端树形结构加载 | `export_scene_tree_parquet()` |
| **模型数据** | `db_models_{dbnum}.parquet` | 模型实例 + 变换矩阵 | `export_db_models_parquet()` |
| **实例数据** | `instances_{dbnum}.parquet` 等 5 个文件 | 完整的实例关系和几何引用 | `export_dbnum_instances_parquet()` |
| **完整导出** | GLB + Parquet 组合 | 生产环境完整模型包 | `export_all_relates_prepack_lod_parquet()` |
| **AABB 数据** | `inst_aabb.parquet` | 空间计算和碰撞检测 | `export_inst_aabb_parquet()` |

---

## 2. 场景树导出 (Scene Tree)

### 2.1 功能描述

导出场景节点的层次结构，供前端快速加载和渲染场景树。

### 2.2 数据源

**主表**: `scene_node`  
**关联表**: `pe` (获取节点名称)

### 2.3 SQL 查询

```sql
SELECT
    record::id(id) as id,
    parent,
    has_geo,
    is_leaf,
    generated,
    dbnum,
    geo_type,
    (SELECT VALUE name FROM pe WHERE id = scene_node.id LIMIT 1)[0] ?? '' as name
FROM scene_node
WHERE dbnum = {dbnum}
```

### 2.4 输出字段

| 字段名 | 类型 | 说明 |
|-------|------|------|
| `id` | i64 | 节点 ID |
| `parent` | Option\<i64\> | 父节点 ID |
| `has_geo` | bool | 是否有几何体 |
| `is_leaf` | bool | 是否叶子节点 |
| `generated` | bool | 是否已生成几何体 |
| `dbnum` | i32 | 数据库编号 |
| `geo_type` | Option\<String\> | 几何体类型 |
| `name` | String | 节点名称 |

### 2.5 代码位置

- **文件**: `src/scene_tree/parquet_export.rs`
- **函数**: `export_scene_tree_parquet(dbnum: u32, output_dir: &Path)`
- **调用时机**: 场景树初始化完成后自动导出

### 2.6 输出路径

```
{project_tree_dir}/scene_tree_{dbnum}.parquet
```

---

## 3. 模型数据导出 (Model Data)

### 3.1 功能描述

导出模型实例的基本信息、世界变换矩阵和几何体引用，按 `dbnum` 分组。

### 3.2 数据源

**主表**: `inst_relate`  
**关联表**: 
- `pe_transform` (世界变换矩阵)
- `geo_relate` (几何体引用)

### 3.3 SQL 查询

```sql
SELECT
    <string>in as refno,
    in.dbnum as dbnum,
    in.noun as noun,
    (
        SELECT VALUE world_trans.d.matrix
        FROM pe_transform
        WHERE id = type::record('pe_transform', record::id(in))
        LIMIT 1
    )[0] as matrix,
    out.out.id as geo_id,
    out.out.id as geo_hash,
    out.trans.d.matrix as geo_matrix
FROM inst_relate
WHERE (SELECT VALUE world_trans FROM pe_transform
       WHERE id = type::record('pe_transform', record::id(in))
       LIMIT 1
      )[0] != NONE
  AND in.dbnum != none
  {db_filter}
```

### 3.4 输出字段

| 字段名 | 类型 | 说明 |
|-------|------|------|
| `refno` | String | 实例引用号 |
| `noun` | String | 对象类型 (EQUI, PIPE, etc.) |
| `geo_hash` | String | 几何体哈希值 |
| `t0-t15` | f64 × 16 | 4×4 变换矩阵 (列优先) |

### 3.5 处理流程

1. **查询数据**: 从 SurrealDB 查询 `inst_relate` + `pe_transform` + `geo_relate`
2. **按 dbnum 分组**: 将结果按数据库编号分组
3. **构建列数据**: 提取 refno, noun, geo_hash, 以及 16 个矩阵分量
4. **生成 DataFrame**: 使用 Polars 创建 DataFrame
5. **写入文件**: 为每个 dbnum 生成独立的 `.parquet` 文件

### 3.6 代码位置

- **文件**: `src/fast_model/export_model/export_parquet.rs`
- **函数**: `export_db_models_parquet(target_path: &Path, db_nums: Option<Vec<i64>>)`
- **调用方式**: 通过 Web API 或直接调用

### 3.7 输出路径

```
{target_path}/db_models_{dbnum}.parquet
```

---

## 4. 实例数据导出 (Instance Data)

### 4.1 功能描述

这是**最完整**的导出方式，生成多个关联的 Parquet 表，支持前端使用 DuckDB 进行复杂查询。

### 4.2 输出文件列表

| 文件名 | Schema | 说明 |
|--------|--------|------|
| **instances.parquet** | refno, noun, name, owner, trans_hash, aabb_hash, spec_value, has_neg, dbnum | 实例主表，一行一个实例 |
| **geo_instances.parquet** | refno, geo_index, geo_hash, geo_trans_hash | 几何体引用表，支持多几何体实例 |
| **tubings.parquet** | refno, tubi_index, tubi_refno, tubi_noun | 管道段数据 (仅 TUBI 类型) |
| **transforms.parquet** | trans_hash, t0-t15 | 变换矩阵共享表 (去重) |
| **aabb.parquet** | aabb_hash, min_x/y/z, max_x/y/z | 包围盒共享表 (去重) |

### 4.3 数据查询流程

#### Step 1: 确定导出范围

```rust
if let Some(root) = root_refno {
    // 查询子孙节点 (visible only)
    query_visible_descendants(root, dbnum).await?
} else {
    // 全量查询 dbnum
    query_all_visible_by_dbnum(dbnum).await?
}
```

#### Step 2: 批量查询实例关系

**SQL (inst_relate)**:
```sql
SELECT
    owner_refno,
    owner_type,
    in as refno,
    in.noun as noun,
    fn::default_full_name(in) as name,
    record::id(in->inst_relate_aabb[0].out) as aabb_hash,
    spec_value as spec_value
FROM inst_relate
WHERE in IN [{refno_list}]
    AND in->inst_relate_aabb[0].out != NONE
    AND in->inst_relate_aabb[0].out.d != NONE
```

**批处理**: 每批 1000 个 refno，避免 SQL 过长

#### Step 3: 查询几何体关系

**SQL (geo_relate)**:
```sql
SELECT
    <string>in as refno,
    geo_index,
    record::id(out) as geo_hash,
    trans.d as geo_trans
FROM geo_relate
WHERE in IN [{refno_list}]
    AND visible = true
    AND out != NONE
ORDER BY in, geo_index
```

#### Step 4: 查询管道段 (仅 TUBI)

**SQL (tubi_relate)**:
```sql
SELECT
    <string>in as refno,
    tubi_index,
    <string>out as tubi_refno,
    out.noun as tubi_noun
FROM tubi_relate
WHERE in IN [{tubi_refno_list}]
ORDER BY in, tubi_index
```

#### Step 5: 查询变换矩阵

**SQL (pe_transform)**:
```sql
SELECT
    record::id(id) as refno,
    world_trans.d as world_trans
FROM pe_transform
WHERE id IN [{refno_list}]
    AND world_trans.d != NONE
```

#### Step 6: 查询包围盒

**SQL (aabb_hash)**:
```sql
SELECT
    record::id(id) as hash,
    d as aabb
FROM aabb_hash
WHERE id IN [{aabb_hash_list}]
    AND d != NONE
```

### 4.4 构建 Parquet 表

#### instances.parquet

```rust
struct InstanceRow {
    refno_str: String,      // "24381/145018"
    refno_u64: u64,         // 压缩表示
    noun: String,           // "BRAN"
    name: String,           // "MAIN-PIPE"
    owner_refno_str: Option<String>,
    owner_refno_u64: Option<u64>,
    owner_noun: String,
    trans_hash: String,     // 变换哈希 (关联 transforms.parquet)
    aabb_hash: String,      // 包围盒哈希 (关联 aabb.parquet)
    spec_value: i64,
    has_neg: bool,          // 是否有布尔减运算
    dbnum: u32,
}
```

#### geo_instances.parquet

```rust
struct GeoInstanceRow {
    refno_str: String,      // 关联 instances.parquet
    refno_u64: u64,
    geo_index: u32,         // 几何体索引 (同一个实例可能有多个几何体)
    geo_hash: String,       // 几何体哈希 (对应 GLB 文件名)
    geo_trans_hash: String, // 局部变换哈希
}
```

#### transforms.parquet

```rust
struct TransformRow {
    trans_hash: String,     // MD5 哈希值
    t0, t1, ..., t15: f32,  // 4×4 矩阵 (列优先)
}
```

#### aabb.parquet

```rust
struct AabbRow {
    aabb_hash: String,      // MD5 哈希值
    min_x, min_y, min_z: f32,
    max_x, max_y, max_z: f32,
}
```

### 4.5 去重与压缩

- **transforms**: 通过 MD5 哈希去重，相同变换矩阵只存储一次
- **aabb**: 通过 MD5 哈希去重，相同包围盒只存储一次
- **压缩**: 使用 ZSTD Level 3 压缩

### 4.6 代码位置

- **文件**: `src/fast_model/export_model/export_dbnum_instances_parquet.rs`
- **函数**: `export_dbnum_instances_parquet(...)`
- **调用**: 命令行 `--export-dbnum-instances-parquet`

### 4.7 输出路径

```
{output_dir}/database_models/{dbnum}/
├── instances_{dbnum}.parquet
├── geo_instances_{dbnum}.parquet
├── tubings_{dbnum}.parquet  (如果有 TUBI)
├── transforms.parquet
├── aabb.parquet
└── manifest.json
```

---

## 5. 完整导出 (Prepack LOD + Parquet)

### 5.1 功能描述

结合 GLB 几何体文件和 Parquet 清单，生成生产环境可直接使用的完整模型包。

### 5.2 导出内容

1. **GLB 文件**: 实际的几何体网格 (L1/L2/L3 多级 LOD)
2. **Parquet 清单**: 实例化信息和元数据
3. **Manifest**: JSON 格式的索引文件

### 5.3 LOD 级别

| 级别 | 精度 | 用途 |
|------|------|------|
| L1 | 高精度 | 近距离查看 |
| L2 | 中精度 | 中距离查看 |
| L3 | 低精度 | 远距离/缩略图 |

### 5.4 命令行示例

```bash
# 导出所有 LOD 级别
cargo run -- --export-all-parquet --dbnum 7997 --export-all-lods

# 仅导出 L1
cargo run -- --export-all-parquet --dbnum 7997

# 按 owner_type 过滤
cargo run -- --export-all-parquet --dbnum 7997 --owner-types "BRAN,HANG"

# 指定 refno
cargo run -- --export-all-parquet --export-refnos "24381_145018,24381_145019"
```

### 5.5 代码位置

- **文件**: `src/fast_model/export_model/export_prepack_lod.rs`
- **函数**: `export_all_relates_prepack_lod_parquet(...)`

---

## 6. 命令行使用

### 6.1 场景树导出

场景树导出通常在场景树初始化时自动触发，也可以通过编程方式调用。

### 6.2 模型数据导出

```bash
# 通过 Web API
curl -X POST http://localhost:8080/api/export/parquet \
  -H "Content-Type: application/json" \
  -d '{"db_nums": [7997], "output_path": "output/parquet"}'
```

### 6.3 实例数据导出

```bash
# 默认格式 (Parquet)
cargo run -- --export-dbnum-instances --dbnum 7997

# 显式指定 Parquet
cargo run -- --export-dbnum-instances-parquet --dbnum 7997

# 仅导出某个 BRAN 的子孙
cargo run -- --export-dbnum-instances-parquet \
  --dbnum 7997 \
  --debug-model 24381/145018

# 指定输出目录
cargo run -- --export-dbnum-instances-parquet \
  --dbnum 7997 \
  --output ./my_output
```

### 6.4 完整导出

```bash
# 导出所有 inst_relate 实体 (Prepack LOD + Parquet)
cargo run -- --export-all-parquet \
  --dbnum 7997 \
  --export-all-lods \
  --output ./production_models
```

---

## 7. 数据流转图

### 7.1 总体数据流

```
SurrealDB 数据库
    │
    ├── scene_node ──────────┐
    ├── pe ──────────────────┤
    ├── inst_relate ─────────┤
    ├── geo_relate ──────────┤
    ├── tubi_relate ─────────┼──→ Rust 查询层
    ├── pe_transform ────────┤      │
    ├── aabb_hash ───────────┘      │
    │                               │
    │                               ↓
    │                         数据处理层
    │                         (去重/分组/转换)
    │                               │
    │                               ↓
    │                         Polars DataFrame
    │                               │
    │                               ↓
    │                         ParquetWriter
    │                               │
    │                               ↓
    └─────────────────────→  .parquet 文件
```

### 7.2 实例数据导出详细流程

```
1. 确定导出范围
   ├─ root_refno 存在? → 查询子孙节点
   └─ root_refno 不存在? → 查询全部 dbnum

2. 批量查询 (每批 1000 个)
   ├─ inst_relate (实例关系)
   ├─ geo_relate (几何体关系)
   ├─ tubi_relate (管道段)
   ├─ pe_transform (变换矩阵)
   └─ aabb_hash (包围盒)

3. 数据处理
   ├─ 提取变换矩阵 → MD5 哈希 → transforms 表
   ├─ 提取包围盒 → MD5 哈希 → aabb 表
   ├─ 组装实例行 → instances 表
   ├─ 展开几何体引用 → geo_instances 表
   └─ 提取管道段 → tubings 表

4. 构建 Arrow RecordBatch
   ├─ 定义 Schema (字段类型)
   ├─ 构建 Array (列数据)
   └─ 组合成 RecordBatch

5. 写入 Parquet
   ├─ 配置压缩 (ZSTD Level 3)
   ├─ 写入文件
   └─ 生成 manifest.json
```

---

## 8. 技术栈

### 8.1 核心库

| 库名 | 版本 | 用途 |
|------|------|------|
| **polars** | latest | DataFrame 操作和 Parquet 写入 |
| **arrow** | latest | Arrow 格式和 RecordBatch |
| **parquet** | latest | Parquet 编码和压缩 |
| **surrealdb** | latest | 数据库查询 |
| **anyhow** | latest | 错误处理 |
| **serde** | latest | 序列化/反序列化 |

### 8.2 数据格式

- **Parquet**: 列式存储格式，高压缩率，支持谓词下推
- **Arrow**: 内存中列式数据表示
- **ZSTD**: 高性能压缩算法

### 8.3 查询语言

- **SurrealQL**: SurrealDB 的 SQL 方言，支持图查询

---

## 9. 性能优化

### 9.1 批处理

- **批大小**: 1000 个 refno/批
- **原因**: 避免 SQL 语句过长，平衡内存和网络开销

### 9.2 去重策略

- **变换矩阵**: MD5 哈希，共享存储
- **包围盒**: MD5 哈希，共享存储
- **收益**: 减少 50-70% 存储空间

### 9.3 压缩配置

```rust
WriterProperties::builder()
    .set_compression(Compression::ZSTD(ZstdLevel::try_new(3).unwrap()))
    .build()
```

### 9.4 索引优化

- **refno**: 字符串和 u64 双存储，支持快速查找
- **geo_index**: 保持几何体顺序，支持范围查询

---

## 10. 前端集成示例

### 10.1 DuckDB-WASM 查询

```javascript
import * as duckdb from '@duckdb/duckdb-wasm';

// 初始化
const db = await duckdb.createDB();
const conn = await db.connect();

// 注册 Parquet 文件
await conn.query(`
  CREATE VIEW instances AS
  SELECT * FROM 'instances_7997.parquet';

  CREATE VIEW geo_instances AS
  SELECT * FROM 'geo_instances_7997.parquet';

  CREATE VIEW transforms AS
  SELECT * FROM 'transforms.parquet';
`);

// 查询某个 BRAN 的所有子实例
const result = await conn.query(`
  SELECT i.refno_str, i.noun, i.name,
         gi.geo_hash, t.t0, t.t1, ..., t.t15
  FROM instances i
  JOIN geo_instances gi ON i.refno_str = gi.refno_str
  JOIN transforms t ON i.trans_hash = t.trans_hash
  WHERE i.owner_refno_str = '24381/145018'
  ORDER BY i.refno_str, gi.geo_index
`);
```

### 10.2 空间范围查询

```javascript
// Frustum Culling
const visibleInstances = await conn.query(`
  SELECT i.refno_str, i.noun,
         a.min_x, a.min_y, a.min_z,
         a.max_x, a.max_y, a.max_z
  FROM instances i
  JOIN aabb a ON i.aabb_hash = a.aabb_hash
  WHERE a.min_x > ${frustum.minX} AND a.max_x < ${frustum.maxX}
    AND a.min_y > ${frustum.minY} AND a.max_y < ${frustum.maxY}
    AND a.min_z > ${frustum.minZ} AND a.max_z < ${frustum.maxZ}
`);
```

---

## 11. 故障排查

### 11.1 常见问题

#### Q1: 导出的 Parquet 文件为空

**原因**:
- 查询条件过滤掉了所有数据
- `world_trans` 或 `aabb` 字段为 `NONE`

**解决**:
```sql
-- 检查数据完整性
SELECT COUNT(*) FROM inst_relate
WHERE in.dbnum = 7997
  AND in->inst_relate_aabb[0].out != NONE;
```

#### Q2: 前端加载 Parquet 失败

**原因**:
- CORS 配置问题
- 文件路径错误

**解决**:
- 检查 Web 服务器 CORS 配置
- 确认文件路径正确

#### Q3: 内存溢出

**原因**:
- 一次性查询数据量过大

**解决**:
- 使用 `root_refno` 限制范围
- 调整批处理大小

### 11.2 调试技巧

```bash
# 启用详细日志
cargo run -- --export-dbnum-instances-parquet \
  --dbnum 7997 \
  --verbose

# 检查生成的文件
ls -lh output/database_models/7997/

# 使用 DuckDB CLI 验证
duckdb -c "SELECT COUNT(*) FROM 'instances_7997.parquet'"
```

---

## 12. 相关文档

- [导出命令使用说明](../EXPORT_COMMANDS.md)
- [Parquet DuckDB 验证计划](plans/2026-02-07-parquet-duckdb-verification.md)
- [模型生成流程分析](MODEL_GENERATION_FLOW_ANALYSIS.md)

---

## 13. 附录

### 13.1 Parquet Schema 定义

#### instances.parquet

```
message schema {
  required binary refno_str (UTF8);
  required int64 refno_u64;
  required binary noun (UTF8);
  required binary name (UTF8);
  optional binary owner_refno_str (UTF8);
  optional int64 owner_refno_u64;
  required binary owner_noun (UTF8);
  required binary trans_hash (UTF8);
  required binary aabb_hash (UTF8);
  required int64 spec_value;
  required boolean has_neg;
  required int32 dbnum;
}
```

#### transforms.parquet

```
message schema {
  required binary trans_hash (UTF8);
  required float t0;
  required float t1;
  ...
  required float t15;
}
```

### 13.2 文件大小参考

| 数据集 | instances | geo_instances | transforms | aabb | 总计 |
|--------|-----------|---------------|------------|------|------|
| dbnum=7997 (小型) | 2.3 MB | 3.1 MB | 1.2 MB | 0.8 MB | 7.4 MB |
| ref0=24381（映射 dbnum=7997，旧文误写为 dbnum=24381） | 45 MB | 68 MB | 12 MB | 8 MB | 133 MB |

### 13.3 查询性能参考

| 查询类型 | 数据量 | 耗时 |
|---------|--------|------|
| 全表扫描 | 100K 行 | ~50ms |
| 按 refno 过滤 | 1K 行 | ~5ms |
| JOIN 3 表 | 10K 行 | ~20ms |
| 空间范围查询 | 5K 行 | ~15ms |

---

**文档结束**

