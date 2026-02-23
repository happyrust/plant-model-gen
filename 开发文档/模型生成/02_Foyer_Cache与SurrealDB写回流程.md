# Foyer Cache 生成后写回 SurrealDB 流程分析

## 概述

本文档详细说明模型生成过程中 Foyer Cache 与 SurrealDB 的数据写入流程，包括双写模式、布尔结果处理以及 Cache Flush 机制。

---

## 一、整体架构概览

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          模型生成阶段                                    │
│  orchestrator.rs::gen_all_geos_data()                                   │
│         │                                                               │
│         ▼                                                               │
│  ┌──────────────────┐                                                   │
│  │ ShapeInstancesData │ ◄── 几何生成结果                                │
│  └────────┬─────────┘                                                   │
│           │                                                             │
│     ┌─────┴─────┐                                                       │
│     ▼           ▼                                                       │
│ ┌────────┐  ┌────────────┐                                              │
│ │ Foyer  │  │ SurrealDB  │  ◄── 并行写入（由 use_surrealdb 控制）       │
│ │ Cache  │  │ 直接写入   │                                              │
│ └────────┘  └────────────┘                                              │
└─────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│                          布尔运算阶段                                    │
│  manifold_bool.rs::apply_boolean_for_query_cache()                      │
│         │                                                               │
│         ▼                                                               │
│  ┌──────────────────┐                                                   │
│  │ 布尔结果 (GLB)   │                                                   │
│  └────────┬─────────┘                                                   │
│           │                                                             │
│           ▼                                                             │
│  ┌────────────────────────┐                                             │
│  │ Cache 内更新            │                                            │
│  │ inst_relate_bool_map   │  ◄── 仅写入 Cache，不直接写 SurrealDB       │
│  └────────────────────────┘                                             │
└─────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│                          Cache Flush 阶段（可选）                        │
│  cache_flush.rs::flush_latest_instance_cache_to_surreal()               │
│         │                                                               │
│         ▼                                                               │
│  ┌──────────────────┐      ┌────────────────────┐                       │
│  │ 读取最新 Batch   │ ───► │ save_instance_data │                       │
│  │ from Foyer Cache │      │ _optimize()        │                       │
│  └──────────────────┘      └────────┬───────────┘                       │
│                                     │                                   │
│                                     ▼                                   │
│                            ┌────────────────────┐                       │
│                            │ SurrealDB 写入     │                       │
│                            │ + inst_relate_bool │                       │
│                            └────────────────────┘                       │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 二、核心数据结构

### 2.1 Cache 数据结构

**文件**: `src/fast_model/instance_cache.rs`

```rust
// Cache Key
pub struct InstanceCacheKey {
    pub dbnum: u32,        // 数据库编号
    pub batch_id: String,  // 批次 ID (格式: "{dbnum}_{seq}")
}

// Cache Value (序列化后的 Batch)
pub struct CachedInstanceBatch {
    pub dbnum: u32,
    pub batch_id: String,
    pub created_at: i64,
    pub inst_info_map: HashMap<RefnoEnum, EleGeosInfo>,      // 实例信息
    pub inst_geos_map: HashMap<String, EleInstGeosData>,     // 几何数据
    pub inst_tubi_map: HashMap<RefnoEnum, EleGeosInfo>,      // 管道信息
    pub neg_relate_map: HashMap<RefnoEnum, Vec<RefnoEnum>>,  // 负实体关系
    pub ngmr_neg_relate_map: HashMap<RefnoEnum, Vec<(RefnoEnum, RefnoEnum)>>,
    pub inst_relate_bool_map: HashMap<RefnoEnum, CachedInstRelateBool>,  // 布尔结果
}

// 布尔结果条目
pub struct CachedInstRelateBool {
    pub mesh_id: String,   // 布尔后的 mesh ID
    pub status: String,    // 状态："Success" | "Failed" | 其他
    pub created_at: i64,
}
```

### 2.2 SurrealDB 表结构

| 表名 | 类型 | 用途 |
|------|------|------|
| `inst_geo` | 普通表 | 几何单元（geo_hash 为主键） |
| `inst_info` | 普通表 | 实例元数据 |
| `inst_relate` | RELATION | pe → inst_info 关系 |
| `geo_relate` | RELATION | inst_info → inst_geo 关系 |
| `neg_relate` | RELATION | geo_relate → pe（切割几何 → 被切割正实体） |
| `ngmr_relate` | RELATION | geo_relate → pe（NGMR 切割几何 → 目标正实体） |
| `inst_relate_bool` | 普通表 | 布尔结果状态 |
| `inst_relate_cata_bool` | 普通表 | CATE 级布尔结果状态 |
| `inst_relate_aabb` | RELATION | pe → aabb 关系 |
| `aabb` | 普通表 | 包围盒数据 |

---

## 三、写回流程详解

### 3.1 模型生成期间的实时写入

**入口**: `src/fast_model/gen_model/orchestrator.rs`

```
gen_all_geos_data()
    │
    ├─► 启动 insert_task (tokio::spawn)
    │       │
    │       └─► while receiver.recv_async() {
    │               │
    │               ├─► [1] save_instance_data_optimize() → SurrealDB
    │               │       (当 use_surrealdb=true)
    │               │
    │               ├─► [2] cache_manager.insert_from_shape() → Foyer Cache
    │               │
    │               └─► [3] parquet_writer.write_batch() → Parquet
    │           }
    │
    └─► 等待所有 worker 完成
```

### 3.2 Foyer Cache 写入流程

**文件**: `src/fast_model/instance_cache.rs:134-160`

```
insert_from_shape(dbnum, shape_insts)
    │
    ├─► 生成 batch_id = "{dbnum}_{seq}"
    │
    ├─► 构造 CachedInstanceBatch {
    │       inst_info_map,
    │       inst_geos_map,
    │       inst_tubi_map,
    │       neg_relate_map,
    │       ngmr_neg_relate_map,
    │       inst_relate_bool_map: HashMap::new()  // 初始为空
    │   }
    │
    └─► insert_batch(batch)
            │
            ├─► serde_json::to_vec(&batch) → payload
            │
            ├─► HybridCache.insert(key, value)
            │       (内存 128MB + 磁盘 1GB)
            │
            └─► update_index(dbnum, batch_id)
                    │
                    └─► 写入 instance_cache_index.json
```

### 3.3 SurrealDB 直接写入流程

**文件**: `src/fast_model/pdms_inst.rs:125+`

```
save_instance_data_optimize(inst_mgr, replace_exist)
    │
    ├─► [Schema 迁移]
    │       ensure_inst_relate_relation_schema()
    │       ensure_inst_relate_aabb_relation_schema()
    │
    ├─► [replace_exist=true 时清理旧数据]
    │       delete_inst_relate_by_in()
    │       delete_inst_relate_bool_records()
    │       delete_inst_geo_by_hashes()
    │       delete_geo_relate_by_inst_info_ids()
    │       delete_boolean_relations_by_targets()
    │
    ├─► [写入 inst_geo & geo_relate]
    │       TransactionBatcher (CHUNK=100, MAX_TX=5, CONCURRENT=2)
    │       INSERT IGNORE INTO inst_geo [...]
    │       INSERT INTO geo_relate [...]
    │
    ├─► [写入 inst_info & inst_relate]
    │       UPSERT inst_info:⟨id⟩ CONTENT {...}
    │       RELATE pe:⟨refno⟩ -> inst_relate -> inst_info:⟨id⟩
    │
    ├─► [写入 neg_relate & ngmr_relate]
    │       RELATE pe:⟨neg_refno⟩ -> neg_relate -> pe:⟨target_refno⟩
    │       RELATE pe:⟨ngmr_refno⟩ -> ngmr_relate -> pe:⟨target_refno⟩
    │
    └─► [写入 aabb & inst_relate_aabb]
            UPSERT aabb:⟨hash⟩ SET d = {...}
            RELATE pe:⟨refno⟩ -> inst_relate_aabb -> aabb:⟨hash⟩
```

### 3.4 布尔结果写回 Cache

**文件**: `src/fast_model/foyer_cache/manifold_bool.rs`

```
apply_boolean_for_query_cache()
    │
    ├─► 执行 Manifold 布尔运算
    │
    ├─► 导出 GLB 文件
    │
    └─► cache_manager.upsert_inst_relate_bool(dbnum, batch_id, refno, mesh_id, status)
            │
            ├─► cache.get(dbnum, batch_id) → 读取现有 batch
            │
            ├─► batch.inst_relate_bool_map.insert(refno, CachedInstRelateBool{...})
            │
            └─► insert_batch(batch) → 回写整个 batch
```

### 3.5 Cache Flush 到 SurrealDB

**文件**: `src/fast_model/cache_flush.rs:16-111`

> **注意**：此函数仅同步每个 dbnum 的**最新 batch**，历史 batch 不会被同步。
> 如需完整同步，需在模型生成时启用 `use_surrealdb=true`。

```
flush_latest_instance_cache_to_surreal(cache_dir, dbnums, replace_exist, verbose)
    │
    ├─► InstanceCacheManager::new(cache_dir)
    │
    ├─► for dbnum in targets {
    │       │
    │       ├─► cache.list_batches(dbnum) → 获取所有 batch_id
    │       │
    │       ├─► latest_batch_id = batch_ids.last()
    │       │
    │       ├─► cache.get(dbnum, latest_batch_id) → CachedInstanceBatch
    │       │
    │       ├─► 构造 ShapeInstancesData
    │       │
    │       ├─► save_instance_data_optimize(&shape, replace_exist)
    │       │       └─► 写入 inst_geo, inst_info, inst_relate, geo_relate, ...
    │       │
    │       └─► for (refno, b) in inst_relate_bool_map {
    │               save_inst_relate_bool(refno, mesh_id, status, "cache_flush")
    │           }
    │   }
    │
    └─► return flushed_count
```

---

## 四、关键函数索引

| 功能 | 函数 | 文件位置 |
|------|------|----------|
| Cache 写入 | `insert_from_shape()` | `src/fast_model/instance_cache.rs:134` |
| Cache 读取 | `get()` | `src/fast_model/instance_cache.rs:162` |
| Cache 布尔更新 | `upsert_inst_relate_bool()` | `src/fast_model/instance_cache.rs:198` |
| SurrealDB 写入 | `save_instance_data_optimize()` | `src/fast_model/pdms_inst.rs:125` |
| 布尔状态写入 | `save_inst_relate_bool()` | `src/fast_model/utils.rs:116` |
| Cache Flush | `flush_latest_instance_cache_to_surreal()` | `src/fast_model/cache_flush.rs:16` |
| 模型生成编排 | `gen_all_geos_data()` | `src/fast_model/gen_model/orchestrator.rs` |

---

## 五、配置与控制

### 5.1 数据源模式控制（强互斥）

| 配置项 | 位置 | 说明 |
|--------|------|------|
| `use_cache` | `src/options.rs` | 启用 foyer cache 模式（此时 `use_surrealdb` 必须为 `false`） |
| `use_surrealdb` | `src/options.rs` | 启用 SurrealDB 模式（此时 `use_cache` 必须为 `false`） |
| `foyer_cache_dir` | `DbOptionExt` | Foyer Cache 目录路径 |

启动阶段会校验：`use_cache` 与 `use_surrealdb` 必须严格互斥，且恰好一个为 `true`。  
非法组合（同为 `true` 或同为 `false`）会直接失败并提示修正配置（fail-fast）。

### 5.2 并发控制参数

```rust
// pdms_inst.rs
const CHUNK_SIZE: usize = 100;           // 单条 INSERT 记录数
const MAX_TX_STATEMENTS: usize = 5;      // 事务内最大语句数
const MAX_CONCURRENT_TX: usize = 2;      // 最大并发事务数
```

### 5.3 Cache 容量配置

```rust
// instance_cache.rs
memory: 128 * 1024 * 1024,           // 内存缓存 128MB
capacity: 1024 * 1024 * 1024,        // 磁盘缓存 1GB
```

### 5.4 错误处理

| 场景 | 处理策略 |
|------|----------|
| Cache 序列化失败 | 跳过写入，打印错误日志 |
| Cache 反序列化失败 | 返回 None，打印错误日志 |
| SurrealDB 写入失败 | 通过 `TransactionBatcher` 内部重试机制处理 |
| 布尔结果更新时 batch 不存在 | 返回错误（`anyhow::bail!`），不静默失败 |
| NaN transform 检测 | 跳过该几何单元，打印警告日志 |

---

## 六、CLI 命令

```bash
# 将 Cache 数据 Flush 到 SurrealDB
gen_model flush-cache-to-db \
    --flush-cache-dbnums 1,2,3 \
    --flush-cache-replace
```

---

## 七、数据流时序图

```
时间 ──────────────────────────────────────────────────────────────────►

[模型生成阶段]
    │
    ├─► ShapeInstancesData 生成
    │       │
    │       ▼
    │   按模式二选一：
    │   - use_cache=true, use_surrealdb=false
    │     => Foyer Cache（insert_from_shape）
    │   - use_cache=false, use_surrealdb=true
    │     => SurrealDB（save_instance_data_optimize）
    │
[布尔运算阶段]
    │
    ├─► apply_boolean_for_query_cache()
    │       │
    │       ▼
    │   upsert_inst_relate_bool()
    │       │
    │       ▼
    │   CachedInstanceBatch
    │   (inst_relate_bool_map 已填充)
    │
[Cache Flush 阶段] (可选，手动触发)
    │
    └─► flush_latest_instance_cache_to_surreal()
            │
            ├─► save_instance_data_optimize()
            │
            └─► save_inst_relate_bool() × N
                    │
                    ▼
                SurrealDB
                inst_relate_bool 表
```

---

## 八、总结

1. **强互斥模式**: `use_cache` 与 `use_surrealdb` 必须二选一；不再允许“cache + SurrealDB 同时开启”

2. **Cache-Only 布尔**: 布尔运算结果仅写入 Cache 的 `inst_relate_bool_map`，不直接写 SurrealDB

3. **延迟落库**: 通过 `flush-cache-to-db` 命令可将 Cache 数据批量同步到 SurrealDB（显式手动触发）

4. **幂等写入**: SurrealDB 写入使用 `UPSERT` 和 `INSERT IGNORE` 保证幂等性

5. **事务控制**: 使用 `TransactionBatcher` 控制并发，避免 SurrealDB 连接超限

---

## 相关文档

- [01_数据表结构与保存流程.md](01_数据表结构与保存流程.md) - 数据表结构详解
- [布尔运算/01_架构概述.md](../布尔运算/01_架构概述.md) - 布尔运算架构

---

**文档版本**：v1.0
**最后更新**：2026-02-04
