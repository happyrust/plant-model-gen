# Instance 缓存层设计方案

> 目标：通过本地缓存层减轻模型生成过程中 SurrealDB 的写入压力，实现流水线并行处理

## 一、背景与问题

### 1.1 当前架构瓶颈

```
┌─────────────────┐    flume channel    ┌──────────────────────────┐
│ 几何体生成      │ ───────────────────→ │ insert_task (异步)       │
│ (prim/cata/loop)│   ShapeInstancesData │ └→ save_instance_data()  │
└─────────────────┘                      └──────────────────────────┘
                                                    ↓ 同步阻塞
                                         ┌──────────────────────────┐
                                         │ SurrealDB 写入           │
                                         └──────────────────────────┘
                                                    ↓ 串行等待
                                         ┌──────────────────────────┐
                                         │ mesh_worker / bool_worker│
                                         └──────────────────────────┘
```

**问题：**
1. SurrealDB 写入是同步阻塞的，几何生成需等待数据库 I/O
2. mesh_worker/bool_worker 依赖数据库状态，必须等写入完成
3. 流水线未充分并行，整体吞吐量受限

### 1.2 优化目标

- 几何生成与数据库写入解耦
- mesh_worker 可直接从缓存读取，无需扫库
- 实现真正的流水线并行处理

---

## 二、数据依赖分析

### 2.1 ShapeInstancesData 结构

```rust
pub struct ShapeInstancesData {
    pub inst_info_map: DashMap<RefnoEnum, EleGeosInfo>,
    pub inst_geos_map: DashMap<String, EleInstGeosData>,
    pub inst_tubi_map: DashMap<RefnoEnum, EleGeosInfo>,
    pub neg_relate_map: HashMap<RefnoEnum, Vec<RefnoEnum>>,
    pub ngmr_neg_relate_map: HashMap<RefnoEnum, Vec<(RefnoEnum, RefnoEnum)>>,
}
```

### 2.2 mesh_worker 数据依赖

| 字段 | 来源 | 缓存可提供 |
|------|------|-----------|
| geo_hash | EleInstGeo.geo_hash | ✅ |
| geo_param | EleInstGeo.geo_param | ✅ |
| transform | EleInstGeo.transform | ✅ |
| unit_flag | EleInstGeo.unit_flag | ✅ |
| meshed 状态 | 数据库 inst_geo.meshed | ❌ 需本地追踪 |

**结论：mesh_worker 所需数据 100% 可从缓存获取**

### 2.3 bool_worker 数据依赖

| 字段 | 来源 | 缓存可提供 |
|------|------|-----------|
| neg_relate_map | ShapeInstancesData | ✅ |
| ngmr_neg_relate_map | ShapeInstancesData | ✅ |
| 正实体 world_transform | EleGeosInfo | ✅ |
| 正实体 geo_type | EleInstGeo | ✅ |
| 负实体 geo_param | 需查询 inst_geo | ⚠️ 需扩展 |
| 负实体 transform | 需查询 geo_relate | ⚠️ 需扩展 |
| 负实体 mesh 文件 | 磁盘文件 | ✅ |

**结论：bool_worker 需要扩展缓存结构以包含负实体详细信息**

---

## 三、缓存层架构设计

### 3.1 优化后的数据流

```
┌─────────────────┐                      ┌──────────────────────────┐
│ 几何体生成      │ ──────────────────→  │ 本地缓存 (rkyv)          │
│ (prim/cata/loop)│   快速落盘 (<1ms)    │ cache/{dbnum}/batch_*.bin│
└─────────────────┘                      └──────────────────────────┘
                                                    │
                    ┌───────────────────────────────┼───────────────────────────────┐
                    ↓                               ↓                               ↓
         ┌──────────────────┐           ┌──────────────────┐           ┌──────────────────┐
         │ DB Consumer      │           │ mesh_worker      │           │ bool_worker      │
         │ 缓存 → SurrealDB │           │ 缓存 → mesh 文件 │           │ 缓存 → bool mesh │
         └──────────────────┘           └──────────────────┘           └──────────────────┘
                    ↓                               ↓                               ↓
         ┌──────────────────┐           ┌──────────────────┐           ┌──────────────────┐
         │ SurrealDB        │           │ assets/meshes/   │           │ assets/meshes/   │
         └──────────────────┘           └──────────────────┘           └──────────────────┘
```

### 3.2 缓存文件组织

```
output/
└── instance_cache/
    ├── index.json                    # 全局索引
    ├── dbnum_1001/
    │   ├── batch_001_1706234567.bin  # rkyv 序列化数据
    │   ├── batch_002_1706234568.bin
    │   └── status.json               # 处理状态
    ├── dbnum_1002/
    │   └── ...
    └── dbnum_1003/
        └── ...
```

### 3.3 缓存数据结构（rkyv 序列化）

```rust
use rkyv::{Archive, Deserialize, Serialize};

/// 可序列化的缓存批次数据
#[derive(Archive, Serialize, Deserialize)]
pub struct CachedInstanceBatch {
    /// 数据库编号
    pub dbnum: u32,
    /// 批次 ID
    pub batch_id: String,
    /// 创建时间戳
    pub created_at: i64,
    /// 实例信息 (inst_info 表)
    pub inst_info_map: HashMap<String, CachedEleGeosInfo>,
    /// 几何数据 (inst_geo + geo_relate 表)
    pub inst_geos_map: HashMap<String, CachedEleInstGeosData>,
    /// TUBI 信息
    pub inst_tubi_map: HashMap<String, CachedEleGeosInfo>,
    /// 负实体关系
    pub neg_relate_map: HashMap<String, Vec<String>>,
    /// NGMR 关系
    pub ngmr_neg_relate_map: HashMap<String, Vec<(String, String)>>,
    /// 负实体几何缓存（bool_worker 专用）
    pub neg_geo_cache: HashMap<String, CachedNegGeoInfo>,
}

/// 负实体几何信息（扩展字段，支持 bool_worker）
#[derive(Archive, Serialize, Deserialize)]
pub struct CachedNegGeoInfo {
    pub refno: String,
    pub geo_hash: String,
    pub geo_param: CachedGeoParam,
    pub transform: [f64; 16],
    pub aabb: Option<[f64; 6]>,
}
```

### 3.4 处理状态追踪

```rust
/// 批次处理状态
#[derive(Serialize, Deserialize)]
pub struct BatchStatus {
    pub batch_id: String,
    pub created_at: i64,
    /// 数据库写入状态
    pub db_synced: bool,
    pub db_synced_at: Option<i64>,
    /// mesh 生成状态
    pub mesh_processed: bool,
    pub mesh_processed_at: Option<i64>,
    pub mesh_count: usize,
    /// bool 运算状态
    pub bool_processed: bool,
    pub bool_processed_at: Option<i64>,
    pub bool_count: usize,
}
```

---

## 四、Worker 改造方案

### 4.1 mesh_worker 改造

**当前流程（扫库）：**
```rust
// 1. 查询数据库获取待处理 geo_hash
let pending = query_pending_mesh_geo_ids().await?;
// 2. 查询 geo_param
let geo_param = query_geo_param(geo_hash).await?;
// 3. 生成 mesh
generate_mesh(geo_param)?;
```

**改造后（读缓存）：**
```rust
// 1. 扫描缓存目录
for batch_file in scan_cache_dir(dbnum)? {
    // 2. 零拷贝读取（mmap）
    let batch = read_cache_zero_copy(&batch_file)?;

    // 3. 遍历 inst_geos_map 生成 mesh
    for (key, geos_data) in &batch.inst_geos_map {
        for geo in &geos_data.insts {
            if !is_mesh_exists(&geo.geo_hash) {
                generate_mesh(&geo.geo_param)?;
            }
        }
    }

    // 4. 更新状态
    update_batch_status(&batch_file, "mesh_processed", true)?;
}
```

### 4.2 bool_worker 改造

**当前流程（扫库）：**
```rust
// 1. 查询 neg_relate 关系
let neg_relates = query_neg_relates(refno).await?;
// 2. 查询负实体 geo_param 和 transform
let neg_geo = query_neg_geo_info(neg_refno).await?;
// 3. 执行布尔运算
boolean_subtract(pos_mesh, neg_mesh)?;
```

**改造后（读缓存）：**
```rust
// 1. 从缓存读取关系和负实体信息
let batch = read_cache_zero_copy(&batch_file)?;

// 2. 遍历 neg_relate_map
for (pos_refno, neg_refnos) in &batch.neg_relate_map {
    // 3. 从 neg_geo_cache 获取负实体信息（无需查库）
    let neg_geos: Vec<_> = neg_refnos.iter()
        .filter_map(|r| batch.neg_geo_cache.get(r))
        .collect();

    // 4. 执行布尔运算
    boolean_subtract_batch(pos_refno, &neg_geos)?;
}
```

---

## 五、foyer 混合缓存方案

### 5.1 为什么选择 foyer

[foyer](https://github.com/foyer-rs/foyer) 是一个高性能的 Rust 混合缓存库，灵感来自 Facebook/CacheLib。

| 特性 | foyer | 自研 rkyv 文件缓存 |
|------|-------|-------------------|
| 内存+磁盘混合 | ✅ 原生支持 | ❌ 需自行实现 |
| LRU/LFU 淘汰 | ✅ 多种算法 | ❌ 需自行实现 |
| 并发安全 | ✅ 无锁设计 | ⚠️ 需自行处理 |
| 异步支持 | ✅ tokio 原生 | ⚠️ 需自行封装 |
| 序列化 | ✅ serde/自定义 Code | rkyv |
| 维护成本 | ✅ 社区维护 | ❌ 自行维护 |

### 5.2 foyer 核心概念

```rust
// 1. 纯内存缓存
let cache: Cache<K, V> = CacheBuilder::new(capacity).build();

// 2. 混合缓存（内存 + 磁盘）
let hybrid: HybridCache<K, V> = HybridCacheBuilder::new()
    .memory(64 * 1024 * 1024)  // 64MB 内存
    .storage()
    .with_engine_config(BlockEngineConfig::new(device))
    .build()
    .await?;
```

### 5.3 缓存键值设计

```rust
/// 缓存键：dbnum + batch_id
#[derive(Clone, Hash, Eq, PartialEq)]
pub struct InstanceCacheKey {
    pub dbnum: u32,
    pub batch_id: String,
}

/// 缓存值：序列化的批次数据
#[derive(Clone)]
pub struct InstanceCacheValue {
    pub data: CachedInstanceBatch,
}
```

### 5.4 实现 Code trait（foyer 序列化要求）

```rust
use foyer::{Code, Result};

impl Code for InstanceCacheKey {
    fn encode(&self, writer: &mut impl std::io::Write) -> Result<()> {
        bincode::serialize_into(writer, self)
            .map_err(foyer::Error::bincode_error)
    }

    fn decode(reader: &mut impl std::io::Read) -> Result<Self> {
        bincode::deserialize_from(reader)
            .map_err(foyer::Error::bincode_error)
    }

    fn estimated_size(&self) -> usize {
        4 + self.batch_id.len()
    }
}

impl Code for InstanceCacheValue {
    fn encode(&self, writer: &mut impl std::io::Write) -> Result<()> {
        bincode::serialize_into(writer, &self.data)
            .map_err(foyer::Error::bincode_error)
    }

    fn decode(reader: &mut impl std::io::Read) -> Result<Self> {
        let data = bincode::deserialize_from(reader)
            .map_err(foyer::Error::bincode_error)?;
        Ok(Self { data })
    }

    fn estimated_size(&self) -> usize {
        // 估算序列化大小
        std::mem::size_of_val(&self.data)
    }
}
```

### 5.5 InstanceCacheManager 实现

```rust
use foyer::{HybridCache, HybridCacheBuilder, FsDeviceBuilder, BlockEngineConfig};

pub struct InstanceCacheManager {
    cache: HybridCache<InstanceCacheKey, InstanceCacheValue>,
}

impl InstanceCacheManager {
    pub async fn new(cache_dir: &Path) -> Result<Self> {
        let device = FsDeviceBuilder::new(cache_dir)
            .with_capacity(1024 * 1024 * 1024)  // 1GB 磁盘缓存
            .build()?;

        let cache = HybridCacheBuilder::new()
            .memory(128 * 1024 * 1024)  // 128MB 内存缓存
            .storage()
            .with_engine_config(BlockEngineConfig::new(device))
            .build()
            .await?;

        Ok(Self { cache })
    }

    /// 写入缓存
    pub fn insert(&self, dbnum: u32, batch_id: &str, data: CachedInstanceBatch) {
        let key = InstanceCacheKey { dbnum, batch_id: batch_id.to_string() };
        let value = InstanceCacheValue { data };
        self.cache.insert(key, value);
    }

    /// 读取缓存
    pub async fn get(&self, dbnum: u32, batch_id: &str) -> Option<CachedInstanceBatch> {
        let key = InstanceCacheKey { dbnum, batch_id: batch_id.to_string() };
        self.cache.get(&key).await.ok().flatten().map(|e| e.value().data.clone())
    }
}
```

### 5.6 与 Worker 集成

```rust
// mesh_worker 从 foyer 缓存读取
pub async fn run_mesh_worker_with_cache(
    cache: &InstanceCacheManager,
    dbnum: u32,
) -> Result<()> {
    for batch_id in cache.list_batches(dbnum).await? {
        if let Some(batch) = cache.get(dbnum, &batch_id).await {
            for (_, geos_data) in &batch.inst_geos_map {
                for geo in &geos_data.insts {
                    if !is_mesh_exists(&geo.geo_hash) {
                        generate_mesh(&geo.geo_param)?;
                    }
                }
            }
        }
    }
    Ok(())
}
```

### 5.7 foyer 优势总结

1. **热数据内存缓存**：频繁访问的数据自动保留在内存
2. **冷数据磁盘持久化**：不常用数据自动落盘，节省内存
3. **LRU 自动淘汰**：无需手动管理缓存生命周期
4. **异步 I/O**：不阻塞主线程
5. **断点恢复**：磁盘缓存在重启后仍可用

---

## 六、并行流水线设计

### 6.1 三阶段流水线

```
时间轴: T0 ──→ T1 ──→ T2 ──→ T3 ──→ T4 ──→ T5

阶段1 - 几何生成 + 缓存写入:
[dbnum=1001] → [dbnum=1002] → [dbnum=1003] → ...
     ↓              ↓              ↓
  [写缓存]       [写缓存]       [写缓存]

阶段2 - mesh_worker (并行消费):
          [读缓存 1001] → [生成mesh]
               [读缓存 1002] → [生成mesh]
                    [读缓存 1003] → [生成mesh]

阶段3 - bool_worker (依赖 mesh 完成):
                    [读缓存 1001] → [布尔运算]
                         [读缓存 1002] → [布尔运算]

阶段4 - DB Consumer (后台异步):
[缓存→SurrealDB] ────────────────────────────→
```

### 6.2 依赖关系

```
┌─────────────┐
│ 几何生成    │
└──────┬──────┘
       ↓
┌─────────────┐
│ 缓存写入    │ ← 无阻塞，立即返回
└──────┬──────┘
       │
       ├──────────────┬──────────────┐
       ↓              ↓              ↓
┌─────────────┐ ┌─────────────┐ ┌─────────────┐
│ mesh_worker │ │ DB Consumer │ │ Parquet导出 │
└──────┬──────┘ └─────────────┘ └─────────────┘
       ↓
┌─────────────┐
│ bool_worker │ ← 依赖 mesh 文件存在
└─────────────┘
```

---

## 七、实现步骤

### 7.1 第一阶段：foyer 集成

1. 添加 foyer 依赖（本地路径）
2. 定义 `CachedInstanceBatch` 结构并实现 `Code` trait
3. 实现 `InstanceCacheManager` 封装 foyer HybridCache

### 7.2 第二阶段：缓存写入层

1. 在 `orchestrator.rs` 中集成 `InstanceCacheManager`
2. 替换 `save_instance_data_optimize()` 为缓存写入
3. 实现 `ShapeInstancesData` → `CachedInstanceBatch` 转换

### 7.3 第三阶段：mesh_worker 改造

1. 改造 `run_mesh_worker()` 从 foyer 缓存读取
2. 实现批次遍历逻辑
3. 添加处理状态追踪

### 7.4 第四阶段：bool_worker 改造

1. 扩展缓存结构，添加 `neg_geo_cache`
2. 改造 `run_boolean_worker()` 从缓存读取
3. 处理 mesh 依赖等待逻辑

### 7.5 第五阶段：DB Consumer

1. 实现后台异步消费者（从 foyer 读取写入 SurrealDB）
2. foyer 自动管理缓存淘汰
3. 断点恢复机制

---

## 八、性能预估

### 8.1 单批次数据量假设

- 每批次约 1000 个实例
- 数据大小约 5MB

### 8.2 性能对比

| 操作 | 当前方案 | 缓存方案 |
|------|----------|----------|
| 几何生成后写入 | ~50ms (SurrealDB) | **<5ms** (本地文件) |
| mesh_worker 读取 | ~20ms (扫库) | **<1ms** (零拷贝) |
| bool_worker 读取 | ~30ms (多次查库) | **<2ms** (零拷贝) |

### 8.3 整体吞吐量提升

```
当前方案（串行）:
总耗时 = 几何生成 + DB写入 + mesh生成 + bool运算
       = 100% + 100% + 100% + 100% = 400% 基准时间

缓存方案（流水线）:
总耗时 ≈ max(几何生成, mesh生成, bool运算) + 少量开销
       ≈ 100% + 10% = 110% 基准时间

预计提升: ~3.6x
```

---

## 九、风险与缓解

| 风险 | 缓解措施 |
|------|----------|
| 缓存文件损坏 | rkyv 校验 + 重新生成 |
| 磁盘空间不足 | 定期清理已处理批次 |
| 数据一致性 | 状态文件追踪 + 幂等操作 |
| 断电数据丢失 | 批次粒度恢复 |

---

## 十、总结

### 10.1 核心改进

1. **解耦**：几何生成与数据库写入完全解耦
2. **并行**：mesh_worker/bool_worker 可与 DB 写入并行
3. **零拷贝**：rkyv 实现高效数据读取

### 10.2 关键结论

| Worker | 能否完全基于缓存 | 说明 |
|--------|------------------|------|
| mesh_worker | ✅ 完全可以 | 所需数据 100% 在缓存中 |
| bool_worker | ✅ 可以（需扩展） | 需添加 neg_geo_cache |
| DB Consumer | N/A | 后台异步写入 |

### 10.3 预期收益

- 模型生成吞吐量提升 **3-4 倍**
- 数据库写入压力降低 **90%**
- 支持断点恢复，提高可靠性

