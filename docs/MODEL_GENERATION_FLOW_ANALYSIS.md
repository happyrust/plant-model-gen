# 模型生成流程与 SurrealDB 写回分析

## 概述

本文档分析 `gen_model-dev` 项目中模型生成的完整流程，重点关注数据如何写回到 SurrealDB 数据库。

## 主要入口

### `gen_all_geos_data` (orchestrator.rs)

这是模型生成的主入口函数，根据配置路由到不同的生成策略：

1. **Full Noun 模式** (`process_full_noun_mode`)
2. **增量/手动/调试模式** (`process_targeted_generation`)
3. **全量数据库生成** (`process_full_database_generation`)

## 核心写回流程

### 1. 初始化阶段

```rust
// 初始化 SurrealDB 表结构
if db_option.use_surrealdb {
    aios_core::rs_surreal::inst::init_model_tables().await?;
}
```

### 2. 数据生成与传递

模型生成采用**生产者-消费者模式**：

- **生产者**：`gen_full_noun_geos_optimized` 或 `gen_geos_data` 生成 `ShapeInstancesData`
- **通道**：通过 `flume::unbounded` channel 异步传递数据
- **消费者**：独立的异步任务接收数据并写回数据库

```rust
let (sender, receiver) = flume::unbounded();
let insert_handle = tokio::spawn(async move {
    while let Ok(shape_insts) = receiver.recv_async().await {
        if use_surrealdb {
            save_instance_data_optimize(&shape_insts, replace_exist).await?;
        }
        // 同时写入 cache（如果启用）
        if let Some(ref cache_manager) = cache_manager_for_insert {
            cache_manager.insert_from_shape(dbnum, &shape_insts);
        }
    }
});
```

### 3. 核心写回函数：`save_instance_data_optimize`

位置：`src/fast_model/pdms_inst.rs`

这是实际写回 SurrealDB 的核心函数，采用**事务化批处理**机制。

#### 3.1 数据清理（replace_exist=true 时）

如果 `replace_exist=true`，会先删除旧数据：

```rust
// 1. 删除旧的 inst_relate（按 in=pe）
delete_inst_relate_by_in(&refnos, CHUNK_SIZE).await?;

// 2. 删除旧的 inst_relate_bool（布尔结果状态）
delete_inst_relate_bool_records(&refnos, CHUNK_SIZE).await?;

// 3. 删除旧的 inst_geo（按 geo_hash）
delete_inst_geo_by_hashes(&geo_hashes, CHUNK_SIZE).await?;

// 4. 删除旧的 geo_relate（关系表）
delete_geo_relate_by_inst_info_ids(&inst_info_ids, CHUNK_SIZE).await?;

// 5. 删除旧的 neg_relate/ngmr_relate（布尔关系）
delete_boolean_relations_by_targets(&bool_targets, CHUNK_SIZE).await?;
```

#### 3.2 数据写入顺序

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

3. **neg_relate** - 负实体关系（新结构）
   ```sql
   INSERT IGNORE RELATION INTO neg_relate [{...}];
   ```
   - `in`: geo_relate ID（切割几何）
   - `out`: 正实体 refno（被减实体）
   - `pe`: 负实体 refno（负载体）

4. **ngmr_relate** - NGMR 关系（新结构）
   ```sql
   INSERT IGNORE RELATION INTO ngmr_relate [{...}];
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

#### 3.3 事务批处理机制

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

### 4. 后续处理

#### 4.1 Mesh 生成

```rust
if db_option.inner.gen_mesh {
    run_mesh_worker(Arc::new(db_option.inner.clone()), 100).await?;
}
```

#### 4.2 AABB 更新

```rust
update_inst_relate_aabbs_by_refnos(&aabb_refnos, replace_exist).await?;
```

从 `inst_relate` + `geo_relate` 计算并更新 AABB。

#### 4.3 布尔运算

```rust
if db_option.inner.apply_boolean_operation {
    run_boolean_worker(Arc::new(db_option.inner.clone()), 100).await?;
}
```

## 关键数据表结构

### inst_relate（关系表）
- `in`: pe refno（PE 元素）
- `out`: inst_info ID（实例信息）
- 字段：generic_type, zone_refno, spec_value, has_cata_neg, solid, owner_refno, owner_type

### geo_relate（关系表）
- `in`: inst_info ID
- `out`: inst_geo ID
- `geom_refno`: PE refno
- `geo_type`: Pos/DesiPos/CatePos/Neg/CataCrossNeg
- `visible`: bool

### neg_relate（关系表，新结构）
- `in`: geo_relate ID（切割几何）
- `out`: 正实体 refno（被减实体）
- `pe`: 负实体 refno（负载体）

### ngmr_relate（关系表，新结构）
- `in`: geo_relate ID（CataCrossNeg 切割几何）
- `out`: 目标 refno（正实体）
- `pe`: ele_refno（负载体）
- `ngmr`: ngmr_geom_refno（NGMR 几何引用）

### inst_relate_aabb（关系表）
- `in`: pe refno
- `out`: aabb ID

## 性能优化

1. **批量处理**：使用 `CHUNK_SIZE=100` 批量插入
2. **事务合并**：每个事务最多 5 条语句
3. **并发控制**：最多 2 个并发事务
4. **去重优化**：使用 hash 去重 transform/aabb/vec3
5. **异步处理**：生成和写入并行执行

## 错误处理

1. **事务冲突**：自动重试（最多 8 次，指数退避）
2. **索引冲突**：自动修复 inst_relate_aabb 唯一索引
3. **NaN 检测**：跳过包含 NaN 的 transform
4. **空数据检查**：跳过空批次

## 缓存同步

除了写回 SurrealDB，数据还会同步写入 **foyer cache**：

```rust
if let Some(ref cache_manager) = cache_manager_for_insert {
    cache_manager.insert_from_shape(dbnum, &shape_insts);
}
```

缓存可用于：
- 快速导出（无需查询 SurrealDB）
- 增量更新
- 离线处理

## 总结

模型生成流程采用**异步管道**设计：

1. **生成阶段**：并行生成几何数据
2. **传输阶段**：通过 channel 异步传递
3. **写入阶段**：批量事务化写入 SurrealDB
4. **后处理**：Mesh 生成、AABB 更新、布尔运算

关键设计：
- ✅ 生产者-消费者模式，解耦生成和写入
- ✅ 事务批处理，提升写入性能
- ✅ 依赖顺序写入，保证数据一致性
- ✅ 自动重试机制，提升稳定性
- ✅ 双写机制（SurrealDB + Cache），提升可用性
