# Web Bundle 导出与 Parquet 导出流程图

## 概述

本文档详细说明模型生成流程中的两种导出方案：
1. **Web Bundle 导出**：生成 GLB + JSON 数据包，用于前端 Web 渲染
2. **Parquet 导出**：生成 Parquet 文件，用于数据分析和大规模查询

## 1. Web Bundle 导出流程

### 1.1 触发时机

在模型生成完成后（Index Tree 模式），如果配置了 `mesh_formats.contains(&MeshFormat::Glb)`：

```rust
if db_option.mesh_formats.contains(&MeshFormat::Glb) {
    export_prepack_lod_for_refnos(...).await?;
}
```

### 1.2 导出流程

```
┌─────────────────────────────────────────────────────────────┐
│            Web Bundle 导出流程                                │
│         (export_prepack_lod_for_refnos)                      │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  1. 收集导出 refno            │
        │  - 展开子孙节点               │
        │  - 收集 EQUI 子组件           │
        │  - 收集 BRAN/HANG owner      │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  2. 按 LOD 级别导出 GLB       │
        │  - L1 (默认)                 │
        │  - L2, L3 (可选)             │
        │  - 生成 geometry_L1.glb      │
        │  - 生成 geometry_L2.glb      │
        │  - 生成 geometry_L3.glb      │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  3. 查询几何体实例            │
        │  - 从 inst_relate 查询        │
        │  - 从 geo_relate 查询        │
        │  - 收集所有几何体引用         │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  4. 生成实例数据              │
        │  - 按 zone 分组              │
        │  - 生成 ComponentGroup       │
        │  - 生成 HierarchyGroup       │
        │  - 生成 TubingInstance       │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  5. 生成几何体清单            │
        │  - geometry_manifest.json    │
        │  - 记录 LOD 信息              │
        │  - 记录 mesh 索引             │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  6. 生成实例 JSON 文件         │
        │  - instances_{zone}.json      │
        │  - 包含颜色、名称、变换等      │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  7. 生成总清单                │
        │  - manifest.json             │
        │  - 记录所有资产信息            │
        └──────────────┬───────────────┘
                       │
                       ▼
                   完成导出
```

### 1.3 输出文件结构

```
web_bundle/
├── geometry_L1.glb          # L1 级别几何体
├── geometry_L2.glb          # L2 级别几何体（可选）
├── geometry_L3.glb          # L3 级别几何体（可选）
├── geometry_manifest.json   # 几何体清单
├── instances_zone1.json     # Zone 1 实例数据
├── instances_zone2.json     # Zone 2 实例数据
├── ...
└── manifest.json            # 总清单
```

### 1.4 关键数据结构

**ComponentGroup**（构件分组）：
```json
{
  "refno": "24381/1",
  "noun": "EQUI",
  "name": "设备名称",
  "color_index": 0,
  "name_index": 0,
  "lod_mask": 7,
  "spec_value": 100,
  "instances": [
    {
      "geo_hash": "123456",
      "matrix": [1, 0, 0, 0, 0, 1, 0, 0, ...],
      "geo_index": 0
    }
  ]
}
```

**GeometryManifest**（几何体清单）：
```json
{
  "version": 1,
  "generated_at": "2026-02-08T10:00:00Z",
  "shared_geometries": [
    {
      "geo_hash": "123456",
      "lod_levels": ["L1", "L2"],
      "file": "geometry_L1.glb",
      "mesh_index": 0
    }
  ],
  "dedicated_geometries": [...]
}
```

## 2. Parquet 导出流程

### 2.1 两种 Parquet 导出方案

#### 方案 A：流式写入（ParquetStreamWriter）

在模型生成过程中实时写入 Parquet：

```
┌─────────────────────────────────────────────────────────────┐
│         Parquet 流式写入流程                                  │
│         (ParquetStreamWriter)                                │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  1. 初始化写入器               │
        │  - 创建 output/database_models│
        │  - 按 dbnum 创建子目录         │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  2. 接收 ShapeInstancesData   │
        │  - 从 channel 接收批次         │
        │  - 提取 dbnum                 │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  3. 提取行数据                │
        │  - InstanceRow               │
        │  - TransformRow              │
        │  - GeoItem (嵌套结构)         │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  4. 创建 DataFrame            │
        │  - instances DataFrame        │
        │  - transforms DataFrame       │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  5. 写入增量文件               │
        │  - instance_{timestamp}.parquet│
        │  - transform_{timestamp}.parquet│
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  6. 最终合并                  │
        │  - 合并所有增量文件            │
        │  - 去重（保留最新）            │
        │  - 生成主文件                 │
        │  - instance.parquet          │
        │  - transform.parquet         │
        └──────────────┬───────────────┘
                       │
                       ▼
                   完成导出
```

**输出文件结构**：
```
output/database_models/
├── {dbnum}/
│   ├── instance.parquet          # 主文件（合并后）
│   ├── transform.parquet        # 主文件（合并后）
│   ├── instance_20260208_100000_123456.parquet  # 增量文件
│   ├── instance_20260208_100001_234567.parquet  # 增量文件
│   └── ...
```

#### 方案 B：批量导出（export_dbnum_instances_parquet）

从 SurrealDB 批量查询并导出：

```
┌─────────────────────────────────────────────────────────────┐
│         Parquet 批量导出流程                                  │
│         (export_dbnum_instances_parquet)                    │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  1. 查询实例数据              │
        │  - 从 inst_relate 查询        │
        │  - 从 geo_relate 查询        │
        │  - 从 tubi_relate 查询       │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  2. 查询变换和包围盒          │
        │  - 从 trans 表查询           │
        │  - 从 aabb 表查询            │
        │  - 去重处理                  │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  3. 构建行数据                │
        │  - InstanceRow               │
        │  - GeoInstanceRow             │
        │  - TubingRow                  │
        │  - TransformRow               │
        │  - AabbRow                    │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  4. 创建 RecordBatch          │
        │  - instances RecordBatch      │
        │  - geo_instances RecordBatch  │
        │  - tubings RecordBatch        │
        │  - transforms RecordBatch     │
        │  - aabb RecordBatch           │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  5. 写入 Parquet 文件         │
        │  - instances.parquet         │
        │  - geo_instances.parquet      │
        │  - tubings.parquet            │
        │  - transforms.parquet        │
        │  - aabb.parquet               │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  6. 生成 manifest.json         │
        │  - 记录元信息                 │
        │  - 记录文件大小               │
        └──────────────┬───────────────┘
                       │
                       ▼
                   完成导出
```

**输出文件结构**：
```
output/database_models/{dbnum}/
├── instances.parquet         # 实例表
├── geo_instances.parquet    # 几何实例表
├── tubings.parquet          # 管道表
├── transforms.parquet       # 变换表
├── aabb.parquet             # 包围盒表
└── manifest.json            # 元信息
```

### 2.2 Parquet Schema

**instances.parquet**：
- `refno_str`: String
- `refno_u64`: UInt64
- `noun`: String
- `name`: String
- `owner_refno_str`: String (nullable)
- `owner_refno_u64`: UInt64 (nullable)
- `owner_noun`: String
- `trans_hash`: String
- `aabb_hash`: String
- `spec_value`: Int64
- `has_neg`: Boolean
- `dbnum`: UInt32

**geo_instances.parquet**：
- `refno_str`: String
- `refno_u64`: UInt64
- `geo_index`: UInt32
- `geo_hash`: String
- `geo_trans_hash`: String

**transforms.parquet**：
- `trans_hash`: String
- `m00` ~ `m33`: Float64 (16 个分量)

**aabb.parquet**：
- `aabb_hash`: String
- `min_x`, `min_y`, `min_z`: Float64
- `max_x`, `max_y`, `max_z`: Float64

## 3. 集成到模型生成流程

### 3.1 在 orchestrator.rs 中的集成

```rust
// Index Tree 模式
async fn process_full_noun_mode(...) -> Result<bool> {
    // 1. 初始化 Parquet 写入器（可选）
    let parquet_writer = ParquetStreamWriter::new(&parquet_dir)?;
    
    // 2. 数据生成和写回
    let insert_handle = tokio::spawn(async move {
        while let Ok(shape_insts) = receiver.recv_async().await {
            // 写入 SurrealDB
            save_instance_data_optimize(&shape_insts, replace_exist).await?;
            
            // 写入 Cache
            cache_manager.insert_from_shape(dbnum, &shape_insts);
            
            // 写入 Parquet（流式）
            parquet_writer.write_batch(&shape_insts)?;
        }
    });
    
    // 3. 生成几何数据
    gen_full_noun_geos_optimized(..., sender.clone()).await?;
    
    // 4. 完成 Parquet 写入
    parquet_writer.finalize()?;
    
    // 5. Mesh 生成
    if db_option.inner.gen_mesh {
        run_mesh_worker(...).await?;
    }
    
    // 6. Web Bundle 导出
    if db_option.mesh_formats.contains(&MeshFormat::Glb) {
        export_prepack_lod_for_refnos(...).await?;
    }
    
    // 7. Instances JSON 导出
    if db_option.export_instances {
        export_instances_json_for_dbnos(...).await?;
    }
}
```

### 3.2 完整流程图

```
┌─────────────────────────────────────────────────────────────┐
│              模型生成完整流程                                  │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  1. 数据生成阶段                │
        │  - 生成几何数据                 │
        │  - 通过 channel 传递            │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  2. 数据写回阶段                │
        │  ├─ SurrealDB 写入             │
        │  ├─ Cache 写入                 │
        │  └─ Parquet 流式写入 (可选)    │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  3. Mesh 生成阶段              │
        │  - 生成三角网格                │
        │  - 保存 GLB/OBJ 文件           │
        └──────────────┬───────────────┘
                       │
                       ▼
        ┌──────────────────────────────┐
        │  4. 导出阶段                  │
        │  ├─ Web Bundle 导出 (可选)    │
        │  │  └─ GLB + JSON 数据包      │
        │  ├─ Parquet 批量导出 (可选)   │
        │  │  └─ 从 SurrealDB 查询导出   │
        │  └─ Instances JSON 导出 (可选)│
        │     └─ instances_{dbnum}.json │
        └──────────────┬───────────────┘
                       │
                       ▼
                   完成
```

## 4. 性能对比

### 4.1 Web Bundle 导出

**优点**：
- ✅ 前端可直接使用（GLB + JSON）
- ✅ 支持多 LOD 级别
- ✅ 按 zone 分组，便于加载

**缺点**：
- ❌ 文件较大（GLB 包含完整 mesh）
- ❌ 不适合大规模数据分析

### 4.2 Parquet 流式写入

**优点**：
- ✅ 实时写入，不占用额外内存
- ✅ 增量文件机制，支持断点续传
- ✅ 自动合并和去重

**缺点**：
- ❌ 需要最终合并步骤
- ❌ 增量文件可能较多

### 4.3 Parquet 批量导出

**优点**：
- ✅ 一次性生成完整文件
- ✅ 支持多表关联查询
- ✅ 适合数据分析场景

**缺点**：
- ❌ 需要从 SurrealDB 查询，耗时较长
- ❌ 内存占用较大

## 5. 使用建议

### 5.1 Web Bundle 导出

**适用场景**：
- 前端 Web 渲染
- 需要多 LOD 支持
- 需要按 zone 分组加载

**配置**：
```toml
mesh_formats = ["Glb"]
export_all_lods = false  # 仅导出 L1
```

### 5.2 Parquet 流式写入

**适用场景**：
- 模型生成过程中实时导出
- 大规模数据导出
- 需要增量更新

**配置**：
```rust
let parquet_writer = ParquetStreamWriter::new(&parquet_dir)?;
// 在数据写回时调用
parquet_writer.write_batch(&shape_insts)?;
```

### 5.3 Parquet 批量导出

**适用场景**：
- 从 SurrealDB 导出完整数据
- 数据分析和大规模查询
- 需要多表关联

**配置**：
```rust
export_dbnum_instances_parquet(
    dbnum,
    &output_dir,
    &db_option
).await?;
```

## 6. 总结

两种导出方案各有优势：

- **Web Bundle**：适合前端渲染，提供完整的 GLB + JSON 数据包
- **Parquet**：适合数据分析，提供高效的列式存储格式

根据实际需求选择合适的导出方案，或同时使用两种方案以满足不同场景的需求。
