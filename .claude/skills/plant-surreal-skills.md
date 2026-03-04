# Plant SurrealDB Skills

这是针对 rs-core (aios_core) 项目的 SurrealDB 数据库查询和架构知识库。

---

## 核心原则

### 1. 查询优先级
- **层级查询**：优先使用 TreeIndex（`collect_*` 系列函数），性能提升 10-100 倍
- **属性关系**：使用 SurrealDB 图遍历（`->GMRE`, `->LSTU->CATR` 等）
- **批量查询**：使用数据库端函数（`fn::*`）和 `array::map` 模式，避免循环查询

### 2. 类型安全规范
- **必须使用** `SurrealValue` trait，禁止使用 `serde_json::Value`
- ID 字段使用 `RefnoEnum` 或 `RefU64`（已兼容自动转换）
- 时间戳使用 `surrealdb::types::Datetime`
- 使用 `#[serde(alias = "id")]` 处理字段别名

### 3. 性能优化原则
- 批量查询优于循环查询（性能提升 N 倍）
- 使用 ID Range 替代 WHERE 条件（索引查询）
- 限制递归深度（如 `Some("1..5")`），避免无限递归
- 使用 `array::distinct()` 去重（SurrealDB 不支持 `SELECT DISTINCT`）
- 直接访问字段，避免 `record::id()` 函数调用
- **避免过度防御**：TreeIndex 已保证类型，批量查询时无需再用 noun 过滤（主键查询最快）

---

## 数据库架构速查

### 核心表结构

```
pe (元素主表) - 统一存储所有工程元素
├─ pe_owner (层级关系) - child -> parent 父子关系
├─ inst_relate (几何实例) → inst_info → geo_relate → inst_geo
├─ inst_relate_aabb (包围盒关系)
├─ tubi_relate (管道直段) - 复合 ID: [bran_pe, index]
├─ neg_relate (负实体关系)
├─ ngmr_relate (NGMR 负实体)
└─ tag_name_mapping (位号映射)
```

### PE 表（核心存储表）
- **ID 格式**: `pe:⟨dbnum_refno⟩` 例如 `pe:⟨21491_10000⟩`
- **关键字段**:
  - `id`: RefnoEnum 类型
  - `noun`: 元素类型（'SITE', 'ZONE', 'EQUI', 'PIPE', 'BOX', 'CYLI' 等）
  - `name`: 元素名称
  - `owner`: 父节点引用（指向 pe 表）
  - `children`: 子节点集合（数组字段）
  - `deleted`: 逻辑删除标记
  - `sesno`: 会话编号
  - `dbnum`: 数据库编号

### 类型表与 PE 表的关系
- **WORL、SITE、ZONE、EQUI、PIPE** 等类型表存储特定类型的属性
- 类型表的 **REFNO 字段指向 pe 表**
- 查询示例：`SELECT value REFNO FROM WORL WHERE REFNO.dbnum = 1112`

### pe_owner 关系表（层级关系）
- **关系方向**: `child (in) -[pe_owner]-> parent (out)`
- **重要**: 针对 pe 表之间的连接，不是针对类型表
- **查询子节点**: `SELECT VALUE in FROM pe:⟨parent⟩<-pe_owner WHERE in.deleted = false`
- **查询父节点**: `SELECT VALUE out FROM pe:⟨child⟩->pe_owner`
- **⚠️ 推荐**: 层级查询使用 TreeIndex，性能提升 100 倍

### 模型生成关系链
```
pe (PE 元素)
  ↓ inst_relate
inst_info (实例信息)
  ↓ geo_relate
inst_geo (几何数据)
```

### geo_relate 表的 geo_type 字段
| geo_type | 含义 | 是否导出 |
|----------|------|----------|
| `Pos` | 原始几何（未布尔运算） | ✅ 导出 |
| `DesiPos` | 设计位置 | ✅ 导出 |
| `CatePos` | 布尔运算后的结果 | ✅ 导出 |
| `Compound` | 组合几何体（包含负实体引用） | ❌ 不导出 |
| `CateNeg` | 负实体 | ❌ 不导出 |
| `CataCrossNeg` | 交叉负实体 | ❌ 不导出 |

**导出条件**: `geo_type IN ['Pos', 'DesiPos', 'CatePos']`

---

## 数据库表详细结构

### 1. pe 表（PDMS 元素主表）

**作用**：统一存储所有类型的工厂工程元素

**ID 格式**：`pe:⟨dbnum_refno⟩` 例如 `pe:⟨21491_10000⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | RefnoEnum | 记录唯一标识符 |
| `refno` | RefU64 | 包含 dbnum、sesno、deleted 等字段 |
| `noun` | String | 元素类型（'SITE', 'ZONE', 'EQUI', 'PIPE', 'BOX', 'CYLI' 等） |
| `name` | String | 元素名称 |
| `owner` | RefnoEnum | 父节点引用（指向 pe 表中的记录） |
| `children` | Array<RefnoEnum> | 子节点集合（数组字段，存储子节点的 pe 记录引用） |
| `deleted` | bool | 逻辑删除标记 |
| `sesno` | i32 | 会话编号 |
| `dbnum` | i32 | 数据库编号 |
| `old_pe` | Option<RefnoEnum> | 历史引用 |
| `dt` | Option<Datetime> | 时间戳 |

**查询示例**：

```sql
-- 查询所有 EQUI 类型的元素
SELECT * FROM pe WHERE noun = 'EQUI' AND deleted = false;

-- 查询特定数据库的元素
SELECT * FROM pe WHERE dbnum = 1112 AND noun IN ['SITE', 'ZONE'];

-- 从记录数组查询
SELECT * FROM [pe:⟨123⟩, pe:⟨456⟩];

-- 查询单条记录
SELECT * FROM ONLY pe:⟨123⟩ LIMIT 1;
```

**Rust API**：

```rust
use aios_core::get_pe;

// 获取单个元素
if let Ok(Some(pe)) = get_pe(refno).await {
    println!("name: {}, noun: {}", pe.name, pe.noun);
}
```

---

### 2. inst_relate 表（实例关系表）

**作用**：连接 PE 元素和几何实例，是模型生成的核心关系表

**ID 格式**：`inst_relate:⟨refno⟩`

**关系方向**：`pe:⟨xxx⟩ -> inst_relate -> inst_info:⟨xxx⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | String | 关系记录 ID |
| `in` | RefnoEnum | PE 元素引用（源节点） |
| `out` | String | inst_info 引用（目标节点） |
| `owner` | RefnoEnum | 所属构件 |
| `generic` | String | 构件类型（'EQUI', 'PIPE' 等） |
| `aabb` | Option<AabbData> | 包围盒 |
| `world_trans` | Option<TransformData> | 世界变换矩阵 |
| `booled_id` | Option<String> | 布尔运算结果 ID |
| `dt` | Option<Datetime> | 时间戳 |
| `zone_refno` | Option<RefnoEnum> | 区域引用 |
| `old_pe` | Option<RefnoEnum> | 历史 PE |
| `ptset` | Option<PtsetData> | 点集数据 |

**查询示例**：

```sql
-- 查询构件的实例关系
SELECT * FROM pe:⟨{refno}⟩->inst_relate;

-- 获取实例信息（通过关系）
SELECT VALUE out FROM pe:⟨{refno}⟩->inst_relate;

-- 查询所有有实例关系的构件
SELECT VALUE in FROM inst_relate;

-- 查询特定类型的实例关系
SELECT * FROM inst_relate WHERE generic = 'EQUI';

-- 查询指定 ZONE 的所有实例关系
SELECT * FROM inst_relate WHERE zone_refno = pe:⟨{zone_refno}⟩;
```

**Rust API**：

```rust
use aios_core::{SUL_DB, SurrealQueryExt};

// 查询构件的实例信息
let sql = format!(
    "SELECT * FROM pe:{}->inst_relate",
    refno.to_pe_key()
);
let inst_relate: Vec<InstRelateQuery> = SUL_DB.query_take(&sql, 0).await?;

// 使用封装函数批量查询
use aios_core::rs_surreal::query_insts;
let geom_insts = query_insts(&refnos, enable_holes).await?;
```

---

### 3. inst_info 表（实例信息表）

**作用**：存储几何实例的元数据

**ID 格式**：`inst_info:⟨refno_info⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | String | 实例信息 ID |
| `generic_type` | String | 实例类型（'EQUI', 'PIPE' 等） |
| `owner_refno` | RefnoEnum | 所属 PE 元素 |

**查询示例**：

```sql
-- 查询单个实例信息
SELECT * FROM inst_info:⟨{refno}_info⟩;

-- 查询特定类型的实例
SELECT * FROM inst_info WHERE generic_type = 'EQUI';

-- 查询指定 ZONE 的实例
SELECT * FROM inst_info
WHERE owner_refno IN (
    SELECT VALUE id FROM fn::collect_descendant_ids_by_types(pe:⟨{zone_refno}⟩, ['EQUI'], none, "..")
);
```

---

### 4. inst_geo 表（几何数据表）

**作用**：存储几何体的参数和状态

**ID 格式**：`inst_geo:⟨geo_hash⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | String | 几何数据 ID（hash） |
| `param` | JSON | 几何参数（JSON 格式） |
| `meshed` | bool | 是否已网格化 |
| `visible` | bool | 是否可见 |
| `trans` | Option<TransformData> | 局部变换矩阵 |
| `geo_type` | String | 几何类型（'Pos', 'Neg', 'CataNeg' 等） |
| `refno` | Option<RefnoEnum> | 构件编号 |
| `bad` | bool | 是否为异常几何 |

**查询示例**：

```sql
-- 查询单个几何单元
SELECT * FROM inst_geo:⟨{geo_hash}⟩;

-- 查询已网格化的几何单元
SELECT * FROM inst_geo WHERE meshed = true;

-- 查询错误的几何单元
SELECT * FROM inst_geo WHERE bad = true;

-- 批量查询几何单元
SELECT * FROM [inst_geo:⟨hash1⟩, inst_geo:⟨hash2⟩];
```

---

### 5. geo_relate 表（几何关系表）

**作用**：连接实例信息和具体几何数据

**ID 格式**：`geo_relate:⟨hash⟩`

**关系方向**：`inst_info:⟨xxx⟩ -> geo_relate -> inst_geo:⟨xxx⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | String | 关系记录 ID |
| `in` | String | inst_info 引用（源节点） |
| `out` | String | inst_geo 引用（目标节点） |
| `trans` | Option<TransformData> | 变换矩阵 |
| `visible` | bool | 是否可见 |
| `meshed` | bool | 是否已网格化 |
| `geo_type` | String | 几何类型（'Pos', 'DesiPos', 'CatePos', 'Compound', 'CateNeg', 'CataCrossNeg'） |
| `geom_refno` | Option<String> | 几何参考号 |

**查询示例**：

```sql
-- 查询实例的所有几何关系
SELECT * FROM geo_relate WHERE in = inst_info:⟨{refno}_info⟩;

-- 获取实例的所有几何单元
SELECT VALUE out FROM geo_relate WHERE in = inst_info:⟨{refno}_info⟩;

-- 查询几何关系及其变换信息
SELECT
    gr.in as inst_info,
    gr.out as inst_geo,
    gr.trans as transform,
    gr.geom_refno,
    gr.geo_type,
    gr.visible
FROM geo_relate gr
WHERE gr.in = inst_info:⟨{refno}_info⟩;

-- 查询可见的几何关系（用于导出）
SELECT * FROM geo_relate
WHERE visible = true
  AND geo_type IN ['Pos', 'DesiPos', 'CatePos'];
```

---

### 6. tubi_relate 表（管道直段关系表）

**作用**：存储 BRAN/HANG 下的管道直段信息

**ID 格式**：`tubi_relate:[pe:⟨bran_refno⟩, index]` 例如 `tubi_relate:[pe:⟨21491_10000⟩, 0]`

**关系方向**：`pe:⟨leave⟩ -> tubi_relate:[pe:⟨bran⟩, idx] -> pe:⟨arrive⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id[0]` | pe record | BRAN/HANG 的 pe_key |
| `id[1]` | int | 管道段索引 |
| `in` | RefnoEnum | 起点构件（leave_refno） |
| `out` | RefnoEnum | 终点构件（arrive_refno） |
| `geo` | Option<String> | inst_geo 引用 |
| `aabb` | Option<AabbData> | 包围盒 `{ d: Aabb }` |
| `world_trans` | Option<TransformData> | 世界变换 `{ d: Transform }` |
| `bore_size` | String | 管径尺寸 |
| `bad` | bool | 是否为异常段 |
| `system` | Option<RefnoEnum> | 所属系统 |
| `dt` | Option<Datetime> | 时间戳 |

**查询示例（使用 ID Range）**：

```sql
-- 查询 BRAN 的所有管道直段（推荐）
SELECT * FROM tubi_relate:[pe:⟨bran_refno⟩, 0]..[pe:⟨bran_refno⟩, ..];

-- 字段提取（直接访问复合 ID）
SELECT
    id[0] as refno,              -- 取复合 ID 的第一个元素
    in as leave,                 -- 起点
    out as arrive,               -- 终点
    id[0].old_pe as old_refno,   -- 访问该 PE 的 old_pe 字段
    id[0].owner.noun as generic, -- 访问该 PE 的 owner 的 noun 字段
    aabb.d as world_aabb,        -- 取包围盒的数据部分
    world_trans.d as world_trans,-- 取变换矩阵的数据部分
    record::id(geo) as geo_hash  -- 提取 geo record 的 ID 字符串
FROM tubi_relate:[pe_key, 0]..[pe_key, ..]
WHERE aabb.d != NONE;
```

**Rust API**：

```rust
use aios_core::rs_surreal::query_tubi_insts_by_brans;

// 批量查询管道直段
let tubi_insts = query_tubi_insts_by_brans(&bran_refnos).await?;
```

---

### 7. neg_relate 表（负实体关系表）

**作用**：存储布尔运算中的负实体关系

**ID 格式**：`neg_relate:[neg_refno, index]`

**关系方向**：`geo_relate (in) -> neg_relate -> pe (out)` （负实体几何 -> 正实体）

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `in` | String | geo_relate 引用（负实体几何关系） |
| `out` | RefnoEnum | 正实体 PE（被减实体） |
| `pe` | RefnoEnum | 负载体 PE（负实体所属构件） |

**查询示例**：

```sql
-- 查询被负实体切割的关系
SELECT * FROM neg_relate WHERE out = pe:⟨{target_refno}⟩;

-- 获取负载体（负实体）
SELECT VALUE pe FROM pe:⟨{target_refno}⟩<-neg_relate;

-- 查询负实体几何关系
SELECT
    nr.out as target_refno,
    nr.pe as neg_carrier,
    nr.in as geo_relate_id
FROM neg_relate nr
WHERE nr.out = pe:⟨{target_refno}⟩;
```

---

### 8. ngmr_relate 表（NGMR 负实体关系表）

**作用**：存储 NGMR 类型的负实体关系

**ID 格式**：`ngmr_relate:[ele_pe, target_pe, ngmr_pe]`

**关系方向**：`geo_relate (in) -> ngmr_relate -> pe (out)`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `in` | String | geo_relate 引用（负实体几何关系） |
| `out` | RefnoEnum | 正实体 PE（目标） |
| `pe` | RefnoEnum | 负载体 PE（NGMR 负实体所属构件） |

**查询示例**：

```sql
-- 查询 NGMR 切割关系
SELECT * FROM ngmr_relate WHERE out = pe:⟨{target_refno}⟩;

-- 获取 NGMR 负载体
SELECT VALUE pe FROM pe:⟨{target_refno}⟩<-ngmr_relate;

-- 联合查询所有负实体关系
SELECT VALUE pe FROM pe:⟨{target_refno}⟩<-neg_relate
UNION
SELECT VALUE pe FROM pe:⟨{target_refno}⟩<-ngmr_relate;
```

**Rust API**：

```rust
// 使用数据库端函数查询所有负实体
let sql = format!(
    "SELECT VALUE fn::query_negative_entities({})",
    target_pe_key
);
let neg_entities: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await?;
```

---

### 9. 辅助表

#### trans 表（变换矩阵表）

**ID 格式**：`trans:⟨hash⟩`

**字段**：`{ id: String, d: Transform }`

**查询示例**：
```sql
SELECT * FROM trans:⟨{transform_hash}⟩;
```

#### aabb 表（包围盒表）

**ID 格式**：`aabb:⟨hash⟩`

**字段**：`{ id: String, d: Aabb }`

**查询示例**：
```sql
SELECT * FROM aabb:⟨{aabb_hash}⟩;
```

#### vec3 表（向量表）

**ID 格式**：`vec3:⟨hash⟩`

**字段**：`{ id: String, d: Vec3 }`

---

### 10. 业务扩展表

#### tag_name_mapping 表（位号映射表）

**作用**：存储 PE 元素到位号（Tag Name）的映射关系

**关系方向**：`pe (in) -> tag_name_mapping`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `in` | RefnoEnum | PE 元素引用 |
| `tag_name` | String | 位号名称 |
| `full_name` | String | 节点全名 |
| `source` | String | 数据来源（如 "excel", "manual"） |
| `created_at` | Datetime | 创建时间 |
| `updated_at` | Option<Datetime> | 更新时间 |

**Rust API**：

```rust
use aios_core::rs_surreal::tag_name_mapping::{
    get_tag_name_by_refno,
    get_tag_names_by_refnos,
    upsert_tag_name_mapping
};

// 查询单个位号
let tag_name = get_tag_name_by_refno(refno).await?;

// 批量查询位号
let tag_names = get_tag_names_by_refnos(&refnos).await?;

// 创建或更新映射
upsert_tag_name_mapping(refno, "P-1001", "SITE/ZONE/EQUI", "excel").await?;
```

---

#### measurement 表（测量数据表）

**作用**：存储三维测量数据

**ID 格式**：`measurement:⟨uuid⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | String | 测量记录 ID |
| `measurement_type` | String | 测量类型（distance, angle, area 等） |
| `project_id` | Option<String> | 项目 ID |
| `points` | Vec<Vec3> | 测量点坐标 |
| `value` | f64 | 测量值 |
| `unit` | String | 单位 |
| `created_by` | String | 创建者 |
| `created_at` | Datetime | 创建时间 |
| `status` | String | 状态 |

**Rust API**：

```rust
use aios_core::rs_surreal::measurement_query::{
    create_measurement,
    list_measurements,
    get_measurement_by_id
};

// 创建测量
create_measurement(measurement_type, points, value, unit, created_by).await?;

// 列出测量
let measurements = list_measurements(project_id).await?;

// 按 ID 查询
let measurement = get_measurement_by_id(id).await?;
```

---

#### annotation 表（批注数据表）

**作用**：存储三维批注数据

**ID 格式**：`annotation:⟨uuid⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | String | 批注记录 ID |
| `annotation_type` | String | 批注类型 |
| `project_id` | Option<String> | 项目 ID |
| `position` | Vec3 | 批注位置 |
| `content` | String | 批注内容 |
| `target_refno` | Option<RefnoEnum> | 关联的 PE 元素 |
| `created_by` | String | 创建者 |
| `assignee` | Option<String> | 指派人 |
| `status` | String | 状态 |
| `created_at` | Datetime | 创建时间 |

**Rust API**：

```rust
use aios_core::rs_surreal::annotation_query::{
    create_annotation,
    list_annotations,
    get_annotation_by_id
};

// 创建批注
create_annotation(annotation_type, position, content, target_refno, created_by).await?;

// 列出批注
let annotations = list_annotations(project_id).await?;
```

---

#### dbnum_info_table 表（数据库统计表）

**作用**：存储各数据库编号的统计信息，由事件自动维护

**ID 格式**：`dbnum_info_table:⟨dbnum⟩`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | u32 | 数据库编号 |
| `dbnum` | u32 | 数据库编号（冗余） |
| `count` | u64 | PE 记录数量 |
| `sesno` | i32 | 最大会话号 |
| `max_ref1` | i32 | 最大 ref1 值 |
| `updated_at` | Datetime | 更新时间 |

**自动更新**：通过 `DEFINE EVENT update_dbnum_event ON pe` 自动维护

**查询示例**：

```sql
-- 查询数据库统计信息
SELECT * FROM dbnum_info_table:⟨{dbnum}⟩;

-- 查询所有数据库统计
SELECT * FROM dbnum_info_table;
```

---

#### pbs 表（PBS 元素表）

**作用**：存储产品分解结构（Product Breakdown Structure）元素

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | RecordId | PBS 元素 ID |
| `name` | String | 名称 |
| `noun` | String | 类型 |
| `deleted` | bool | 是否删除 |
| `children_cnt` | Option<i32> | 子节点数量 |

**层级关系**：通过 `pbs_owner` 关系表维护

**Rust API**：

```rust
use aios_core::rs_surreal::pbs::{
    get_pbs_element,
    list_pbs_elements
};

// 查询 PBS 元素
let pbs = get_pbs_element(pbs_id).await?;

// 列出 PBS 元素
let pbs_list = list_pbs_elements().await?;
```

---

### 11. 布尔运算相关表

#### inst_relate_aabb 表（实例包围盒关系表）

**作用**：存储 PE 元素与包围盒的关联

**关系方向**：`pe (in) -> inst_relate_aabb -> aabb (out)`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `in` | RefnoEnum | PE 元素引用 |
| `out` | String | aabb record 引用 |

**查询示例**：

```sql
-- 查询构件的包围盒关系
SELECT * FROM pe:⟨{refno}⟩->inst_relate_aabb;

-- 获取构件的包围盒
SELECT VALUE out FROM pe:⟨{refno}⟩->inst_relate_aabb;
```

---

#### inst_relate_bool 表（布尔运算关系表）

**作用**：存储几何单元级别的布尔运算结果

**关系方向**：`inst_geo (in) -> inst_relate_bool -> inst_geo (out)`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `in` | String | 原始几何 inst_geo 引用 |
| `out` | String | 布尔运算后的 inst_geo 引用 |
| `status` | String | 布尔运算状态（'Success', 'Failed' 等） |

**查询示例**：

```sql
-- 查询布尔运算结果
SELECT * FROM inst_relate_bool WHERE in = inst_geo:⟨{geo_hash}⟩;

-- 获取布尔运算后的几何
SELECT VALUE out FROM inst_geo:⟨{geo_hash}⟩->inst_relate_bool;
```

---

#### inst_relate_cata_bool 表（CATE 布尔运算关系表）

**作用**：存储实例级别的布尔运算结果

**关系方向**：`inst_info (in) -> inst_relate_cata_bool -> inst_geo (out)`

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `in` | String | inst_info 引用 |
| `out` | String | 布尔运算后的 inst_geo 引用 |
| `status` | String | 布尔运算状态 |

**查询示例**：

```sql
-- 查询 CATE 布尔运算结果
SELECT * FROM inst_relate_cata_bool WHERE in = inst_info:⟨{refno}_info⟩;
```

---

### 12. 复合查询示例

#### 查询构件的完整几何实例信息

```sql
SELECT
    ir.in as refno,
    ir.generic,
    gr.out as geo_hash,
    gr.trans as local_transform,
    gr.geo_type,
    gr.visible,
    ig.meshed,
    ig.bad
FROM pe:⟨{refno}⟩->inst_relate AS ir
INNER JOIN ir.out->geo_relate AS gr
INNER JOIN gr.out AS ig
WHERE gr.visible = true;
```

---

#### 查询布尔运算成功的实例（用于导出）

```sql
SELECT
    gr.out as original_geo,
    COALESCE(br.out, gr.out) as final_geo,  -- 优先使用布尔结果
    br.status as bool_status
FROM inst_info:⟨{refno}_info⟩->geo_relate AS gr
LEFT JOIN gr.out->inst_relate_bool AS br
WHERE br.status = 'Success' OR br.status = NONE;
```

---

#### 批量查询多个构件的几何实例

```sql
LET $refnos = fn::collect_descendant_ids_by_types(pe:⟨{root_refno}⟩, ['EQUI', 'PIPE'], none, "..");
SELECT
    ir.in as refno,
    ir.generic,
    array::map(ir.out->geo_relate, |$gr| $gr.out) as geo_hashes
FROM inst_relate ir
WHERE ir.in IN $refnos;
```

---

## 常用查询模式

### 1. 层级查询（推荐 TreeIndex）

```rust
// 查询子节点（单层）
use aios_core::collect_children_filter_ids;
let children = collect_children_filter_ids(refno, &["EQUI", "PIPE"]).await?;

// 查询所有子节点（不限制类型）
let all_children = collect_children_filter_ids(refno, &[]).await?;

// 查询子孙节点（多层，不限深度）
use aios_core::collect_descendant_filter_ids;
let descendants = collect_descendant_filter_ids(&[refno], &["EQUI"], None).await?;

// 限制深度（1-5 层）
let shallow = collect_descendant_filter_ids(&[refno], &["EQUI"], Some("1..5")).await?;

// 批量查询多个根节点
let multi_descendants = collect_descendant_filter_ids(
    &[refno1, refno2, refno3],
    &["BOX", "CYLI"],
    None
).await?;

// 查询祖先节点
use aios_core::query_filter_ancestors;
let zones = query_filter_ancestors(refno, &["ZONE"]).await?;
let sites_or_zones = query_filter_ancestors(refno, &["SITE", "ZONE"]).await?;
```

### 2. 属性关系遍历（SurrealDB）

```rust
// 查询几何成员关系（GMRE）和几何结构（GSTR）
let result = aios_core::query_single_by_paths(
    cata_refno,
    &["->GMRE", "->GSTR"],  // 属性关系遍历
    &["REFNO"],             // 要获取的字段
).await?;

// 查询离开点（LSTU）的元件库（CATR）关系
let catr = aios_core::query_single_by_paths(
    leave_refno,
    &["->LSTU->CATR"],      // 属性关系链式遍历
    &["REFNO"],
).await?;
```

**适用场景**：
- `->GMRE`, `->GSTR`: 几何属性关系
- `->LSTU->CATR`: 管道连接关系
- 其他 PDMS 属性关系（SPRE, CATR, GMRE, GSTR 等）

**注意**：属性关系查询仍需要通过 SurrealDB，TreeIndex 只处理层级关系（父子）。

### 3. 几何实例查询

```rust
// 查询几何实例
use aios_core::rs_surreal::query_insts;
let geom_insts: Vec<GeomInstQuery> = query_insts(&refnos, enable_holes).await?;

// 查询管道直段（使用 ID Range）
use aios_core::rs_surreal::query_tubi_insts_by_brans;
let tubi_insts: Vec<TubiInstQuery> = query_tubi_insts_by_brans(&bran_refnos).await?;

// 按区域查询实例
use aios_core::rs_surreal::query_insts_by_zone;
let geom_insts = query_insts_by_zone(&zone_refnos, enable_holes).await?;
```

### 4. 批量查询优化

```rust
// 使用 array::map + array::flatten + array::distinct 模式
let sql = format!(
    r#"
    let $ids = array::distinct(array::filter(array::flatten(array::map([{}], |$refno|
        fn::collect_descendant_ids_by_types($refno, {}, none, "..")
    )), |$v| $v != none));
    SELECT * FROM $ids;
    "#,
    refno_list, types_expr
);

// 性能对比：
// 旧实现（2000节点）: ~55ms（1次收集 + 10次分块过滤）
// 新实现（2000节点）: ~5ms（1次数据库端处理）
// 提升: 91%
```

### 5. 泛型查询函数（推荐）

```rust
use aios_core::collect_descendant_with_expr;

// 查询 ID 列表
let ids: Vec<RefnoEnum> = collect_descendant_with_expr(
    &[refno], &["EQUI"], None, "VALUE id"
).await?;

// 查询完整元素
let elements: Vec<SPdmsElement> = collect_descendant_with_expr(
    &[refno], &["EQUI", "PIPE"], None, "*"
).await?;

// 查询属性映射（带层级范围）
let attrs: Vec<NamedAttrMap> = collect_descendant_with_expr(
    &[refno], &["ZONE"], Some("1..5"), "VALUE id.refno.*"
).await?;
```

---

## SurrealDB 自定义函数速查

### 层级查询函数
- `fn::ancestor($pe)` - 获取祖先节点
- `fn::children($pe)` - 获取子节点
- `fn::first_child($pe)` / `fn::last_child($pe)` - 获取第一个/最后一个子节点
- `fn::collect_children($root, $types)` - 收集子节点
- `fn::collect_descendant_ids_by_types($pe, $types, $inclusive, $range_str)` - 收集子孙 ID
- `fn::collect_descendant_infos($root, $types, $inclusive, $range_str)` - 收集子孙信息
- `fn::collect_descendant_ids_has_inst($root, $types, $inclusive)` - 收集有 inst_relate 的子孙
- `fn::collect_descendants_filter_inst($root, $types, $filter, $include_self, $skip_deleted)` - 过滤已有实例的子孙
- `fn::collect_descendants_filter_spre($root, $types, $filter_inst, $inclusive, $range_str)` - 过滤有 SPRE/CATR 的子孙

### 节点导航函数
- `fn::prev_pe($pe)` / `fn::next_pe($pe)` - 获取兄弟节点
- `fn::prev_pe_exclude_type($pe, 'ATTA')` - 获取上一个节点（排除类型）
- `fn::next_pe_exclude_type($pe, 'ATTA')` - 获取下一个节点（排除类型）
- `fn::find_ancestor_type($pe, 'SPEC')` - 查找特定类型的祖先
- `fn::find_ancestor_types($pe, ['SITE', 'ZONE'])` - 查找多个类型的祖先

### 几何类型函数
- `fn::visible_geo_descendants($root, $include_self, $range_str)` - 可见几何子孙
- `fn::negative_geo_descendants($root, $include_self, $range_str)` - 负实体几何子孙
- `fn::query_negative_entities($pe)` - 查询负实体（union neg_relate 和 ngmr_relate）

### 管道连接函数
- `fn::query_tubi_to($pe)` - 从当前节点出发的直段
- `fn::query_tubi_from($pe)` - 到达当前节点的直段
- `fn::query_bran_first_tubi($pe)` - 查询 BRAN 的第一个直段
- `fn::prev_connect_pe($pe)` / `fn::next_connect_pe($pe)` - 连接节点导航
- `fn::prev_connect_pe_data($pe)` / `fn::next_connect_pe_data($pe)` - 连接节点数据

### 名称和属性函数
- `fn::default_name($pe)` - 获取默认名称
- `fn::default_names($pes)` - 批量获取默认名称
- `fn::default_full_name($pe)` - 获取完整名称
- `fn::ancestor_atts($pe)` - 获取祖先属性

### 版本和历史函数
- `fn::newest_pe($pe)` / `fn::newest_pe_id($pe)` - 获取最新版本
- `fn::latest_pe($pe, $sesno, $dbnum)` - 获取指定时间点的版本
- `fn::find_pe_by_datetime($pe, $datetime)` - 按时间查找版本
- `fn::ses_date($pe)` / `fn::ses_data($pe)` - 会话信息

---

## 查询语法速查

### Record ID 格式
```sql
-- 标准格式
pe:⟨12345_67890⟩
inst_geo:⟨abc123hash⟩
trans:⟨transform_hash⟩
aabb:⟨aabb_hash⟩

-- 复合 ID（用于历史版本）
pe:⟨["12345_67890", 880]⟩

-- 复合 ID（tubi_relate）
tubi_relate:[pe:⟨21491_10000⟩, 0]

-- 动态构建 Record ID
let $pe_id = type::record('pe', '12345_67890');
let $old_pe_id = type::record('pe', [$id, $sesno]);
```

### 图遍历语法
```sql
-- 正向遍历（->）
SELECT * FROM pe:⟨123⟩->inst_relate;
SELECT VALUE out FROM pe:⟨123⟩->tubi_relate;

-- 反向遍历（<-）
SELECT VALUE in FROM pe:⟨123⟩<-pe_owner;
SELECT * FROM pe:⟨123⟩<-pe_owner;

-- 链式遍历
SELECT * FROM pe:⟨123⟩->LSTU->CATR;

-- 条件过滤遍历
SELECT * FROM pe:⟨123⟩<-pe_owner[? !in.deleted];
SELECT VALUE in FROM pe:⟨123⟩<-pe_owner WHERE in.noun = 'BOX';
```

### 递归路径查询
```sql
-- 基本格式：@.{range+options}.field

-- 收集所有子孙（不包含根节点）
SELECT VALUE array::flatten(@.{..+collect}.children) FROM ONLY pe:⟨123⟩;

-- 收集所有子孙（包含根节点）
SELECT VALUE array::flatten(@.{..+collect+inclusive}.children) FROM ONLY pe:⟨123⟩;

-- 限制深度（1-5 层）
SELECT VALUE array::flatten(@.{1..5+collect}.children) FROM ONLY pe:⟨123⟩;

-- 精确 N 层
SELECT VALUE array::flatten(@.{3+collect}.children) FROM ONLY pe:⟨123⟩;

-- 获取子孙节点的特定字段
SELECT VALUE array::flatten(@.{..+collect+inclusive}.children).{ id, noun }
FROM ONLY $root LIMIT 1;

-- 获取祖先链（使用 owner 字段向上递归）
SELECT VALUE @.{..+collect+inclusive}.owner FROM ONLY pe:⟨123⟩ LIMIT 1;
```

### ID Range 查询（推荐）
```sql
-- 查询 tubi_relate 中某个 BRAN 下的所有记录
SELECT * FROM tubi_relate:[pe:⟨bran_refno⟩, 0]..[pe:⟨bran_refno⟩, ..];

-- 字段提取（直接访问复合 ID）
SELECT
    id[0] as refno,           -- 取复合 ID 的第一个元素
    id[0].old_pe as old_refno, -- 访问该 PE 的 old_pe 字段
    in as leave,              -- 直接返回 PE record
    aabb.d as world_aabb      -- 取包围盒的数据部分
FROM tubi_relate:[pe_key, 0]..[pe_key, ..];
```

### 去重查询
```sql
-- ❌ 错误 - SurrealDB 不支持 SELECT DISTINCT
SELECT DISTINCT field FROM table;

-- ✅ 正确 - 使用 array::distinct()
SELECT array::distinct((SELECT VALUE field FROM table)) AS unique_values;

-- 对数组字面量去重
SELECT array::distinct([1, 2, 1, 3, 3, 4]) AS unique_values;

-- 结合 GROUP BY 实现去重
SELECT field FROM table GROUP BY field;
```

### RELATE 关系语句
```sql
-- 基本语法
RELATE $in->relation_table:[$key1, $key2]->$out
SET field1 = value1, field2 = value2;

-- tubi_relate 示例
RELATE pe:⟨leave⟩->tubi_relate:[pe:⟨bran⟩, index]->pe:⟨arrive⟩
SET
    geo = inst_geo:⟨geo_hash⟩,
    aabb = aabb:⟨aabb_hash⟩,
    world_trans = trans:⟨trans_hash⟩,
    bore_size = 'DN100',
    bad = false;

-- inst_relate 示例
RELATE $pe->inst_relate:[$pe_id]->$inst_geo
SET world_trans = $trans, aabb = $aabb;
```

---

## TreeIndex 使用指南

### 何时使用 TreeIndex
- ✅ **层级查询**（子节点、子孙节点、祖先节点）
- ❌ **属性关系**（GMRE、GSTR、LSTU、CATR 等）- 仍需使用 SurrealDB

### 性能对比
| 场景 | SurrealDB 递归 | TreeIndex | 性能提升 |
|------|---------------|-----------|----------|
| 查询 1000 个节点的子孙（10 层） | ~500ms | ~5ms | **100 倍** |
| 查询单层子节点（100 个） | ~50ms | ~0.5ms | **100 倍** |
| 查询祖先（5 层） | ~30ms | ~0.3ms | **100 倍** |
| 批量查询（10 个根节点） | ~5s | ~50ms | **100 倍** |

### 查询路由
| 查询类型 | 使用提供者 | 数据源 |
|---------|-----------|--------|
| 层级查询（子节点/子孙/祖先） | TreeIndex | `.tree` 文件（内存） |
| PE 查询（get_pe） | SurrealDB | `pe` 表 |
| 属性查询（get_named_attmap） | SurrealDB | `named_attr` 表 |
| 实例查询（inst_relate） | SurrealDB | `inst_relate` 表 |

### 初始化（gen_model-dev）
```rust
use crate::fast_model::query_provider::get_model_query_provider;

// 获取查询提供者（自动使用 TreeIndex）
let provider = get_model_query_provider().await?;
// 输出：使用 TreeIndex 查询提供者（层级查询走 indextree）

// 手动初始化（使用大栈线程避免栈溢出）
let handle = std::thread::Builder::new()
    .name("tree-index-loader".to_string())
    .stack_size(64 * 1024 * 1024)  // 64MB 栈
    .spawn(|| TreeIndexQueryProvider::from_tree_dir("output/scene_tree"))?;
let provider = handle.join()??;
```

### 迁移建议
| 旧方式（SurrealDB） | 新方式（TreeIndex） | 性能提升 |
|-------------------|-------------------|----------|
| `SELECT VALUE in FROM pe:⟨refno⟩<-pe_owner` | `collect_children_filter_ids(refno, &[])` | **100 倍** |
| `SELECT VALUE array::flatten(@.{..+collect}.children)` | `collect_descendant_filter_ids(&[refno], &[], None)` | **100 倍** |
| `SELECT VALUE out FROM pe:⟨refno⟩->pe_owner` | `query_filter_ancestors(refno, &[])` | **100 倍** |

---

## 最佳实践清单

### ✅ 推荐做法
1. **层级查询使用 TreeIndex**（性能提升 10-100 倍）
2. **批量查询优于循环查询**（性能提升 N 倍）
3. **使用 `SurrealValue` trait**（类型安全）
4. **使用 ID Range 替代 WHERE 条件**（索引查询）
5. **使用数据库端函数**（`fn::*`，减少网络往返）
6. **限制递归深度**（如 `Some("1..5")`，避免无限递归）
7. **使用 `array::distinct()` 去重**（SurrealDB 不支持 `SELECT DISTINCT`）
8. **直接访问字段**（避免 `record::id()` 函数调用）
9. **使用 `#[serde(alias = "id")]`**（处理字段别名）
10. **分块处理大数据量**（避免内存溢出）

### ❌ 避免做法
1. **使用 `serde_json::Value`**（类型不安全）
2. **循环查询代替批量查询**（性能差 N 倍）
3. **使用 `SELECT DISTINCT`**（SurrealDB 不支持）
4. **使用 `record::id()` 代替直接访问字段**（额外开销）
5. **层级查询使用 SurrealDB 递归**（慢 10-100 倍）
6. **无限递归不限制深度**（可能栈溢出）
7. **WHERE 条件代替 ID Range**（全表扫描）
8. **过度防御性查询**（TreeIndex 已保证类型，批量查询时添加 `noun = 'SITE'` 等过滤是多余的）

---

## 性能陷阱：过度防御性查询

### 问题场景

当使用 TreeIndex 找到 SITE 祖先后，再批量查询 pe 表时添加 `noun = 'SITE'` 过滤：

```sql
-- ❌ 错误：过度防御
SELECT record::id(id) AS rid, spec_value, name
FROM pe
WHERE id IN [pe:17496_10000, pe:17496_10001, ...]
  AND noun = 'SITE'  -- 多余的过滤条件
```

### 问题分析

1. **TreeIndex 已保证类型**：通过 `collect_ancestors_root_to_parent` 找到的 SITE 祖先，已经确保是 SITE 类型
2. **主键查询最快**：`WHERE id IN [...]` 使用主键索引，是最快的查询方式
3. **额外过滤开销**：添加 `noun = 'SITE'` 需要额外的字段比较，降低性能 10-20%
4. **数据一致性风险**：如果 TreeIndex 和 pe 表数据不一致，过滤会导致数据丢失

### 正确做法

```sql
-- ✅ 正确：直接用 ID 查询
SELECT record::id(id) AS rid, spec_value, name
FROM pe
WHERE id IN [pe:17496_10000, pe:17496_10001, ...]
```

### Rust 实现

```rust
// 1. 通过 TreeIndex 找 SITE 祖先（已保证类型）
let site_refnos: Vec<String> = unique_site_refnos
    .iter()
    .map(|r| format!("pe:{}", r))
    .collect();

// 2. 直接批量查询（无需 noun 过滤）
let sql = format!(
    "SELECT record::id(id) AS rid, spec_value, name FROM pe WHERE id IN [{}]",
    site_refnos.join(", ")
);

let results: Vec<SiteSpecValue> = SUL_DB.query_take(&sql, 0).await?;
```

### 性能对比

| 查询方式 | 索引使用 | 性能 | 说明 |
|---------|---------|------|------|
| `WHERE id IN [...]` | 主键索引 | ⚡ 最快 | 推荐 |
| `WHERE id IN [...] AND noun = 'SITE'` | 主键索引 + 字段过滤 | 🐌 慢 10-20% | 过度防御 |

### 何时需要 noun 过滤

仅在以下情况需要 noun 过滤：

1. **不确定类型**：查询来源不保证类型（如用户输入的 refno 列表）
2. **调试验证**：需要验证 TreeIndex 和 pe 表的数据一致性
3. **混合类型查询**：`WHERE noun IN ['SITE', 'ZONE']`

---

## 常见查询场景

### 场景 1：查询区域内的所有设备
```rust
// 使用 TreeIndex（推荐）
let equi_refnos = collect_descendant_filter_ids(
    &[zone_refno],
    &["EQUI"],
    None
).await?;

// 批量查询多个区域
let multi_equi = collect_descendant_filter_ids(
    &[zone1, zone2, zone3],
    &["EQUI", "PIPE"],
    Some("1..5")  // 限制深度
).await?;
```

### 场景 2：查询构件的几何实例
```rust
// 单个构件
let sql = format!(
    "SELECT * FROM pe:{}->inst_relate",
    refno.to_pe_key()
);
let inst_relate: Vec<InstRelateQuery> = SUL_DB.query_take(&sql, 0).await?;

// 批量查询
use aios_core::rs_surreal::query_insts;
let geom_insts = query_insts(&refnos, enable_holes).await?;
```

### 场景 3：查询管道直段（ID Range）
```rust
// 使用 ID Range 查询（推荐）
let sql = format!(
    r#"
    SELECT
        id[0] as refno,
        in as leave,
        aabb.d as world_aabb,
        world_trans.d as world_trans,
        record::id(geo) as geo_hash
    FROM tubi_relate:[{}, 0]..[{}, ..]
    WHERE aabb.d != NONE
    "#,
    bran_pe_key, bran_pe_key
);
let tubi_insts: Vec<TubiInstQuery> = SUL_DB.query_take(&sql, 0).await?;

// 或使用封装函数
use aios_core::rs_surreal::query_tubi_insts_by_brans;
let tubi_insts = query_tubi_insts_by_brans(&bran_refnos).await?;
```

### 场景 4：查询负实体关系
```rust
// 查询负载体（负实体）
let sql = format!(
    "SELECT VALUE pe FROM pe:{}<-neg_relate",
    target_refno.to_pe_key()
);
let neg_carriers: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await?;

// 联合查询 neg_relate 和 ngmr_relate
let sql = format!(
    r#"
    SELECT VALUE pe FROM pe:{}<-neg_relate
    UNION
    SELECT VALUE pe FROM pe:{}<-ngmr_relate
    "#,
    target_pe_key, target_pe_key
);
```

### 场景 5：导出构件的完整几何信息
```sql
SELECT {
    refno: ir.in,
    world_transform: ir.world_trans,
    aabb: ira.out,
    geos: array::map(
        ir.out->geo_relate[WHERE visible = true AND geo_type IN ['Pos', 'DesiPos', 'CatePos']],
        |$gr| {
            geo_hash: $gr.out,
            local_transform: $gr.trans,
            geo_type: $gr.geo_type
        }
    )
}
FROM pe:⟨{refno}⟩->inst_relate AS ir
LEFT JOIN ir.in->inst_relate_aabb AS ira;
```

### 场景 6：查询区域内的所有可见几何
```sql
LET $zone_refnos = fn::collect_descendant_ids_by_types(pe:⟨{site_refno}⟩, ['ZONE'], none, "..");
SELECT
    ir.in as refno,
    gr.out as geo_hash,
    gr.trans as transform
FROM inst_relate ir
INNER JOIN ir.out->geo_relate AS gr
WHERE ir.zone_refno IN $zone_refnos
  AND gr.visible = true
  AND gr.geo_type IN ['Pos', 'DesiPos', 'CatePos'];
```

---

## 代码位置参考

### 核心模块（rs-core）
- **查询扩展**: `src/rs_surreal/query_ext.rs` - SurrealQueryExt trait
- **实例查询**: `src/rs_surreal/inst.rs` - query_insts, query_tubi_insts_by_brans
- **层级查询**: `src/rs_surreal/graph.rs` - collect_descendant_*, collect_children_*
- **PE 查询**: `src/rs_surreal/query.rs` - get_pe, get_named_attmap
- **结构体定义**: `src/rs_surreal/inst_structs.rs` - GeomInstQuery, TubiInstQuery

### 查询提供者
- **SurrealDB 提供者**: `src/query_provider/surreal_provider.rs`
- **TreeIndex 提供者**: `src/query_provider/tree_index_provider.rs`
- **TreeIndex 查询兼容层**: `gen_model-dev/src/fast_model/query_compat.rs`
- **TreeIndex 查询入口**: `gen_model-dev/src/fast_model/query_provider.rs`

### 数据库函数和架构
- **数据库函数定义**: `resource/surreal/common.surql`
- **表结构初始化**: `src/rs_surreal/inst.rs::init_model_tables()`

### 业务模块
- **位号映射**: `src/rs_surreal/tag_name_mapping.rs`
- **测量查询**: `src/rs_surreal/measurement_query.rs`
- **批注查询**: `src/rs_surreal/annotation_query.rs`
- **PBS 查询**: `src/rs_surreal/pbs.rs`
- **MDB 查询**: `src/rs_surreal/mdb.rs`

---

## 可见几何类型

### 正实体类型
```
BOX, CYLI, SLCY, CONE, DISH, CTOR, RTOR, PYRA, SNOU, POHE, POLYHE,
EXTR, REVO, FLOOR, PANE, ELCONN, CMPF, WALL, GWALL, SJOI, FITT, PFIT,
FIXING, PJOI, GENSEC, RNODE, PRTELE, GPART, SCREED, PALJ, CABLE, BATT,
CMFI, SCOJ, SEVE, SBFI, STWALL, SCTN, NOZZ
```

### 负实体类型
```
NBOX, NCYL, NLCY, NSBO, NCON, NSNO, NPYR, NDIS, NXTR, NCTO, NRTO, NREV,
NSCY, NSCO, NLSN, NSSP, NSCT, NSRT, NSDS, NSSL, NLPY, NSEX, NSRE
```

---

## 快速决策树

### 我需要查询层级关系？
- **是** → 使用 TreeIndex（`collect_children_filter_ids`, `collect_descendant_filter_ids`, `query_filter_ancestors`）
  - 性能提升：10-100 倍
  - 适用：子节点、子孙节点、祖先节点
- **否** → 继续

### 我需要查询属性关系？
- **是** → 使用 SurrealDB 图遍历（`query_single_by_paths`）
  - 适用：`->GMRE`, `->GSTR`, `->LSTU->CATR` 等 PDMS 属性关系
  - TreeIndex 不支持属性关系
- **否** → 继续

### 我需要批量查询多个节点？
- **是** → 使用 `array::map` + 数据库端函数（`fn::*`）
  - 性能提升：91%（2000节点：55ms → 5ms）
  - 避免循环查询
- **否** → 使用单节点查询

### 我需要查询几何实例？
- **是** → 使用封装函数
  - `query_insts(&refnos, enable_holes)` - 几何实例
  - `query_tubi_insts_by_brans(&bran_refnos)` - 管道直段
  - `query_insts_by_zone(&zone_refnos, enable_holes)` - 按区域查询
- **否** → 使用通用查询

### 我需要去重？
- **是** → 使用 `array::distinct()`（SurrealDB 不支持 `SELECT DISTINCT`）
- **否** → 继续

---

## 参考文档

### 项目文档（rs-core）
- **数据库查询总结**: `docs/数据库查询总结/数据库查询总结.md` - 完整查询语法和使用模式
- **数据库架构**: `docs/数据库查询总结/数据库架构.md` - 表结构详细说明
- **常用查询方法**: `docs/数据库查询总结/常用查询方法.md` - Rust API 使用指南
- **tubi_relate 查询指南**: `docs/数据库查询总结/tubi_relate查询指南.md` - 管道直段查询
- **SurrealDB 函数参考**: `docs/数据库查询总结/SurrealDB函数参考.md` - 数据库端函数

### 架构文档
- **数据库架构文档**: `数据库架构文档.md` - 完整的数据库架构说明
- **模型生成表结构**: `gen_model-dev/开发文档/模型生成/01_数据表结构与保存流程.md`
