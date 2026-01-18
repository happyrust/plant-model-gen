# export_dbnum_instances_json 开发方案

## 1. 概述

### 1.1 目标
创建一个新函数 `export_dbnum_instances_json`，用于扫描 `inst_relate` 表的数据，生成简化的 JSON 文件，同时为每个 refno 添加从 `inst_relate_aabb` 获取的 AABB 包围盒数据。

### 1.2 背景需求
- 前端需要获取模型实例的 AABB 包围盒数据用于空间查询和碰撞检测
- 现有的 `instances_*.json` 格式包含不必要的字段（如 `color_index`、`geo_index`、`colors` 数组）
- 需要更简洁的 JSON 结构以提高传输效率和解析性能

## 2. 功能需求

### 2.1 JSON 结构简化
使用扁平的 `groups` 数组替代 `bran_groups/equi_groups/ungrouped` 三层结构：

```json
{
  "version": 2,
  "generated_at": "2026-01-14T...",
  "groups": [
    {
      "owner_refno": "...",
      "owner_noun": "BRAN",  // or "EQUI"
      "owner_name": "...",
      "children": [...],
      "tubings": [...]
    }
  ]
}
```

### 2.2 字段变更

#### 移除的字段
| 字段 | 位置 | 原因 |
|------|------|------|
| `colors` | 顶层数组 | 前端自行管理颜色调色板 |
| `color_index` | 每个组件 | 不需要颜色索引 |
| `geo_index` | instances 中 | `geo_hash` 已足够标识几何体 |
| `name_index` | 管道实例 | 不需要名称索引 |

#### 新增的字段
| 字段 | 类型 | 格式 |
|------|------|------|
| `aabb` | 每个组件 | `{ "min": [x, y, z], "max": [x, y, z] }` |

### 2.3 调用方式
- **CLI 命令**: `cargo run -- export-dbnum-instances-json --dbno 1112 --verbose`
- **内部调用**: 可被其他导出流程调用

## 3. 技术设计

### 3.1 数据库聚合策略

#### 查询 1: inst_relate 聚合查询
使用 SurrealDB 的 `GROUP BY` 在数据库层面完成 BRAN/EQUI 分组：

```sql
-- 关联查询 inst_relate + AABB + 几何体数据
SELECT
    owner_refno,
    owner_type,
    record::id(in) as refno,
    in.noun as noun,
    in.dbnum as dbnum,
    out.spec_value as spec_value,
    -- AABB 数据（通过 ->inst_relate_aabb 关联）
    (SELECT out.d FROM in->inst_relate_aabb) as aabb,
    -- 几何体实例数据（通过 out 关联获取）
    ...
FROM inst_relate
WHERE owner_type IN ['BRAN', 'HANG', 'EQUI']
    AND in.dbnum = $dbno
GROUP BY owner_refno;
```

**优势**:
- 在数据库层面完成分组，减少数据传输量
- 一次性获取所有相关数据，减少查询次数
- 利用数据库索引优化查询性能

#### 查询 2: tubi_relate 范围查询
tubings 数据使用 ID ranges 查询：

```sql
-- 查询 tubi_relate 及其 AABB 数据
SELECT
    id[0] as refno,           -- BRAN/HANG 的 refno
    in as leave,              -- leave 引用
    id[0].owner.noun as generic,
    aabb.d as world_aabb,     -- AABB 数据
    world_trans.d as world_trans,  -- 变换矩阵
    record::id(geo) as geo_hash,
    id[0].dt as date,
    spec_value
FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..]
```

**说明**:
- `pe_key` 是 BRAN/HANG 的 refno 转换的 pe key 格式
- 需要为每个 BRAN refno 执行一次 ID ranges 查询
- `aabb.d` 包含 `{ mins: Point3, maxs: Point3 }` 格式的包围盒数据
- `world_trans.d` 是变换矩阵（16 个 f64）

### 3.2 数据流

```
1. SurrealDB 聚合查询 (inst_relate)
   ├─ GROUP BY owner_refno
   ├─ WHERE owner_type IN ['BRAN', 'HANG', 'EQUI']
   ├─ WHERE in.dbnum = $dbno
   └─ 关联查询 AABB 数据 (inst_relate_aabb)
    ↓
2. SurrealDB 查询 (tubi_relate)
   ├─ WHERE id IN $bran_refnos
   └─ 关联查询 AABB 数据
    ↓
3. Rust 数据结构（已分组）
   ├─ groups (按 owner_refno 分组)
   └─ tubings (按 bran_refno 分组)
    ↓
4. build_instances_payload_json
    ↓
5. instances_{dbno}.json (简化格式)
```

### 3.3 AABB 数据格式转换

数据库中的 AABB 格式（parry3d::Aabb）:
```json
{
  "mins": { "x": 100.0, "y": 200.0, "z": 300.0 },
  "maxs": { "x": 150.0, "y": 250.0, "z": 350.0 }
}
```

输出的 AABB 格式:
```json
{
  "min": [100.0, 200.0, 300.0],
  "max": [150.0, 250.0, 350.0]
}
```

### 3.4 AABB 查询与计算机制差异说明

本功能需要处理三种不同类型的 AABB 数据，它们使用不同的存储和计算机制。

#### 3.4.1 BRAN/EQUI Owner AABB（动态计算 Union）

**数据源**: 动态计算（不是直接查询）

**计算方式**: 对子组件和 tubi 的 AABB 取并集（Union）
```
owner_aabb = union(all_children_aabb ∪ all_tubings_aabb)
```

**实现步骤**:
1. 查询 BRAN/EQUI 下所有子组件的 AABB（从 `inst_relate_aabb`）
2. 查询 BRAN/EQUI 下所有 tubi 的 AABB（从 `tubi_relate.aabb`）
3. 对所有 AABB 取并集，计算出 owner 的整体 AABB
4. 输出格式：`{ "min": [x, y, z], "max": [x, y, z] }`

**伪代码**:
```rust
fn compute_owner_aabb(
    children_aabbs: Vec<Option<Aabb>>,
    tubings_aabbs: Vec<Option<Aabb>>,
) -> Option<Aabb> {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut min_z = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut max_z = f64::MIN;

    // 合并所有子组件的 AABB
    for aabb in children_aabbs.iter().flatten() {
        min_x = min_x.min(aabb.mins.x);
        min_y = min_y.min(aabb.mins.y);
        min_z = min_z.min(aabb.mins.z);
        max_x = max_x.max(aabb.maxs.x);
        max_y = max_y.max(aabb.maxs.y);
        max_z = max_z.max(aabb.maxs.z);
    }

    // 合并所有 tubi 的 AABB
    for aabb in tubings_aabbs.iter().flatten() {
        min_x = min_x.min(aabb.mins.x);
        min_y = min_y.min(aabb.mins.y);
        min_z = min_z.min(aabb.mins.z);
        max_x = max_x.max(aabb.maxs.x);
        max_y = max_y.max(aabb.maxs.y);
        max_z = max_z.max(aabb.maxs.z);
    }

    if min_x == f64::MAX {
        None  // 没有有效的 AABB
    } else {
        Some(Aabb {
            mins: Point3 { x: min_x, y: min_y, z: min_z },
            maxs: Point3 { x: max_x, y: max_y, z: max_z },
        })
    }
}
```

**特点**:
- BRAN/EQUI 作为分组节点，其 AABB 需要动态计算
- 不是预先存储的，而是在导出时实时计算
- 通过 Union 所有子节点的 AABB 得到整体包围盒

#### 3.4.2 Components AABB（查询关系表）

**数据源**: `inst_relate_aabb` 关系表

**表结构**:
```sql
DEFINE TABLE inst_relate_aabb TYPE RELATION;
DEFINE FIELD in ON TABLE inst_relate_aabb TYPE record<pe>;
DEFINE FIELD out ON TABLE inst_relate_aabb TYPE record<aabb>;
```

**查询方式**: 使用 SurrealDB 图遍历
```sql
SELECT
    record::id(in) as refno,
    -- 子组件的 AABB（通过 in 查询）
    (SELECT out.d FROM in->inst_relate_aabb) as aabb
FROM inst_relate
WHERE owner_type IN ['BRAN', 'HANG', 'EQUI']
    AND in.dbnum = $dbno;
```

**特点**:
- AABB 存储在独立的关系表中
- 一个 pe (component) 只能对应一条 AABB 记录
- 通过图遍历语法 `in->inst_relate_aabb` 查询
- 子组件的 AABB 是预先计算并存储的

#### 3.4.3 Tubings AABB（查询表字段）

**数据源**: `tubi_relate` 表的 `aabb` 字段

**表结构**:
```sql
DEFINE FIELD aabb ON TABLE tubi_relate TYPE record<aabb>;
```

**查询方式**: 直接字段访问
```sql
SELECT
    id[0] as refno,
    aabb.d as world_aabb,     -- 直接访问字段
    world_trans.d as world_trans
FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..]
```

**特点**:
- AABB 数据预计算并直接存储在主表中
- 通过字段路径 `aabb.d` 直接访问
- 查询效率最高

#### 3.4.4 对比总结

| 特性 | BRAN/EQUI Owner | Components | Tubings |
|------|----------------|-----------|---------|
| 数据来源 | 动态计算 Union | 关系表查询 | 表字段查询 |
| 计算方式 | `union(children ∪ tubi)` | `in->inst_relate_aabb` | `aabb.d` |
| 是否预存储 | ❌ 否（实时计算） | ✅ 是 | ✅ 是 |
| 查询性能 | 较低（需多次查询+计算） | 中等（需要 JOIN） | 高（直接访问） |

## 4. JSON 输出格式

### 4.1 完整示例

```json
{
  "version": 2,
  "generated_at": "2026-01-14T05:05:12.640Z",
  "groups": [
    {
      "owner_refno": "17496_170847",
      "owner_noun": "BRAN",
      "owner_name": "1RSI0035-273-PMC-S03-S013/S011",
      "children": [
        {
          "refno": "17496_170848",
          "noun": "ATTA",
          "name": "1RSI-S013.012/ATTA-01",
          "aabb": {
            "min": [100.0, 200.0, 300.0],
            "max": [150.0, 250.0, 350.0]
          },
          "lod_mask": 7,
          "spec_value": 0,
          "refno_transform": [
            -1.000000238418579, 1.2246469849340659e-16, 2.5849394142282115e-26, 0.0,
            -1.2246361323244292e-16, -0.9999914169311523, 0.004206231329590082, 0.0,
            5.151148357460925e-19, 0.004206231329590082, 0.9999911785125732, 0.0,
            -33770.171875, 5398.58984375, -10802.3603515625, 1.0
          ],
          "instances": [
            {
              "geo_hash": "9067713634515800924",
              "geo_transform": [
                -35.1524543762207, 0.0, -136.22850036621094, 0.0,
                136.22850036621094, 0.0, -35.1524543762207, 0.0,
                0.0, -200.0, 0.0, 0.0,
                7.62939453125e-6, 0.0, 0.0009613037109375, 1.0
              ]
            }
          ]
        }
      ],
      "tubings": [
        {
          "refno": "17496_170849",
          "noun": "TUBI",
          "name": "1RSI-S013.012/TUBI-01",
          "aabb": {
            "min": [100.0, 200.0, 300.0],
            "max": [150.0, 250.0, 350.0]
          },
          "geo_hash": "12345678901234567890",
          "matrix": [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0
          ],
          "order": 0,
          "lod_mask": 7,
          "spec_value": 0
        }
      ]
    },
    {
      "owner_refno": "17496_170850",
      "owner_noun": "EQUI",
      "owner_name": "/SITE/ZONE/EQUI-001",
      "children": [
        ...
      ]
    }
  ]
}
```

### 4.2 字段说明

#### 顶层字段
| 字段 | 类型 | 说明 |
|------|------|------|
| `version` | number | JSON 格式版本号 |
| `generated_at` | string | 生成时间（ISO 8601） |
| `groups` | array | 分组数组（扁平结构） |

#### Group 字段
| 字段 | 类型 | 说明 |
|------|------|------|
| `owner_refno` | string | 所有者参考号 |
| `owner_noun` | string | 所有者类型（BRAN/EQUI） |
| `owner_name` | string | 所有者名称 |
| `children` | array | 子组件数组 |
| `tubings` | array | 管道数组（仅 BRAN） |

#### Component 字段
| 字段 | 类型 | 说明 |
|------|------|------|
| `refno` | string | 组件参考号 |
| `noun` | string | 组件类型 |
| `name` | string | 组件名称 |
| `aabb` | object | 包围盒数据 |
| `lod_mask` | number | LOD 掩码 |
| `spec_value` | number | 规格值 |
| `refno_transform` | array | 参考号变换矩阵（16 个 f32） |
| `instances` | array | 几何体实例数组 |

#### Instance 字段
| 字段 | 类型 | 说明 |
|------|------|------|
| `geo_hash` | string | 几何体哈希值 |
| `geo_transform` | array | 几何体变换矩阵（16 个 f32） |

#### Tubing 字段
| 字段 | 类型 | 说明 |
|------|------|------|
| `refno` | string | 管道参考号 |
| `noun` | string | 类型（TUBI） |
| `name` | string | 管道名称 |
| `aabb` | object | 包围盒数据 |
| `geo_hash` | string | 几何体哈希值 |
| `matrix` | array | 变换矩阵（16 个 f32） |
| `order` | number | 顺序号 |
| `lod_mask` | number | LOD 掩码 |
| `spec_value` | number | 规格值 |

## 5. 实现步骤

### 5.1 核心函数实现

#### 步骤 1: 在 export_prepack_lod.rs 中添加 AABB 查询函数

```rust
/// 批量查询 refno 对应的 AABB 数据
async fn query_refno_aabbs(refnos: &[RefnoEnum]) -> Result<HashMap<RefnoEnum, ([f64; 3], [f64; 3])>> {
    // 实现数据库查询逻辑
    // 返回 HashMap<RefnoEnum, (min, max)>
}
```

#### 步骤 2: 创建 export_dbnum_instances_json 函数

```rust
pub async fn export_dbnum_instances_json(
    dbno: u32,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
) -> Result<ExportStats> {
    // 1. 执行数据库聚合查询，获取按 owner_refno 分组的数据
    // 2. 查询 tubi_relate 获取管道数据
    // 3. 处理查询结果，构建 Rust 数据结构
    // 4. 调用 build_instances_payload_json 生成 JSON
    // 5. 写入 instances_{dbno}.json 文件
}
```

#### 步骤 3: 创建简化的 build_instances_payload_json 函数

```rust
fn build_instances_payload_json(
    export_data: &ExportData,
    aabb_map: &HashMap<RefnoEnum, ([f64; 3], [f64; 3])>,
    generated_at: &str,
    refno_name_map: &HashMap<RefnoEnum, String>,
    equi_owners: &[RefnoEnum],
) -> serde_json::Value {
    // 构建简化的 JSON，移除 colors、color_index、geo_index、name_index
    // 添加 aabb 字段

    json!({
        "version": 2,
        "generated_at": generated_at,
        "groups": groups
    })
}
```

### 5.2 CLI 命令实现

#### 步骤 4: 在 cli_modes.rs 中添加 CLI 模式

```rust
pub async fn export_dbnum_instances_json_mode(
    dbno: u32,
    verbose: bool,
    output_override: Option<PathBuf>,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    // CLI 命令实现
}
```

#### 步骤 5: 在 main.rs 中注册子命令

```rust
.subcommand(Command::new("export-dbnum-instances-json")
    .about("导出 dbnum 的实例数据为 JSON（含 AABB）")
    .arg(arg!(<dbno> "数据库编号")
        .value_parser(value_parser!(u32)))
    .arg(arg!(-v --verbose "详细输出"))
    .arg(arg!(-o --output [DIR] "输出目录")))
```

### 5.3 测试实现

#### 步骤 6: 创建独立测试程序

文件: `src/bin/test_export_dbnum_instances_json.rs`

```rust
//! 测试 export_dbnum_instances_json 函数
//!
//! 运行方式:
//! cargo run --bin test_export_dbnum_instances_json --features="web_server" -- 1112

use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dbnum: u32 = if args.len() > 1 {
        args[1].parse().unwrap_or(1112)
    } else {
        1112
    };

    println!("🚀 测试 export_dbnum_instances_json dbnum={}", dbnum);

    aios_core::init_db_from_env().await?;
    let db_option = Arc::new(aios_core::options::DbOption::from_env()?);
    let output_dir = PathBuf::from("output/instances");

    let stats = aios_database::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json(
        dbnum,
        &output_dir,
        db_option,
        true,
    )
    .await?;

    println!("✅ 导出完成！");
    println!("📊 统计: {:?}", stats);

    Ok(())
}
```

#### 步骤 7: 创建单元测试

文件: `src/test/test_export/test_export_dbnum_instances.rs`

```rust
//! 测试 export_dbnum_instances_json 函数

use std::path::PathBuf;
use std::sync::Arc;
use crate::test::test_helper::get_test_ams_db_manager_async;
use crate::test::test_query::init_test_surreal;

#[tokio::test]
async fn test_export_dbnum_instances_json_1112() {
    init_test_surreal().await;

    let dbnum = 1112;
    let output_dir = PathBuf::from("output/test/instances");
    let db_option = Arc::new(get_test_ams_db_manager_async().await.db_option().clone());

    let result = aios_database::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json(
        dbnum,
        &output_dir,
        db_option,
        false,
    )
    .await;

    assert!(result.is_ok(), "导出应该成功");

    let stats = result.unwrap();
    assert!(stats.total_refnos > 0, "应该有导出的 refno");

    // 验证生成的 JSON 文件
    let json_path = output_dir.join(format!("instances_{}.json", dbnum));
    assert!(json_path.exists(), "JSON 文件应该存在");

    let json_content = std::fs::read_to_string(&json_path).unwrap();
    let json_value: serde_json::Value = serde_json::from_str(&json_content).unwrap();

    // 验证基本结构
    assert_eq!(json_value["version"], 2);
    assert!(json_value["groups"].is_array());
    assert!(!json_value["colors"].is_array(), "不应该有 colors 数组");

    // 验证 groups 结构
    if let Some(groups) = json_value["groups"].as_array() {
        if let Some(first_group) = groups.first() {
            assert!(first_group["owner_refno"].is_string());
            assert!(first_group["owner_noun"].is_string());

            // 验证 children 有 aabb 字段
            if let Some(children) = first_group["children"].as_array() {
                if let Some(first_child) = children.first() {
                    assert!(first_child["aabb"].is_object(), "child 应该有 aabb 字段");
                    assert!(first_child["color_index"].is_null(), "不应该有 color_index");
                }
            }

            // 验证 tubings 有 aabb 字段
            if let Some(tubings) = first_group["tubings"].as_array() {
                if let Some(first_tubi) = tubings.first() {
                    assert!(first_tubi["aabb"].is_object(), "tubi 应该有 aabb 字段");
                    assert!(first_tubi["color_index"].is_null(), "不应该有 color_index");
                    assert!(first_tubi["name_index"].is_null(), "不应该有 name_index");
                }
            }
        }
    }
}

#[tokio::test]
async fn test_export_dbnum_instances_json_aabb_format() {
    // 测试 AABB 格式正确性
    init_test_surreal().await;

    let dbnum = 1112;
    let output_dir = PathBuf::from("output/test/instances_aabb");
    let db_option = Arc::new(get_test_ams_db_manager_async().await.db_option().clone());

    aios_database::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json(
        dbnum,
        &output_dir,
        db_option,
        false,
    )
    .await
    .unwrap();

    let json_path = output_dir.join(format!("instances_{}.json", dbnum));
    let json_content = std::fs::read_to_string(&json_path).unwrap();
    let json_value: serde_json::Value = serde_json::from_str(&json_content).unwrap();

    // 验证 AABB 格式 { "min": [x, y, z], "max": [x, y, z] }
    if let Some(groups) = json_value["groups"].as_array() {
        for group in groups {
            if let Some(children) = group["children"].as_array() {
                for child in children {
                    if let Some(aabb) = child["aabb"].as_object() {
                        assert!(aabb.contains_key("min"));
                        assert!(aabb.contains_key("max"));

                        if let Some(min) = aabb["min"].as_array() {
                            assert_eq!(min.len(), 3, "min 应该有 3 个元素");
                        }
                        if let Some(max) = aabb["max"].as_array() {
                            assert_eq!(max.len(), 3, "max 应该有 3 个元素");
                        }
                    }
                }
            }
        }
    }
}
```

#### 步骤 8: 添加模块声明

创建 `src/test/test_export/mod.rs`:
```rust
pub mod test_export_dbnum_instances;
```

更新 `src/test/mod.rs`:
```rust
pub mod test_export;
```

## 6. 关键文件清单

### 6.1 需要修改的文件

| 文件路径 | 修改内容 |
|---------|---------|
| `src/fast_model/export_model/export_prepack_lod.rs` | 添加核心导出函数 |
| `src/cli_modes.rs` | 添加 CLI 命令入口 |
| `src/main.rs` | 注册子命令 |

### 6.2 需要创建的文件

| 文件路径 | 说明 |
|---------|------|
| `src/bin/test_export_dbnum_instances_json.rs` | 独立测试程序 |
| `src/test/test_export/mod.rs` | 测试模块声明 |
| `src/test/test_export/test_export_dbnum_instances.rs` | 单元测试 |

## 7. 验证方法

### 7.1 单元测试
```bash
# 运行所有导出测试
cargo test --lib test_export

# 运行特定测试
cargo test --lib test_export_dbnum_instances_json_1112
cargo test --lib test_export_dbnum_instances_json_aabb_format
```

### 7.2 集成测试（CLI）
```bash
# 导出 dbnum=1112 的数据
cargo run -- export-dbnum-instances-json --dbno 1112 --verbose

# 指定输出目录
cargo run -- export-dbnum-instances-json --dbno 1112 -o /path/to/output
```

### 7.3 独立测试程序
```bash
# 使用默认 dbnum (1112)
cargo run --bin test_export_dbnum_instances_json --features="web_server"

# 指定 dbnum
cargo run --bin test_export_dbnum_instances_json --features="web_server" -- 1112
```

### 7.4 验证清单

运行测试后，检查生成的 `instances_1112.json` 文件：

- [ ] 使用 `groups` 数组（不是 bran_groups/equi_groups）
- [ ] 每个 component 有 `aabb` 字段
- [ ] 没有 `colors` 数组
- [ ] 没有 `color_index` 字段
- [ ] 没有 `geo_index` 字段
- [ ] 没有 `name_index` 字段
- [ ] AABB 格式为 `{ "min": [x, y, z], "max": [x, y, z] }`
- [ ] 所有必需字段都存在（refno, noun, name, lod_mask, spec_value 等）

## 8. 注意事项

### 8.1 AABB 数据处理
- **可能为空**: 某些 refno 可能没有生成 AABB 数据，需要处理 None 情况
- **单位转换**: 如果启用了单位转换，AABB 坐标也需要进行转换
- **格式转换**: 从 parry3d::Aabb 的 `{ mins: Point3, maxs: Point3 }` 转换为 `{ min: [x,y,z], max: [x,y,z] }`

### 8.2 性能考虑
- **批量查询**: refno 数量可能很大，需要分批查询（建议每批 2000 条）
- **数据库索引**: 确保查询字段有索引：
  - `inst_relate.owner_type`
  - `inst_relate.in.dbnum`
  - `inst_relate_aabb.in`
- **内存管理**: 大数据量时注意内存使用，可以考虑流式写入

### 8.3 兼容性
- **版本号**: JSON 格式版本号为 2，与现有格式区分
- **向后兼容**: 现有的 `export_instances_json_by_dbnum` 函数保持不变
- **渐进迁移**: 前端可以逐步迁移到新格式

## 9. 时间估算

| 任务 | 预估时间 |
|------|---------|
| 核心函数实现 | 4 小时 |
| CLI 命令实现 | 1 小时 |
| 单元测试编写 | 2 小时 |
| 集成测试 | 1 小时 |
| 文档更新 | 1 小时 |
| **总计** | **9 小时** |

## 10. 相关文档

- [现有 instances_*.json 格式](../gen-model-fork/output/instances/instances_1112.json)
- [inst_relate 表结构](../gen-model-fork/src/data_interface/surreal_schema.sql)
- [export_prepack_lod.rs 实现](../gen-model-fork/src/fast_model/export_model/export_prepack_lod.rs)
- [测试参考](../gen-model-fork/src/bin/test_export_instances.rs)
