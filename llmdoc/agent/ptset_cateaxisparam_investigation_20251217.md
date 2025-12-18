# ptset 和 CateAxisParam 数据结构调查报告

**调查时间**: 2025-12-17
**项目**: gen-model-fork
**调查目标**: 了解 ptset（点集/连接点）的数据结构、存储方式和查询 API

---

## 代码部分 (The Evidence)

### CateAxisParam 和 PlantAxisMap 定义

- `src/data_interface/structs.rs` (type alias PlantAxisMap):
  - 定义: `pub type PlantAxisMap = BTreeMap<i32, CateAxisParam>;`
  - CateAxisParam 来自 `aios_core::parsed_data` 包（外部依赖）
  - PlantAxisMap 是 i32 索引到 CateAxisParam 的有序映射

### ptset_map 在 inst_info 表中的存储

- `src/fast_model/pdms_inst.rs` (save_instance_data_optimize 函数，第338行):
  - 关键注释: "使用压缩格式存储 ptset（减少约 70-80% 存储空间）"
  - 调用: `info.gen_sur_json_compact(false)` - 将 ptset 以紧凑 JSON 格式存储到数据库
  - inst_info 表通过 `INSERT IGNORE INTO` 语句保存数据

### ptset_map 的使用模式

- `src/fast_model/cata_model.rs` (第706-748行):
  - ptset_map 从 design_axis_map 中提取并与 EleGeosInfo 关联
  - 通过 ARRI/LEAV 属性访问特定连接点: `cur_ptset_map.values().find(|x| x.number == arrive)`
  - 关键属性:
    - `number`: CateAxisParam 中的点编号标识
    - `pt`: 点的坐标（Vec3）
    - `pbore`: 点的管道外径

- `src/fast_model/cata_model.rs` (第914-944行):
  - ptset_map 可以继承自上一个处理的元素或从 target_cata.ptset 获取
  - 或逻辑: `ptset_map.as_ref().or(target_cata.ptset.as_ref()).cloned().unwrap_or_default()`

### ptset_map 在 TubiInfoData 中的应用

- `src/fast_model/cata_model.rs` (第736-747行):
  - 通过 arrive/leave 点从 ptset_map 中查找对应的 CateAxisParam
  - 创建 TubiInfoData: `TubiInfoData::from_axis_params(&cata_hash, a, l)`
  - 用于管道信息的存储（tubi_info_map）

### 点集查询 API (ArangoDB/SurrealDB)

- `src/plug_in/radiating.rs` (query_bran_point_map 函数，第254-275行):
  - **AQL 查询示例**:
    ```
    for v in 1 inbound @id @@pdms_edges
        filter v.noun != 'ATTA'
        let cata_hash = document(@@pdms_inst_infos,v._key)
        let hash = cata_hash.cata_hash == null ? cata_hash._key : cata_hash.cata_hash
        let geo = document(@@pdms_inst_geos,hash)
        return {
            'refno': v._key,
            'att_type': v.noun,
            'ptset_map': geo.ptset_map
        }
    ```
  - 该查询通过 inst_geos 表获取 ptset_map（不是直接从 inst_info）
  - 关键映射: cata_hash → inst_geos → ptset_map

- `src/plug_in/radiating.rs` (第171-231行 - 使用示例):
  - 访问模式: `point.ptset_map.get(arrive)` / `point.ptset_map.get(leave)`
  - 返回值: Option<CateAxisParam>
  - CateAxisParam 的字段:
    - `pt`: Vec3 - 点的三维坐标
    - `pbore`: f32 - 点的管道外径
    - `number`: i32 - 点编号（用于 ARRI/LEAV 属性匹配）

### 轴参数（Axis Parameter）查询 API

- `src/fast_model/resolve.rs` (resolve_axis_params 函数，第104-120行):
  - **函数签名**: `async fn resolve_axis_params(refno: RefnoEnum, context: Option<CataContext>) -> anyhow::Result<BTreeMap<i32, CateAxisParam>>`
  - 返回值: BTreeMap<i32, CateAxisParam>（与 PlantAxisMap 类型相同）
  - 实现流程:
    1. 获取 SCOM 引用: `aios_core::get_cat_refno(refno)`
    2. 获取 SCOM 信息: `get_or_create_scom_info(scom_refno)`
    3. 迭代 axis_params 并解析: `resolve_axis_param(&scom.axis_params[i], &scom, &context)`

- `src/fast_model/resolve.rs` (get_or_create_scom_info 函数，第24-101行):
  - 查询 PTRE 或 PSTR 引用: `attr_map.get_foreign_refno(ptref_name)`
  - 调用 aios_core 的轴参数查询: `query_axis_params(ptre_refno).await`
  - 返回值包含:
    - `axis_params: Vec<CateAxisParam>` - 轴参数数组
    - `axis_param_numbers: Vec<i32>` - 对应的编号数组

### EleGeosInfo 数据结构中的 ptset_map

- `src/fast_model/cata_model.rs` (第717-731行):
  - EleGeosInfo 是 inst_info 表的内存表示
  - 包含字段: `ptset_map: PlantAxisMap`（即 BTreeMap<i32, CateAxisParam>）
  - 在保存前由 `gen_sur_json_compact()` 序列化

### inst_info 表的主要字段

- `src/fast_model/pdms_inst.rs` (第328-377行 - inst_info 和 inst_relate 的插入):
  - inst_info 表字段包括:
    - `refno` (PE key) - 元件参考号
    - `cata_hash` - 元件库哈希（用于查找 inst_geos）
    - `world_transform` - 世界坐标系变换
    - `generic_type` - 通用类型
    - `has_cata_neg` - 是否有负实体
    - `is_solid` - 是否为实体
    - `owner_refno` / `owner_type` - 所有者信息
    - `ptset` - 压缩存储的点集（通过 gen_sur_json_compact）

---

## 报告 (The Answers)

### 1. CateAxisParam 的完整定义

**CateAxisParam** 是 aios_core 包中定义的结构体，在本项目中通过以下方式使用:

- **定义位置**: `aios_core::parsed_data::CateAxisParam`（外部依赖，需查看 rs-core 仓库）
- **主要属性** (基于使用模式推断):
  - `number: i32` - 点的编号/索引（用于 ARRI/LEAV 属性的匹配）
  - `pt: Vec3` - 点的三维坐标（世界空间）
  - `pbore: f32` - 点的管道外径（用于管径计算）
  - 其他属性可能包括方向向量等

- **在本项目中的别名**: `PlantAxisMap = BTreeMap<i32, CateAxisParam>`
  - 按编号（i32）索引，有序存储多个点

### 2. inst_info 表中 ptset 字段的存储方式

**存储位置和方式**:

- **主存储表**: inst_info（SurrealDB）
- **存储方式**: 通过 `gen_sur_json_compact(false)` 方法进行紧凑序列化
- **优化**: 压缩格式可减少约 70-80% 的存储空间
- **查询关系**: inst_info.cata_hash → inst_geos.ptset_map

**存储 SQL 示例**:
```sql
INSERT IGNORE INTO inst_info [
  { id: refno, ..., ptset: {...compressed format...} }
];
```

**字段关联**:
- inst_info 表保存了主体信息（包括 cata_hash）
- 实际的 ptset_map 通过 cata_hash 关联到 inst_geos 表的几何数据

### 3. 现有的查询 ptset 的 API

**SurrealDB/ArangoDB 查询示例** (在 radiating.rs 中):

```sql
for v in 1 inbound @id @@pdms_edges
    filter v.noun != 'ATTA'
    let cata_hash = document(@@pdms_inst_infos, v._key)
    let hash = cata_hash.cata_hash == null ? cata_hash._key : cata_hash.cata_hash
    let geo = document(@@pdms_inst_geos, hash)
    return {
        'refno': v._key,
        'att_type': v.noun,
        'ptset_map': geo.ptset_map
    }
```

**Rust API 函数**:

1. **轴参数解析** (对应 ptset 的来源):
   ```rust
   pub async fn resolve_axis_params(
       refno: RefnoEnum,
       context: Option<CataContext>
   ) -> anyhow::Result<BTreeMap<i32, CateAxisParam>>
   ```

2. **SCOM 信息获取**:
   ```rust
   pub async fn get_or_create_scom_info(
       cata_refno: RefnoEnum
   ) -> anyhow::Result<ScomInfo>
   // 包含: axis_params: Vec<CateAxisParam>
   ```

3. **点集查询** (AQL API):
   - 函数: `query_bran_point_map(bran_refno, database)`
   - 返回: `Vec<InstPointMap>`
   - InstPointMap 中包含 `ptset_map: BTreeMap<i32, CateAxisParam>`

### 4. ptset 包含的信息

**从 CateAxisParam 推断的信息**:

| 字段 | 类型 | 说明 | 用途 |
|------|------|------|------|
| number | i32 | 点编号 | 与 ARRI/LEAV 属性匹配，标识进出点 |
| pt | Vec3 | 三维坐标 | 几何计算、距离计算、可视化 |
| pbore | f32 | 管道外径 | 管道长度计算、截面面积计算 |
| 方向向量 | Vec3 (推测) | 连接方向 | 可能用于弯管等方向性元件 |

**使用场景**:

1. **管道元件**（ELBO/BEND/VALV/REDU）:
   - arrive 点: 连接进入点
   - leave 点: 连接离开点
   - 计算: `distance(arrive_pt, leave_pt)` - 元件长度

2. **三通元件**（TEE）:
   - 包含 3 个点（编号 1, 2, 3）
   - 计算: `pt1.distance(0) + pt2.distance(0) + pt3.distance(0)` - 等效长度

3. **管径变径**（REDU）:
   - arrive.pbore - 进入管径
   - leave.pbore - 离开管径
   - 用于自动管径转换

---

## 结论 (Conclusions)

1. **CateAxisParam** 是核心的几何点定义，来自 aios_core，包含点号、坐标、外径等信息

2. **ptset_map** 以压缩 JSON 格式存储在 inst_info 表中，通过 cata_hash 关联到 inst_geos 表的几何数据

3. **现有 API 包括**:
   - Rust 异步函数: `resolve_axis_params()`, `get_or_create_scom_info()`, `query_bran_point_map()`
   - AQL 查询: 通过 inst_infos → cata_hash → inst_geos → ptset_map 的链式查询
   - 没有现成的独立 REST API 端点获取 ptset

4. **ptset 的关键信息**:
   - **number**: i32 类型的点编号（1, 2, 3...），用于 ARRI/LEAV 属性匹配
   - **pt**: Vec3 三维坐标，用于几何计算
   - **pbore**: f32 管道外径，用于截面积分和管径计算
   - 可能还有方向/法向量用于特殊几何体

---

## 关键关系 (Relations)

- `src/fast_model/cata_model.rs` → 构建 EleGeosInfo 并设置 ptset_map
- `src/fast_model/pdms_inst.rs` → 序列化并保存到 inst_info 表
- `src/fast_model/resolve.rs` → 通过 SCOM 解析轴参数（ptset_map 的来源）
- `src/plug_in/radiating.rs` → 查询和使用 ptset_map 进行散热计算
- `aios_core` 包 → 提供 CateAxisParam 类型定义和查询函数

---

*调查完成时间: 2025-12-17*
