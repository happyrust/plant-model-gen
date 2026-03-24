# 实例导出 v3 开发方案

> 参考 Parquet 导出的规范化数据模型，精简 JSON 实例导出格式，配套修改 delivery-code 前端加载器。

## 1. 背景与动机

### 1.1 现状

plant-model-gen 有三条导出路径，各自的数据模型和单位处理策略不统一：

| 导出路径 | 格式 | 矩阵存储 | 单位转换 | 坐标旋转 | 冗余字段 |
|---------|------|---------|---------|---------|---------|
| `export_prepack_lod.rs` | JSON v2 | `geo_transform` + `refno_transform` 分级内联 | 服务端做（区分 unit_flag） | 不做 | color_index, name_index, lod_mask, name |
| `export_dbnum_instances_web.rs` | JSON v2 | `matrix` 合并内联 + `uniforms` 块 | 不做（前端全处理） | 不做 | color_index, name_index, site_name_index |
| `export_dbnum_instances_parquet.rs` | Parquet | trans_hash 引用（去重） | 服务端做（仅平移） | 不做 | 无冗余 |

### 1.2 问题

1. **字段冗余**：`color_index`、`name_index`、`site_name_index`、`lod_mask` 前端已不再需要
2. **矩阵膨胀**：每个 instance 内联 16 个 f32，大量共享变换被重复存储
3. **字段不统一**：`geo_transform` vs `matrix`、有/无 `uniforms` 块、层级位置不一致
4. **单位/旋转策略混乱**：三条路径各不相同，前端需要分别适配

### 1.3 目标

- 采用 Parquet 的规范化模型（trans_hash 引用），但保留 JSON 格式
- 去掉所有已不需要的字段
- 单位转换和坐标旋转做成可配置参数
- 双端（plant-model-gen + delivery-code）同步改造

## 2. 设计

### 2.1 导出变换配置（ExportTransformConfig）

```rust
pub struct ExportTransformConfig {
    /// 源单位（SurrealDB 存储单位，默认 mm）
    pub source_unit: LengthUnit,
    /// 目标单位（导出单位，如 mm / m / ft）
    pub target_unit: LengthUnit,
    /// 是否在导出时做坐标系旋转（Z-up → Y-up, 绕 X 轴 -90°）
    pub apply_rotation: bool,
    /// 是否在导出时将 trans_hash 解析为内联矩阵
    /// - true: 矩阵内联到每个 instance（兼容 v2 前端）
    /// - false: 矩阵存入顶层 transforms 字典（v3 去重模式）
    pub inline_matrices: bool,
}
```

对应 manifest.json 输出：

```json
{
  "export_transform": {
    "source_unit": "mm",
    "target_unit": "m",
    "rotation_applied": false,
    "matrices_inlined": false
  }
}
```

前端根据此配置决定残差处理：
- 若 `rotation_applied=false`，前端补做 Z-up → Y-up
- 若 `target_unit` 与渲染单位不同，前端补做残差缩放

### 2.2 v3 JSON Schema

```json
{
  "version": 3,
  "format": "json",
  "generated_at": "2026-03-24T12:00:00Z",
  "dbnum": 123,

  "export_transform": {
    "source_unit": "mm",
    "target_unit": "mm",
    "rotation_applied": false,
    "matrices_inlined": false
  },

  "transforms": {
    "abc123": [1,0,0,0, 0,1,0,0, 0,0,1,0, 100,200,300,1],
    "def456": [...]
  },

  "aabb": {
    "xyz789": [min_x, min_y, min_z, max_x, max_y, max_z]
  },

  "bran_groups": [
    {
      "refno": "21491",
      "noun": "BRAN",
      "children": [
        {
          "refno": "21491_18946",
          "noun": "ELBO",
          "owner_noun": "BRAN",
          "trans_hash": "abc123",
          "aabb_hash": "xyz789",
          "spec_value": 150,
          "has_neg": false,
          "geos": [
            {
              "geo_hash": "90824",
              "geo_index": 0,
              "geo_trans_hash": "def456",
              "unit_mesh": false
            }
          ]
        }
      ],
      "tubings": [
        {
          "refno": "21491_50001",
          "owner_refno": "21491",
          "order": 0,
          "geo_hash": "t_5",
          "trans_hash": "ghi012",
          "aabb_hash": "jkl345",
          "spec_value": 150
        }
      ]
    }
  ],

  "equi_groups": [ ... ],
  "ungrouped": [ ... ]
}
```

### 2.3 与 v2 的字段变化对照

| v2 字段 | v3 处理 | 原因 |
|---------|--------|------|
| `color_index` | 删除 | 前端不再需要 |
| `name_index` / `site_name_index` | 删除 | 前端不再需要 |
| `name` | 删除 | 前端不再需要 |
| `names` 表 | 删除 | 前端不再需要 |
| `lod_mask` | 删除 | 前端可从 geometry_manifest 推导 |
| `uniforms: { refno, owner_refno, owner_noun }` | 展平为顶层字段 `refno`, `owner_noun` | 减少嵌套 |
| `matrix` (16 f32 内联) | → `trans_hash` (引用) | 去重，体积缩减 40-60% |
| `instances` 数组 | 更名为 `geos` | 与 Parquet 的 `geo_instances` 对齐 |
| `geo_transform` / `refno_transform` | → `geo_trans_hash` / `trans_hash` (引用) | 去重 + 统一命名 |
| 新增 `aabb_hash` | 引用顶层 `aabb` 字典 | 支持空间查询/视锥裁剪 |
| 新增 `unit_mesh` | 标记几何体是否为标准单位网格 | 前端需要此信息决定缩放策略 |

### 2.4 体积估算

以 10 万实例、1 万唯一变换为例：
- v2：10万 × 16×4B (matrix) ≈ 6.4MB 矩阵数据
- v3：1万 × 16×4B (字典) + 10万 × ~20B (hash 引用) ≈ 2.6MB
- 预估缩减：**~60%**

## 3. 改造范围

### 3.1 plant-model-gen（Rust 导出端）

#### 3.1.1 新建 `ExportTransformConfig`

文件：`src/fast_model/export_model/export_transform_config.rs`

```rust
use crate::fast_model::unit_converter::LengthUnit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportTransformConfig {
    pub source_unit: LengthUnit,
    pub target_unit: LengthUnit,
    pub apply_rotation: bool,
    pub inline_matrices: bool,
}

impl Default for ExportTransformConfig {
    fn default() -> Self {
        Self {
            source_unit: LengthUnit::Millimeter,
            target_unit: LengthUnit::Millimeter,
            apply_rotation: false,
            inline_matrices: false,
        }
    }
}
```

#### 3.1.2 新建 `export_dbnum_instances_v3.rs`

核心逻辑参考 `export_dbnum_instances_parquet.rs`（数据收集和分组），输出为 JSON。

主要函数：

```rust
pub async fn export_dbnum_instances_v3(
    dbnum: u32,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
    transform_config: ExportTransformConfig,
    root_refno: Option<RefnoEnum>,
) -> Result<V3ExportStats>
```

流程：
1. 查询 inst_relate（复用现有逻辑）
2. 按 BRAN/HANG、EQUI、ungrouped 分组
3. 查询 geo_relate 获取 geo_hash + geo_trans_hash
4. 查询 tubi_relate
5. 收集所有唯一 trans_hash 和 aabb_hash
6. 批量查询 trans 表和 aabb 表
7. 根据 `transform_config` 处理矩阵（单位转换、旋转）
8. 根据 `inline_matrices` 决定输出格式
9. 组装 JSON 并写入

#### 3.1.3 矩阵处理逻辑

```rust
fn process_transform(
    raw_mat: DMat4,
    config: &ExportTransformConfig,
    unit_flag: bool,
) -> [f32; 16] {
    let mut mat = raw_mat;

    // 1. 单位转换
    if config.source_unit != config.target_unit {
        let factor = UnitConverter::new(config.source_unit, config.target_unit)
            .conversion_factor() as f64;
        // 平移始终缩放
        mat.w_axis.x *= factor;
        mat.w_axis.y *= factor;
        mat.w_axis.z *= factor;
        // unit_mesh: 旋转/缩放列也缩放
        if unit_flag {
            mat.x_axis *= DVec3::splat(factor).extend(0.0);
            mat.y_axis *= DVec3::splat(factor).extend(0.0);
            mat.z_axis *= DVec3::splat(factor).extend(0.0);
        }
    }

    // 2. 坐标旋转 Z-up → Y-up
    if config.apply_rotation {
        let rot = DMat4::from_rotation_x(-std::f64::consts::FRAC_PI_2);
        mat = rot * mat;
    }

    mat.to_cols_array().map(|v| v as f32)
}
```

注意：当 `inline_matrices=false` 时，trans 字典中的矩阵统一以 **非 unit_mesh** 模式处理（仅缩放平移），unit_mesh 的额外缩放通过 `geos[].unit_mesh` 标记让前端在组装时处理。这样保证同一个 trans_hash 在字典中有唯一值。

#### 3.1.4 CLI / Web Server 入口

在 `cli_modes.rs` 中新增：

```
--export-v3 [--target-unit m] [--apply-rotation] [--inline-matrices]
```

在 `stream_generate.rs` 的 `StreamGenerateRequest` 中新增：

```rust
#[serde(default)]
pub export_v3: bool,
#[serde(default)]
pub export_transform_config: Option<ExportTransformConfig>,
```

#### 3.1.5 mod.rs 注册

在 `src/fast_model/export_model/mod.rs` 中添加：

```rust
pub mod export_transform_config;
pub mod export_dbnum_instances_v3;
```

### 3.2 delivery-code（JS 前端消费端）

#### 3.2.1 v3 加载路径

在 `AiosPrepackLoader.js` 中扩展或新建 `AiosPrepackLoaderV3.js`：

```javascript
// 检测版本
if (manifest.version === 3) {
    return this.loadV3(manifest);
}

async loadV3(manifest) {
    // 1. 加载 transforms 字典
    this.transformsDict = manifest.transforms; // Map<hash, Float32Array(16)>

    // 2. 加载 aabb 字典
    this.aabbDict = manifest.aabb;

    // 3. 读取导出配置
    const exportConfig = manifest.export_transform || {};
    this.needsRuntimeRotation = !exportConfig.rotation_applied;
    this.needsRuntimeScale = this.computeResidualScale(exportConfig);

    // 4. 遍历 bran_groups / equi_groups / ungrouped
    this.processV3Groups(manifest);
}
```

#### 3.2.2 矩阵组装

```javascript
assembleMatrix(component, geo) {
    // 1. 查找世界变换
    const worldMat = this.transformsDict[component.trans_hash] || IDENTITY_16;

    // 2. 查找几何变换
    const geoMat = this.transformsDict[geo.geo_trans_hash] || IDENTITY_16;

    // 3. 组合 world × geo
    const finalMat = new Matrix4().fromArray(worldMat);

    if (component.has_neg) {
        // 负实体特殊处理（如果需要）
    }

    const geoMatrix = new Matrix4().fromArray(geoMat);
    finalMat.multiply(geoMatrix);

    // 4. unit_mesh 额外缩放
    if (geo.unit_mesh && this.needsRuntimeScale) {
        // unit mesh 的旋转/缩放列需要额外乘以缩放因子
    }

    // 5. 运行时变换（残差旋转+缩放）
    if (this.runtimeTransform) {
        finalMat.premultiply(this.runtimeTransform);
    }

    return finalMat;
}
```

#### 3.2.3 清理

- `groupInstancesByGeoHash` 的 v3 分支遍历 `geos` 而非 `instances`
- 移除所有 `color_index`、`name_index`、`site_name_index` 相关代码（仅 v3 路径）
- refno 映射：直接读 `component.refno`，不再走 `inst.uniforms.refno`

## 4. 实施计划

### Phase 1：plant-model-gen v3 导出（最简模式）

- 新建 `ExportTransformConfig`
- 新建 `export_dbnum_instances_v3.rs`
- 配置：`inline_matrices=false`, `apply_rotation=false`, `target_unit=mm`
- CLI 入口注册
- 输出验证：对比 v2 和 v3 的 refno 覆盖率和几何体数量

### Phase 2：delivery-code v3 加载

- `AiosPrepackLoader` 新增 v3 检测和加载路径
- 实现 transforms 字典加载和矩阵组装
- 前端做全部转换（mm→m + Z-up→Y-up）
- 验证：与 v2 渲染结果对比

### Phase 3：配置参数验证

- 逐步开启 `target_unit=m`、`apply_rotation=true`
- 验证前端残差处理正确
- 性能对比：加载时间、内存占用

### Phase 4：生产切换

- 生产环境切换为 v3
- 可选清理 v2 专有代码
- 更新文档

## 5. 向后兼容

- `export_dbnum_instances_web.rs`（v2）保持不变，不删除
- `AiosPrepackLoader` 保持 v1/v2 加载路径
- v3 通过 `version === 3` 检测，不影响既有流程
- 渐进式切换，任何时候可回退到 v2

## 6. 风险与注意事项

1. **unit_mesh 标记的准确性**：需要在 geo_relate 查询时确定每个 geo_hash 是否为 unit mesh（当前逻辑在 `is_standard_unit_geometry()` 中判断小数字 + t_ 前缀）
2. **has_neg 的矩阵影响**：负实体组合矩阵的处理需要与 v2 的 `combine_world_geo_matrix` 保持一致
3. **TUBI 没有 geo_trans_hash**：TUBI 的变换是世界变换直接应用，没有几何级变换，v3 中 TUBI 只有 `trans_hash` 没有 `geos` 数组
4. **trans_hash 去重的边界条件**：同一个 trans_hash 对应的原始矩阵是唯一的，但如果 unit_flag 不同导致转换后的值不同，需要在字典层面统一处理（建议字典只做平移缩放，unit_mesh 缩放交给前端）
