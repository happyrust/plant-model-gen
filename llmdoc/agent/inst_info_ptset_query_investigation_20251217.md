# SurrealDB inst_info 表 ptset 数据查询调查报告

**调查时间**: 2025-12-17
**项目**: gen-model-fork
**调查目标**: 如何从 SurrealDB 的 inst_info 表查询 ptset 数据，并获取批量元件的 ptset 信息

---

## 代码部分 (The Evidence)

### 1. SurrealDB 表常量定义

- `src/consts.rs` (AQL 集合常量，第 43-84 行):
  - `AQL_PDMS_INST_INFO_COLLECTION` = "pdms_inst_infos" - inst_info 表对应的 AQL 集合名称
  - `AQL_PDMS_INST_GEO_COLLECTION` = "pdms_inst_geos" - inst_geos 表对应的 AQL 集合名称
  - `AQL_PDMS_EDGES_COLLECTION` = "pdms_edges" - 元素层级关系边表
  - `AQL_PDMS_ELES_COLLECTION` = "pdms_eles" - 元素基本数据集合

### 2. inst_info 表的数据保存方式

- `src/fast_model/pdms_inst.rs` (save_instance_data_optimize 函数，第 328-348 行):
  - **保存方法**: 通过 `info.gen_sur_json_compact(false)` 序列化
  - **压缩优化**: 压缩格式可减少约 70-80% 的存储空间
  - **插入语句**: `INSERT IGNORE INTO inst_info [{}]` - 一次性插入多个记录
  - **关键字段**: ptset、refno、cata_hash、world_transform、generic_type、has_cata_neg、is_solid、owner_refno、owner_type
  - EleGeosInfo 对象通过 gen_sur_json_compact 方法生成 SurrealQL JSON 格式的插入语句

### 3. EleGeosInfo 数据结构

- `src/fast_model/cata_model.rs` (第 717-748 行):
  - **数据结构**: EleGeosInfo 包含字段:
    - `refno`: RefnoEnum - 元件参考号（作为主键）
    - `cata_hash`: Option<String> - 元件库哈希（用于查找 inst_geos）
    - `ptset_map`: PlantAxisMap - 点集（BTreeMap<i32, CateAxisParam>）
    - `world_transform`: Transform - 世界坐标变换
    - `generic_type`: GenericType - 通用类型
    - `has_cata_neg`: bool - 是否有负实体
    - `is_solid`: bool - 是否为实体
    - `owner_refno`: RefnoEnum - 所有者参考号
    - `owner_type`: String - 所有者类型
  - **ptset_map 类型**: `PlantAxisMap = BTreeMap<i32, CateAxisParam>`

### 4. 点集数据模型

- `src/data_interface/structs.rs` (第 1-10 行):
  - **PlantAxisMap 定义**: `pub type PlantAxisMap = BTreeMap<i32, CateAxisParam>;`
  - **CateAxisParam 来源**: `aios_core::parsed_data::CateAxisParam` (外部依赖)
  - **数据结构** (基于使用模式):
    - `number: i32` - 点的编号/索引
    - `pt: Vec3` - 点的三维坐标
    - `pbore: f32` - 点的管道外径

### 5. 现有的 AQL/SurrealQL 查询实现

- `src/plug_in/radiating.rs` (query_bran_point_map 函数，第 254-275 行):
  - **查询场景**: 查询 BRAN 下面所有元件的点集（除去 ATTA）
  - **AQL 查询语句**:
    ```
    with @@pdms_eles,@@pdms_edges,@@pdms_inst_infos,@@pdms_inst_geos
    for v in 1 inbound @id @@pdms_edges
        filter v.noun != 'ATTA'
        let cata_hash = document(@@pdms_inst_infos,v._key)
        let hash = cata_hash.cata_hash == null ? cata_hash._key : cata_hash.cata_hash
        let geo = document(@@pdms_inst_geos,hash)
        filter geo != null
        return {
            'refno': v._key,
            'att_type': v.noun,
            'ptset_map': geo.ptset_map
        }
    ```
  - **关键关系**: inst_infos → cata_hash → inst_geos → ptset_map
  - **数据流**: 通过 cata_hash 从 inst_infos 表链接到 inst_geos 表获取 ptset_map
  - **返回类型**: `Vec<InstPointMap>` - 包含 refno、att_type、ptset_map

### 6. 轴参数解析 API（ptset 的来源）

- `src/fast_model/resolve.rs` (resolve_axis_params 函数，第 104-120 行):
  - **函数签名**: `pub async fn resolve_axis_params(refno: RefnoEnum, context: Option<CataContext>) -> anyhow::Result<BTreeMap<i32, CateAxisParam>>`
  - **用途**: 解析设计元件的轴参数，这是 ptset_map 的直接来源
  - **实现流程**:
    1. 获取 SCOM 引用: `aios_core::get_cat_refno(refno)`
    2. 获取 SCOM 信息: `get_or_create_scom_info(scom_refno)`
    3. 遍历并解析: `resolve_axis_param(&scom.axis_params[i], &scom, &context)`
  - **返回值**: BTreeMap<i32, CateAxisParam> （与 PlantAxisMap 相同）

- `src/fast_model/resolve.rs` (get_or_create_scom_info 函数，第 24-101 行):
  - **获取 PTRE/PSTR 引用**: `attr_map.get_foreign_refno(ptref_name)`
  - **查询轴参数**: 调用 aios_core 的 `query_axis_params(ptre_refno).await`
  - **返回值**: ScomInfo 结构包含:
    - `axis_params: Vec<CateAxisParam>` - 轴参数数组
    - `axis_param_numbers: Vec<i32>` - 对应的编号数组

### 7. ptset 在模型生成中的使用

- `src/fast_model/cata_model.rs` (第 706-748 行):
  - **ptset_map 提取**: 从 design_axis_map 中获取 cur_ptset_map
  - **连接点查询**: `cur_ptset_map.values().find(|x| x.number == arrive)`
  - **ARRI/LEAV 属性**: 用 arrive 和 leave 点编号查找对应的 CateAxisParam
  - **Tubi 信息生成**: `TubiInfoData::from_axis_params(&cata_hash, a, l)` - 创建管道信息

---

## 报告 (The Answers)

### 1. inst_info 表的 Schema 定义

**表名**: inst_info (SurrealQL: "pdms_inst_infos")

**主要字段**:
| 字段名 | 类型 | 说明 |
|--------|------|------|
| id | Record (PE key) | 主键，元件参考号 (refno) |
| refno | number | 元件参考号（冗余存储） |
| cata_hash | string | 元件库哈希，用于链接 inst_geos 表 |
| ptset | object (JSON) | 压缩的点集数据，包含所有轴参数 |
| world_transform | object | 世界坐标系变换（位置、旋转、缩放） |
| generic_type | string | 通用类型（CATE/PRIM/LOOP） |
| has_cata_neg | bool | 是否有负实体 |
| is_solid | bool | 是否为实体 |
| owner_refno | number | 所有者参考号（如 BRAN/HANG/EQUI） |
| owner_type | string | 所有者类型 |

**注意**:
- ptset 字段以压缩 JSON 格式存储（通过 `gen_sur_json_compact()` 方法）
- 实际的完整 ptset_map 存储在 inst_geos 表中，inst_info 中的 ptset 是压缩版本
- 两个表通过 cata_hash 关联：inst_info.cata_hash → inst_geos.id

### 2. 查询单个元件的 ptset 的 3 种方式

#### 方案 A：从 inst_geos 表查询（推荐）

**SurrealQL 查询**:
```sql
SELECT ptset_map FROM inst_geos WHERE id = $cata_hash;
```

**Rust 代码示例**:
```rust
let cata_hash = "some_hash_value";
let sql = format!("SELECT ptset_map FROM inst_geos WHERE id = '{}'", cata_hash);
let result: Vec<BTreeMap<i32, CateAxisParam>> = SUL_DB.query_take(&sql, 0).await?;
```

**优点**: 直接获取完整的 ptset_map，性能最好

#### 方案 B：从 inst_info 表查询

**SurrealQL 查询**:
```sql
SELECT ptset FROM inst_info WHERE id = $refno;
```

**优点**: 可直接通过 refno 查询，无需知道 cata_hash

#### 方案 C：通过 Rust API 解析

**Rust 函数**:
```rust
pub async fn resolve_axis_params(
    refno: RefnoEnum,
    context: Option<CataContext>,
) -> anyhow::Result<BTreeMap<i32, CateAxisParam>>
```

**使用场景**: 需要计算和解析轴参数时（考虑坐标变换等）

### 3. 给定多个 refno，查询所有元件的 ptset 的方案

#### 方案 A：批量查询（推荐用于大数据量）

**SurrealQL 查询**:
```sql
-- 查询模型的所有元件的 ptset_map
LET $refnos = [
    PE:1000001,
    PE:1000002,
    PE:1000003
];

FOR $refno IN $refnos
    LET $inst_info = (SELECT cata_hash FROM inst_info WHERE id = $refno)[0]
    LET $geo = (SELECT ptset_map FROM inst_geos WHERE id = $inst_info.cata_hash)[0]
    RETURN {
        refno: $refno,
        cata_hash: $inst_info.cata_hash,
        ptset_map: $geo.ptset_map
    };
```

#### 方案 B：一条 SELECT IN 语句

**SurrealQL 查询**:
```sql
SELECT id as refno, cata_hash FROM inst_info
WHERE id IN [PE:1000001, PE:1000002, PE:1000003];

-- 然后批量查询 inst_geos
SELECT id, ptset_map FROM inst_geos
WHERE id IN [$hash1, $hash2, $hash3];
```

#### 方案 C：Rust 异步并发查询

**Rust 代码模式**:
```rust
// 1. 获取所有 refno 对应的 cata_hash
let refnos = vec![RefnoEnum::from(...), ...];
let inst_infos: Vec<InstInfo> = query_batch_inst_infos(&refnos).await?;

// 2. 并发查询所有 inst_geos
let cata_hashes: Vec<String> = inst_infos.iter()
    .map(|x| x.cata_hash.clone().unwrap_or_else(|| x.refno.to_string()))
    .collect();

let results = futures::future::join_all(
    cata_hashes.iter().map(|hash| {
        let sql = format!("SELECT ptset_map FROM inst_geos WHERE id = '{}'", hash);
        SUL_DB.query_take::<BTreeMap<i32, CateAxisParam>>(&sql, 0)
    })
).await;
```

### 4. 现有的查询 ptset 的 Rust 函数

#### 已有函数

1. **resolve_axis_params** (`src/fast_model/resolve.rs`, 第 104-120 行)
   - 用途: 解析单个元件的轴参数
   - 签名: `async fn resolve_axis_params(refno: RefnoEnum, context: Option<CataContext>) -> anyhow::Result<BTreeMap<i32, CateAxisParam>>`
   - 返回值: BTreeMap<i32, CateAxisParam> （即 ptset_map）

2. **query_bran_point_map** (`src/plug_in/radiating.rs`, 第 254-275 行)
   - 用途: 查询 BRAN 下所有元件的 ptset_map
   - 签名: `async fn query_bran_point_map(bran_refno: RefU64, database: &ArDatabase) -> anyhow::Result<Vec<InstPointMap>>`
   - 返回值: Vec<InstPointMap>，每个元素包含 refno、att_type、ptset_map

3. **get_or_create_scom_info** (`src/fast_model/resolve.rs`, 第 24-101 行)
   - 用途: 获取元件库的 SCOM 信息，包含轴参数
   - 返回值: ScomInfo，包含 `axis_params: Vec<CateAxisParam>`

#### 尚未有的函数（可能需要自己实现）

- **批量查询多个 refno 的 ptset**（不分层级）
- **指定模型的所有元件 ptset 查询**
- **按元件类型过滤的 ptset 批量查询**

### 5. ptset 包含的具体信息

**CateAxisParam 字段** (来自 aios_core::parsed_data):
| 字段 | 类型 | 用途 |
|------|------|------|
| number | i32 | 点编号，用于 ARRI/LEAV 属性匹配 |
| pt | Vec3 | 点的三维坐标（世界空间） |
| pbore | f32 | 点的管道外径 |
| 其他字段 | - | 可能包含法向量、方向等（需查看 aios_core 源码） |

**ptset_map 结构**:
- 键: i32 - 点的编号
- 值: CateAxisParam - 点的完整信息
- 有序: BTreeMap 保证按编号排序

---

## 结论 (Conclusions)

1. **ptset 存储位置**:
   - inst_info 表存储的是压缩版本的 ptset 字段
   - 完整的 ptset_map 实际上存储在 inst_geos 表中
   - 两表通过 cata_hash 关联

2. **查询推荐方案**:
   - 单个元件: 使用 `SELECT ptset_map FROM inst_geos WHERE id = $cata_hash`
   - 多个元件: 先 SELECT IN inst_info 获取 cata_hash，再批量查询 inst_geos
   - 需要计算的轴参数: 使用 `resolve_axis_params()` Rust API

3. **现有 API 覆盖**:
   - ✅ 单元件轴参数解析: `resolve_axis_params()`
   - ✅ 层级内查询: `query_bran_point_map()` （BRAN 内的元件）
   - ❌ 模型级别的批量查询：尚无现成函数

4. **关键字段映射**:
   - refno → inst_info.id (主键)
   - cata_hash → inst_info.cata_hash (关联键)
   - ptset_map → inst_geos.ptset_map (完整数据)

5. **如何为模型获取所有元件的 ptset**:
   ```
   1. 获取模型的所有 refno
   2. SELECT cata_hash FROM inst_info WHERE id IN (所有 refno)
   3. SELECT ptset_map FROM inst_geos WHERE id IN (所有 cata_hash)
   4. 按 refno 对应上结果
   ```

---

## 关键关系 (Relations)

- `src/fast_model/pdms_inst.rs` → 保存 EleGeosInfo 到 inst_info 表（通过 gen_sur_json_compact）
- `src/fast_model/cata_model.rs` → 构建 EleGeosInfo 并设置 ptset_map
- `src/fast_model/resolve.rs` → 通过 SCOM 解析轴参数（ptset_map 的计算来源）
- `src/plug_in/radiating.rs` → 示例：查询和使用 ptset_map 进行散热计算
- `src/data_interface/structs.rs` → 定义 PlantAxisMap 类型别名
- `src/consts.rs` → 定义 AQL 集合名称常量
- `aios_core::parsed_data` → 提供 CateAxisParam 和 ScomInfo 类型

---

## 快速参考查询语句

### 查询单个元件的 ptset_map
```sql
-- 方式 1: 通过 inst_geos 表（推荐）
SELECT ptset_map FROM inst_geos WHERE id = 'some_hash';

-- 方式 2: 通过 inst_info 表的 ptset 字段
SELECT ptset FROM inst_info WHERE id = PE:1000001;
```

### 查询多个元件的 ptset_map
```sql
-- 两步查询方案
LET $refnos = [PE:1000001, PE:1000002];
LET $inst_infos = SELECT id, cata_hash FROM inst_info
                   WHERE id IN $refnos;
LET $hashes = $inst_infos[].cata_hash;

SELECT id, ptset_map FROM inst_geos WHERE id IN $hashes;
```

### 查询 BRAN 下所有元件的 ptset（参考 radiating.rs）
```sql
with pdms_eles, pdms_edges, pdms_inst_infos, pdms_inst_geos
for v in 1 inbound 'pdms_eles/24383/66521' pdms_edges
    filter v.noun != 'ATTA'
    let inst = document(pdms_inst_infos, v._key)
    let hash = inst.cata_hash ?? v._key
    let geo = document(pdms_inst_geos, hash)
    filter geo != null
    return {
        refno: v._key,
        att_type: v.noun,
        ptset_map: geo.ptset_map
    };
```

---

*调查完成时间: 2025-12-17*
