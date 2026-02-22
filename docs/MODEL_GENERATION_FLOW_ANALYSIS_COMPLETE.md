# 模型生成流程完整分析

## 概述

本文档详细分析 `gen_model-dev` 项目中模型生成的完整流程，包括入口路由、数据生成、数据库写回、后处理等各个环节。

## 1. 入口与路由

### 1.1 主入口函数：`gen_all_geos_data`

**位置**：`src/fast_model/gen_model/orchestrator.rs`

这是模型生成的主入口函数，根据配置和参数路由到不同的生成策略：

```rust
pub async fn gen_all_geos_data(
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    target_sesno: Option<u32>,
) -> Result<bool>
```

### 1.2 路由决策逻辑

根据以下条件决定生成路径：

1. **Index Tree 模式** (`db_option.index_tree_mode = true`)
   - 使用优化的 `gen_index_tree_geos_optimized` 管线
   - 按 NOUN 类型分类处理（Loop/Prim/Cate）
   - 支持并发批量处理

2. **增量/手动/调试模式** (`has_debug || has_manual_refnos || is_incr_update`)
   - 使用 `process_targeted_generation`
   - 支持增量更新、手动指定 refno、调试模式

3. **全量数据库生成**（默认）
   - 使用 `process_full_database_generation`
   - 按 dbnum 循环处理

### 1.3 预检查阶段

在生成前执行预检查（`precheck_coordinator`）：

- ✅ **Tree 文件检查**：确保 `output/scene_tree/{dbnum}.tree` 存在
- ✅ **pe_transform 检查**：确保变换数据就绪
- ✅ **db_meta_info 检查**：确保数据库元数据加载

```rust
let precheck_config = PrecheckConfig {
    enabled: true,
    check_tree: true,
    check_pe_transform: true,
    check_db_meta: true,
    tree_output_dir: "...",
};
run_precheck(db_option, Some(precheck_config)).await?;
```

## 2. 数据生成阶段

### 2.1 Index Tree 模式生成流程

**位置**：`src/fast_model/gen_model/index_tree_mode.rs`

#### 2.1.1 NOUN 分类

将 refno 按类型分类：

- **LoopOwner**：循环拥有者（如 PIPE、BRAN、HANG）
- **Prim**：基本几何体（如 CYL、BOX、SPHE）
- **Cate**：元件库（如 EQUI、STRU）

```rust
let categorized = CategorizedRefnos::new();
// 分类逻辑
categorized.add_loop_owner_refnos(...);
categorized.add_prim_refnos(...);
categorized.add_cate_refnos(...);
```

#### 2.1.2 并发处理策略

- **每次处理 2 个 NOUN 类型**（并发度可配置）
- **批次大小**：`CHUNK_SIZE = 100`
- **使用 TreeIndex** 加速层级查询

```rust
for chunk in noun_infos.chunks(2) {
    // 并发处理本批次的 NOUN 类型
    let handles: Vec<_> = chunk.iter().map(|info| {
        tokio::spawn(async move {
            process_single_noun_type(...).await
        })
    }).collect();
}
```

#### 2.1.3 几何体生成

每个 NOUN 类型通过对应的 processor 生成几何：

- **LoopProcessor**：处理 PIPE、BRAN、HANG 等
- **PrimProcessor**：处理 CYL、BOX、SPHE 等
- **CateProcessor**：处理 EQUI、STRU 等元件库

### 2.2 非 Index Tree 模式生成流程

**位置**：`src/fast_model/gen_model/non_index_tree.rs`

- 支持增量更新（`incr_updates`）
- 支持手动 refno 列表
- 支持调试模式（`debug_model_refnos`）

## 3. 数据写回阶段

### 3.1 生产者-消费者模式

采用异步管道设计，解耦生成和写入：

```rust
let (sender, receiver) = flume::unbounded();

// 生产者：生成几何数据
gen_index_tree_geos_optimized(..., sender.clone()).await?;

// 消费者：异步写回数据库
let insert_handle = tokio::spawn(async move {
    while let Ok(shape_insts) = receiver.recv_async().await {
        save_instance_data_optimize(&shape_insts, replace_exist).await?;
        // 同时写入 cache（如果启用）
        cache_manager.insert_from_shape(dbnum, &shape_insts);
    }
});
```

### 3.2 核心写回函数：`save_instance_data_optimize`

**位置**：`src/fast_model/pdms_inst.rs`

这是实际写回 SurrealDB 的核心函数，采用**事务化批处理**机制。

#### 3.2.1 数据清理（replace_exist=true）

如果 `replace_exist=true`，会先删除旧数据：

1. **删除 inst_relate**（按 `in=pe`）
   ```sql
   DELETE FROM inst_relate WHERE in IN [...];
   ```

2. **删除 inst_relate_bool**（布尔结果状态）
   ```sql
   DELETE [inst_relate_bool:⟨refno⟩, ...];
   ```

3. **删除 inst_geo**（按 `geo_hash`）
   ```sql
   DELETE FROM inst_geo WHERE id IN [...];
   ```

4. **删除 geo_relate**（关系表）
   ```sql
   DELETE geo_relate WHERE in IN [...];
   ```

5. **删除 neg_relate/ngmr_relate**（布尔关系）
   ```sql
   DELETE neg_relate WHERE out IN [...];
   DELETE ngmr_relate WHERE out IN [...];
   ```

#### 3.2.2 数据写入顺序（依赖顺序）

写入采用**依赖顺序**，确保外键关系正确：

1. **inst_geo** - 几何体数据
   ```sql
   INSERT IGNORE INTO inst_geo [{...}];
   ```

2. **geo_relate** - 几何关系（关系表）
   ```sql
   INSERT RELATION INTO geo_relate [{...}];
   ```
   - `in`: inst_info ID
   - `out`: inst_geo ID
   - `geom_refno`: PE refno
   - `geo_type`: Pos/DesiPos/CatePos/Neg/CataCrossNeg

3. **neg_relate** - 负实体关系
   ```sql
   INSERT RELATION IGNORE INTO neg_relate [{...}];
   ```
   - `in`: geo_relate ID（切割几何）
   - `out`: 正实体 refno（被减实体）
   - `pe`: 负实体 refno（负载体）

4. **ngmr_relate** - NGMR 关系
   ```sql
   INSERT RELATION IGNORE INTO ngmr_relate [{...}];
   ```
   - `in`: geo_relate ID（CataCrossNeg 切割几何）
   - `out`: 目标 refno（正实体）
   - `pe`: ele_refno（负载体）
   - `ngmr`: ngmr_geom_refno（NGMR 几何引用）

5. **inst_info** - 实例信息
   ```sql
   INSERT IGNORE INTO inst_info [{...}];
   ```

6. **inst_relate** - 实例关系（关系表）
   ```sql
   INSERT RELATION INTO inst_relate [{...}];
   ```
   - `in`: pe refno
   - `out`: inst_info ID
   - 包含：generic_type, zone_refno, spec_value, has_cata_neg, solid, owner_refno 等

7. **aabb** - 包围盒数据
   ```sql
   INSERT IGNORE INTO aabb [{...}];
   ```

8. **trans** - 变换矩阵
   ```sql
   INSERT IGNORE INTO trans [{...}];
   ```

9. **vec3** - 三维向量
   ```sql
   INSERT IGNORE INTO vec3 [{...}];
   ```

10. **inst_relate_aabb** - 实例 AABB 关系（关系表）
    ```sql
    INSERT RELATION INTO inst_relate_aabb [{...}];
    ```
    - `in`: pe refno
    - `out`: aabb ID
    - **注意**：必须在 aabb 写入之后执行，避免 out 侧空记录

#### 3.2.3 事务批处理机制

使用 `TransactionBatcher` 进行批量事务处理：

```rust
struct TransactionBatcher {
    max_statements: usize,      // 每个事务最多 5 条语句
    max_concurrent: usize,       // 最多 2 个并发事务
    pending: Vec<String>,        // 待执行的 SQL 语句
    tasks: FuturesUnordered<...>, // 异步任务队列
}
```

**特点**：
- 批量合并多条 SQL 到单个事务块
- 自动重试机制（最多 8 次，指数退避）
- 处理事务冲突（Transaction conflict）
- 修复索引冲突（inst_relate_aabb 唯一索引）

**事务块格式**：
```sql
BEGIN TRANSACTION;
INSERT IGNORE INTO inst_geo [...];
INSERT RELATION INTO geo_relate [...];
...
COMMIT TRANSACTION;
```

#### 3.2.4 性能优化

- **批量处理**：使用 `CHUNK_SIZE=100` 批量插入
- **事务合并**：每个事务最多 5 条语句
- **并发控制**：最多 2 个并发事务
- **去重优化**：使用 hash 去重 transform/aabb/vec3
- **异步处理**：生成和写入并行执行

## 4. 后处理阶段

### 4.1 Mesh 生成

**位置**：`src/fast_model/mesh_generate.rs`

```rust
if db_option.inner.gen_mesh {
    // 从 foyer cache 生成 mesh（如果启用）
    if let Some(ref ctx) = foyer_cache_ctx {
        crate::fast_model::foyer_cache::mesh::run_mesh_worker(
            ctx, &mesh_dir, &mesh_precision, &mesh_formats
        ).await?;
    }
    
    // 从 SurrealDB 生成 mesh（如果启用）
    if use_surrealdb {
        run_mesh_worker(Arc::new(db_option.inner.clone()), 100).await?;
    }
}
```

**Mesh 生成流程**：
1. 查询需要生成 mesh 的实例（从 `inst_relate` + `geo_relate`）
2. 读取几何数据（从 cache 或 SurrealDB）
3. 调用 Manifold 库生成三角网格
4. 保存 mesh 文件（OBJ/GLB/GLTF 等格式）

### 4.2 AABB 更新

**位置**：`src/fast_model/mesh_generate.rs`

```rust
update_inst_relate_aabbs_by_refnos(&aabb_refnos, replace_exist).await?;
```

**AABB 计算流程**：
1. 从 `inst_relate` + `geo_relate` 收集所有几何体
2. 计算每个实例的包围盒（AABB）
3. 更新 `inst_relate_aabb` 关系表

### 4.3 布尔运算

**位置**：`src/fast_model/mesh_generate.rs`

```rust
if db_option.inner.apply_boolean_operation {
    // 从 foyer cache 执行布尔运算（如果启用）
    if let Some(ref ctx) = foyer_cache_ctx {
        crate::fast_model::foyer_cache::boolean::run_boolean_worker(ctx).await?;
    }
    
    // 从 SurrealDB 执行布尔运算（如果启用）
    if use_surrealdb {
        run_boolean_worker(Arc::new(db_option.inner.clone()), 100).await?;
    }
}
```

**布尔运算流程**：
1. 查询需要布尔运算的实例（从 `neg_relate` / `ngmr_relate`）
2. 读取正实体和负实体的 mesh
3. 调用 Manifold 库执行布尔运算
4. 更新 `geo_relate` 的 `geo_type`（Pos -> CatePos）
5. 保存布尔结果 mesh

### 4.4 Web Bundle 生成

**位置**：`src/fast_model/export_model/export_prepack_lod.rs`

```rust
if db_option.mesh_formats.contains(&MeshFormat::Glb) {
    export_prepack_lod_for_refnos(
        &all_refnos, &mesh_dir, &output_dir, ...
    ).await?;
}
```

**Web Bundle 内容**：
- GLB 文件（3D 模型）
- JSON 数据包（实例信息、材质、层级关系等）

### 4.5 SQLite 空间索引生成

**位置**：`src/fast_model/gen_model/orchestrator.rs`

```rust
update_sqlite_spatial_index_from_cache(db_option, &touched_dbnums_vec).await?;
```

**索引生成流程**：
1. 从 foyer cache 导出 `instances_{dbnum}.json`
2. 导入到 SQLite RTree 索引
3. 用于房间计算等空间查询的粗筛

### 4.6 Instances JSON 导出

**位置**：`src/fast_model/export_model/export_prepack_lod.rs`

```rust
if db_option.export_instances {
    export_instances_json_for_dbnos(
        &dbnos, mesh_dir, &output_dir, ...
    ).await?;
}
```

**导出内容**：
- `instances_{dbnum}.json`：实例列表
- `aabb.json`：包围盒数据
- `trans.json`：变换矩阵

## 5. 关键数据表结构

### 5.1 inst_relate（关系表）

PE 元素到实例信息的关系：

- `in`: pe refno（PE 元素）
- `out`: inst_info ID（实例信息）
- 字段：generic_type, zone_refno, spec_value, has_cata_neg, solid, owner_refno, owner_type

### 5.2 geo_relate（关系表）

实例信息到几何体的关系：

- `in`: inst_info ID
- `out`: inst_geo ID
- `geom_refno`: PE refno
- `geo_type`: Pos/DesiPos/CatePos/Neg/CataCrossNeg
- `visible`: bool

### 5.3 neg_relate（关系表）

负实体切割关系：

- `in`: geo_relate ID（切割几何）
- `out`: 正实体 refno（被减实体）
- `pe`: 负实体 refno（负载体）

### 5.4 ngmr_relate（关系表）

NGMR 切割关系：

- `in`: geo_relate ID（CataCrossNeg 切割几何）
- `out`: 目标 refno（正实体）
- `pe`: ele_refno（负载体）
- `ngmr`: ngmr_geom_refno（NGMR 几何引用）

### 5.5 inst_relate_aabb（关系表）

PE 元素到包围盒的关系：

- `in`: pe refno
- `out`: aabb ID

## 6. 缓存机制

### 6.1 Foyer Cache

**位置**：`src/fast_model/foyer_cache/`

**缓存内容**：
- 几何数据（`geos/`）
- Mesh 数据（`mesh/`）
- 布尔运算结果（`boolean/`）
- 点集数据（`ptset/`）

**缓存优势**：
- 快速导出（无需查询 SurrealDB）
- 增量更新
- 离线处理

### 6.2 双写机制

数据同时写入 SurrealDB 和 Cache：

```rust
if use_surrealdb {
    save_instance_data_optimize(&shape_insts, replace_exist).await?;
}
if let Some(ref cache_manager) = cache_manager_for_insert {
    cache_manager.insert_from_shape(dbnum, &shape_insts);
}
```

## 7. 错误处理

### 7.1 事务冲突

- 自动重试（最多 8 次，指数退避）
- 处理 "Transaction conflict: Resource busy" 错误

### 7.2 索引冲突

- 自动修复 `inst_relate_aabb` 唯一索引冲突

### 7.3 NaN 检测

- 跳过包含 NaN 的 transform

### 7.4 空数据检查

- 跳过空批次

## 8. 性能优化策略

### 8.1 并发处理

- Index Tree 模式：每次处理 2 个 NOUN 类型（可配置）
- 事务批处理：最多 2 个并发事务
- 生成和写入并行执行

### 8.2 批量处理

- 批量插入：`CHUNK_SIZE=100`
- 事务合并：每个事务最多 5 条语句

### 8.3 索引优化

- 使用 TreeIndex 加速层级查询
- 使用 SQLite RTree 加速空间查询

### 8.4 缓存优化

- Foyer cache 提供快速访问
- 去重优化（transform/aabb/vec3）

## 9. 流程图

```
┌─────────────────────────────────────────────────────────────┐
│                    gen_all_geos_data                        │
│                    (主入口函数)                              │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
            ┌───────────────────────┐
            │    路由决策            │
            └───────┬───────────────┘
                    │
        ┌───────────┼───────────┐
        │           │           │
        ▼           ▼           ▼
   Index Tree   增量/手动/调试   全量数据库
    模式          模式           生成
        │           │           │
        └───────────┼───────────┘
                    │
                    ▼
        ┌───────────────────────┐
        │   预检查阶段            │
        │  - Tree 文件           │
        │  - pe_transform        │
        │  - db_meta_info        │
        └───────────┬───────────┘
                    │
                    ▼
        ┌───────────────────────┐
        │   数据生成阶段          │
        │  - 几何体生成          │
        │  - 通过 channel 传递   │
        └───────────┬───────────┘
                    │
                    ▼
        ┌───────────────────────┐
        │   数据写回阶段          │
        │  - SurrealDB 写入      │
        │  - Cache 写入          │
        └───────────┬───────────┘
                    │
                    ▼
        ┌───────────────────────┐
        │   后处理阶段            │
        │  - Mesh 生成           │
        │  - AABB 更新           │
        │  - 布尔运算            │
        │  - Web Bundle 导出     │
        │  - SQLite 索引生成     │
        └───────────────────────┘
```

## 10. 关键配置项

### 10.1 DbOption 配置

- `index_tree_mode`: 是否启用 Index Tree 模式
- `use_surrealdb`: 是否写入 SurrealDB
- `use_cache`: 是否使用缓存
- `gen_mesh`: 是否生成 mesh
- `apply_boolean_operation`: 是否执行布尔运算
- `replace_exist`: 是否替换已存在的数据

### 10.2 Index Tree 配置

- `concurrency`: 并发度（默认 2）
- `batch_size`: 批次大小（默认 100）
- `enabled_categories`: 启用的 NOUN 类别

## 11. 总结

模型生成流程采用**异步管道**设计：

1. **生成阶段**：并行生成几何数据
2. **传输阶段**：通过 channel 异步传递
3. **写入阶段**：批量事务化写入 SurrealDB
4. **后处理**：Mesh 生成、AABB 更新、布尔运算

**关键设计**：
- ✅ 生产者-消费者模式，解耦生成和写入
- ✅ 事务批处理，提升写入性能
- ✅ 依赖顺序写入，保证数据一致性
- ✅ 自动重试机制，提升稳定性
- ✅ 双写机制（SurrealDB + Cache），提升可用性
- ✅ 并发处理，提升整体性能
