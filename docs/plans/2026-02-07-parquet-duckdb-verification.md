# Parquet 导出 DuckDB 验证指南

> 适用于 `--export-dbnum-instances-parquet` / `--export-dbnum-instances` 导出的多表 Parquet 文件。

## 输出目录结构

```
output/<project>/instances_parquet/
├── instances_7997.parquet        # 实例表
├── geo_instances_7997.parquet    # 几何引用表
├── tubings_7997.parquet          # TUBI 管段表
├── transforms.parquet            # 共享变换矩阵表
├── aabb.parquet                  # 共享包围盒表
└── manifest_7997.json            # 导出元信息
```

## DuckDB 快速验证 SQL

### 前置：安装 DuckDB

```bash
# Windows (scoop)
scoop install duckdb

# 或直接下载：https://duckdb.org/docs/installation/
```

### 1. 行数统计

```sql
-- 查看各表行数
SELECT 'instances' as tbl, count(*) as rows FROM 'instances_7997.parquet'
UNION ALL
SELECT 'geo_instances', count(*) FROM 'geo_instances_7997.parquet'
UNION ALL
SELECT 'tubings', count(*) FROM 'tubings_7997.parquet'
UNION ALL
SELECT 'transforms', count(*) FROM 'transforms.parquet'
UNION ALL
SELECT 'aabb', count(*) FROM 'aabb.parquet';
```

### 2. 实例概览

```sql
-- 按 noun 统计实例分布
SELECT noun, count(*) as cnt
FROM 'instances_7997.parquet'
GROUP BY noun
ORDER BY cnt DESC;

-- 按 owner_noun 统计
SELECT owner_noun, count(*) as cnt
FROM 'instances_7997.parquet'
GROUP BY owner_noun
ORDER BY cnt DESC;
```

### 3. 几何引用分布

```sql
-- 每个实例有几个几何引用
SELECT refno_str, count(*) as geo_count
FROM 'geo_instances_7997.parquet'
GROUP BY refno_str
ORDER BY geo_count DESC
LIMIT 20;

-- geo_hash 去重数量（即不同几何体数量）
SELECT count(DISTINCT geo_hash) as unique_geo_count
FROM 'geo_instances_7997.parquet';
```

### 4. TUBI 管段

```sql
-- 每个 owner 的 TUBI 数量
SELECT owner_refno_str, count(*) as tubi_count
FROM 'tubings_7997.parquet'
GROUP BY owner_refno_str
ORDER BY tubi_count DESC
LIMIT 20;
```

### 5. 变换矩阵采样

```sql
-- 查看前 5 个变换矩阵
SELECT trans_hash, m00, m11, m22, m03, m13, m23
FROM 'transforms.parquet'
LIMIT 5;

-- 检查是否有单位矩阵
SELECT count(*) as identity_count
FROM 'transforms.parquet'
WHERE abs(m00 - 1.0) < 0.001
  AND abs(m11 - 1.0) < 0.001
  AND abs(m22 - 1.0) < 0.001
  AND abs(m03) < 0.001
  AND abs(m13) < 0.001
  AND abs(m23) < 0.001;
```

### 6. 包围盒范围

```sql
-- AABB 范围统计
SELECT
    min(min_x) as global_min_x,
    max(max_x) as global_max_x,
    min(min_y) as global_min_y,
    max(max_y) as global_max_y,
    min(min_z) as global_min_z,
    max(max_z) as global_max_z
FROM 'aabb.parquet';

-- AABB 尺寸分布
SELECT
    aabb_hash,
    (max_x - min_x) as width,
    (max_y - min_y) as height,
    (max_z - min_z) as depth
FROM 'aabb.parquet'
ORDER BY width * height * depth DESC
LIMIT 10;
```

### 7. 抽样 refno 对齐验证

```sql
-- 随机抽取 10 个 refno，检查 geo_instances 和 transforms 关联
SELECT
    i.refno_str,
    i.noun,
    i.owner_noun,
    i.trans_hash,
    i.aabb_hash,
    gi.geo_count,
    CASE WHEN t.trans_hash IS NOT NULL THEN true ELSE false END as trans_exists,
    CASE WHEN a.aabb_hash IS NOT NULL THEN true ELSE false END as aabb_exists
FROM 'instances_7997.parquet' i
LEFT JOIN (
    SELECT refno_str, count(*) as geo_count
    FROM 'geo_instances_7997.parquet'
    GROUP BY refno_str
) gi ON i.refno_str = gi.refno_str
LEFT JOIN 'transforms.parquet' t ON i.trans_hash = t.trans_hash
LEFT JOIN 'aabb.parquet' a ON i.aabb_hash = a.aabb_hash
USING SAMPLE 10;
```

### 8. 按 owner_refno 查询（模拟前端场景）

```sql
-- 查询某个 BRAN 下所有实例（前端典型用例）
SELECT i.refno_str, i.noun, i.name,
       gi.geo_hash, gi.geo_trans_hash
FROM 'instances_7997.parquet' i
JOIN 'geo_instances_7997.parquet' gi ON i.refno_str = gi.refno_str
WHERE i.owner_refno_str = '24381/145018'
ORDER BY i.refno_str, gi.geo_index;
```

### 9. 空间范围查询（模拟前端 frustum culling）

```sql
-- 查询某个空间范围内的实例
SELECT i.refno_str, i.noun,
       a.min_x, a.min_y, a.min_z,
       a.max_x, a.max_y, a.max_z
FROM 'instances_7997.parquet' i
JOIN 'aabb.parquet' a ON i.aabb_hash = a.aabb_hash
WHERE a.min_x > -10000 AND a.max_x < 10000
  AND a.min_y > -10000 AND a.max_y < 10000
LIMIT 20;
```

## CLI 用法示例

```bash
# 导出 dbnum=7997 的 Parquet（默认格式）
cargo run -- --export-dbnum-instances --dbnum 7997

# 显式指定 Parquet 格式
cargo run -- --export-dbnum-instances-parquet --dbnum 7997

# 仅导出某个 BRAN 的 visible 子孙
cargo run -- --export-dbnum-instances-parquet --dbnum 7997 --debug-model 24381/145018

# 指定输出目录
cargo run -- --export-dbnum-instances-parquet --dbnum 7997 --output ./my_output

# JSON 格式导出（旧方式，不受影响）
cargo run -- --export-dbnum-instances-json --dbnum 7997
```

## 前端 DuckDB-WASM 集成提示

```javascript
import * as duckdb from '@duckdb/duckdb-wasm';

// 初始化 DuckDB-WASM 后，注册 Parquet 文件
await db.registerFileURL('instances.parquet', url_to_instances_parquet);
await db.registerFileURL('geo_instances.parquet', url_to_geo_instances_parquet);
await db.registerFileURL('transforms.parquet', url_to_transforms_parquet);
await db.registerFileURL('aabb.parquet', url_to_aabb_parquet);

// 查询
const result = await conn.query(`
  SELECT i.refno_str, i.noun, gi.geo_hash
  FROM 'instances.parquet' i
  JOIN 'geo_instances.parquet' gi ON i.refno_str = gi.refno_str
  WHERE i.owner_refno_str = '24381/145018'
`);
```

## Parquet 表 Schema 参考

| 表名 | 列名 | 类型 | 说明 |
|------|------|------|------|
| **instances** | refno_str | UTF8 | 实例 refno 字符串 (如 "24381/100818") |
| | refno_u64 | UInt64 | refno 数值表示 |
| | noun | UTF8 | 类型 (EQUI/PIPE/...) |
| | name | UTF8 | 名称 |
| | owner_refno_str | UTF8? | 所属 owner refno |
| | owner_refno_u64 | UInt64? | owner refno 数值 |
| | owner_noun | UTF8 | owner 类型 (BRAN/HANG/EQUI) |
| | trans_hash | UTF8 | 世界变换矩阵 hash |
| | aabb_hash | UTF8 | 世界包围盒 hash |
| | spec_value | UInt64 | spec 值 |
| | has_neg | Boolean | 是否有布尔运算 |
| | dbnum | UInt32 | 数据库编号 |
| **geo_instances** | refno_str | UTF8 | 所属实例 refno |
| | refno_u64 | UInt64 | 所属实例 refno 数值 |
| | geo_index | UInt32 | 几何体序号 |
| | geo_hash | UTF8 | 几何体 hash |
| | geo_trans_hash | UTF8 | 几何体局部变换 hash |
| **tubings** | tubi_refno_str | UTF8 | TUBI 的 leave refno |
| | tubi_refno_u64 | UInt64 | TUBI leave refno 数值 |
| | owner_refno_str | UTF8 | 所属 BRAN/HANG refno |
| | owner_refno_u64 | UInt64 | 所属 BRAN/HANG 数值 |
| | order | UInt32 | TUBI 顺序索引 |
| | geo_hash | UTF8 | 几何体 hash |
| | trans_hash | UTF8 | 变换矩阵 hash |
| | aabb_hash | UTF8 | 包围盒 hash |
| | spec_value | UInt64 | spec 值 |
| | dbnum | UInt32 | 数据库编号 |
| **transforms** | trans_hash | UTF8 | 唯一 hash |
| | m00..m33 | Float64 | 4×4 矩阵列主序 (16 列) |
| **aabb** | aabb_hash | UTF8 | 唯一 hash |
| | min_x/y/z | Float64 | 包围盒最小点 |
| | max_x/y/z | Float64 | 包围盒最大点 |
