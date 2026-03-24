# 开发计划：delivery-code 兼容的 JSON 实例导出

## 0. 默认联调配置（当前项目）

联调 SurrealDB / TreeIndex / 输出目录时，统一使用：

```text
-c db_options/DbOption-zsy
```

即完整路径配置文件：`db_options/DbOption-zsy.toml`（`plant-model-gen` 仓库内）。  
示例命令均在此基础上追加 `--export-dbnum-instances-web`、`--dbnum`、`--root-model` 等。

## 1. 背景

delivery-code 前端（DTXPrepackLoader / AiosPrepackLoader）消费 prepack ZIP 包中的 `instances.json`，使用 **V2 格式**（扁平化实例 + 内联矩阵 + uniforms）。

当前 `plant-model-gen` 的 `export_dbnum_instances_json` 输出 V3/V4 格式（hash 引用 + 分离 trans/aabb），与 delivery-code **不兼容**。

本计划创建新模块 `export_dbnum_instances_web.rs`，参考 parquet 导出的直接 SurrealDB 查询方式，输出 delivery-code 可直接消费的 V2 格式。

## 2. 适配差异汇总

### 2.1 已部署格式（delivery-code 期望）vs 当前代码输出

| # | 差异点 | delivery-code 期望 | 当前代码输出 | 严重程度 |
|---|--------|-------------------|-------------|---------|
| D1 | 实例变换字段名 | `matrix` (16-float) | `geo_transform` | 致命 |
| D2 | 实例级 `uniforms` | `{refno, owner_refno, owner_noun}` | 缺失 | 致命 |
| D3 | 矩阵内容 | `world_trans × geo_trans` 组合 | 仅 `geo_trans` (局部) | 致命 |
| D4 | 顶层 `names` 数组 | 有 `[{kind, value}]` | 缺失 | 中 |
| D5 | 实例级 `name_index` | 每个 instance 有 | 缺失 | 中 |
| D6 | 实例级 `lod_mask` | 每个 instance 有 | 仅组件级 | 中 |
| D7 | tubing `uniforms` | 有 | 缺失 | 高 |

### 2.2 不需要适配的部分

- `manifest.json` — 两端格式完全一致
- `geometry_manifest.json` — 基本兼容
- 颜色信息 — **不需要导出**，前端通过 `model-display.config.json` + noun 类型决定

## 3. 单位转换与坐标系分析

| 处理 | 谁做 | 说明 |
|------|------|------|
| 单位转换 (mm→m) | 消费端 (DTXPrepackLoader) | 导出保持原始 mm，manifest 声明 `target_unit: "mm"` |
| 坐标系旋转 (Z-up→Y-up) | 消费端 | `RotationX(-90°)` 仅在前端渲染时应用 |
| 全局 180° Y 旋转 | 消费端 | Legacy 兼容，与数据无关 |
| 矩阵组合 (world × geo) | **生产端** | 每个 instance.matrix 是完整世界空间变换 |

结论：**新导出不做任何单位转换或旋转**，矩阵保持 SurrealDB 原始数据。

## 4. 数据流设计

```
SurrealDB
   │
   ├─ 1. inst_relate (WHERE dbnum=$dbnum)
   │     → refno, noun, name, owner_refno, owner_type, spec_value
   │     → 分组：BRAN/HANG → bran_groups, EQUI → equi_groups, 其他 → ungrouped
   │
   ├─ 2. query_insts_for_export(refnos, enable_holes=true)
   │     → refno → { world_trans_hash, insts: [{geo_hash, trans_hash, unit_flag}] }
   │
   ├─ 3. tubi_relate (范围扫描 BRAN/HANG owner)
   │     → leave_refno, index, geo_hash, world_trans_hash, spec_value
   │
   └─ 4. 批量查询 trans 表（所有收集到的 trans_hash）
         → { hash → DMat4 }
         
组装
   │
   ├─ 非 TUBI:  instance.matrix = world_trans × geo_trans
   │            (has_neg 时 geo_trans 已包含世界信息，不再乘 world_trans)
   ├─ TUBI:     instance.matrix = world_trans (tubi 的 trans 已是世界空间)
   └─ uniforms: { refno, owner_refno, owner_noun }
```

## 5. 输出 JSON Schema (V2)

```jsonc
{
  "version": 2,
  "generated_at": "2026-03-23T...",
  
  // names 表 — 用于 name_index 查找（前端 name↔refno 映射）
  "names": [
    { "kind": "site", "value": "UNKNOWN_SITE" },      // index 0
    { "kind": "bran", "value": "BRAN 1 OF ..." },      // index 1
    { "kind": "component", "value": "14207_6208" },     // index 2
    { "kind": "tubi", "value": "TUBI_14207_6207_1" }    // ...
  ],
  
  "bran_groups": [{
    "refno": "14207_6207",
    "noun": "BRAN",
    "name": "BRAN 1 OF ...",  // 可选
    "name_index": 1,
    "children": [{
      "refno": "14207_6208",
      "noun": "SCTN",
      "name": null,
      "name_index": 2,
      "instances": [{
        "geo_hash": "14591019619471361986",
        "geo_index": 0,
        "matrix": [/* 16 floats, 列主序 */],
        "name_index": 2,
        "site_name_index": 0,
        "lod_mask": 7,
        "uniforms": {
          "refno": "14207_6208",
          "owner_refno": "14207_6207",
          "owner_noun": "BRAN"
        }
      }]
    }],
    "tubings": [{
      "refno": "14207_6207_1",
      "noun": "TUBI",
      "name": "TUBI-14207_6207-0",
      "geo_hash": "...",
      "geo_index": 0,
      "matrix": [/* 16 floats */],
      "name_index": N,
      "site_name_index": 0,
      "order": 0,
      "lod_mask": 7,
      "spec_value": 150,
      "uniforms": {
        "refno": "14207_6207_1",
        "owner_refno": "14207_6207",
        "owner_noun": "BRAN"
      }
    }]
  }],
  
  "equi_groups": [/* 结构同 bran_groups，noun="EQUI" */],
  "ungrouped": [/* 结构同 children，无 tubings */]
}
```

## 6. 文件结构

### 6.1 新增文件

```
src/fast_model/export_model/export_dbnum_instances_web.rs
```

### 6.2 修改文件

```
src/fast_model/export_model/mod.rs        ← 添加 pub mod export_dbnum_instances_web
src/cli_modes.rs                          ← 添加 CLI 入口
```

### 6.3 复用现有代码

| 来源 | 内容 | 复用方式 |
|------|------|---------|
| `export_common.rs` | `InstRelateRow`, `query_inst_relate_batch`, `query_inst_relate_aabb_batch` | 直接调用 |
| `aios_core` | `ExportInstQuery`, `query_insts_for_export` | 直接调用 |
| `export_prepack_lod.rs` | `TubiQueryResult`, `TubiRecord`, `plant_transform_to_dmat4`, trans 查询逻辑 | 内联或抽取 |
| `simple_color_palette.rs` | 不需要（颜色不导出） | — |
| `gen_model/tree_index_manager.rs` | `TreeIndexManager`, `load_index_with_large_stack` | 直接调用 |

## 7. 核心函数签名

```rust
/// 导出 delivery-code 兼容的 V2 格式 instances.json
pub async fn export_dbnum_instances_web(
    dbnum: u32,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
    root_refno: Option<RefnoEnum>,
    mesh_base_dir: Option<PathBuf>,
) -> Result<WebExportStats>
```

### 7.1 内部函数

```rust
/// 批量解析 trans hash → DMat4（不做单位转换）
async fn resolve_trans_to_matrices(
    hashes: &HashSet<String>,
    verbose: bool,
) -> Result<HashMap<String, DMat4>>

/// 构建 names 表
fn build_name_table(
    owner_groups: &HashMap<RefnoEnum, OwnerGroup>,
    tubings_map: &HashMap<RefnoEnum, Vec<TubiInfo>>,
    ungrouped: &[UngroupedInfo],
) -> (Vec<Value>, HashMap<String, usize>)

/// 计算组合矩阵
fn combine_world_geo_matrix(
    world_mat: &DMat4,
    geo_mat: &DMat4,
    has_neg: bool,
) -> Vec<f32>

/// 计算 lod_mask
fn compute_lod_mask_from_disk(
    geo_hash: &str,
    mesh_base_dir: &Path,
) -> u32
```

## 7.1 仅导出单个 BRAN（加快联调）

与全库 `--dbnum` 不同，使用 **`--root-model`** / **`--debug-model`** 指定 BRAN（或任意根 refno），只收集该子树内可见实例的 `inst_relate`，并写出独立文件：

```bash
rustup run nightly cargo run --release -- \
  -c db_options/DbOption-zsy \
  --export-dbnum-instances-web \
  --dbnum 5101 \
  --root-model 24381/145018 \
  --use-surrealdb --verbose
```

- **`--dbnum`**：仍用于工程输出目录、TreeIndex 目录约定等；请与该项目 DESI 一致。
- **输出文件名**：`instances_web_root_24381_145018.json`（refno 中的 `/` 会替换为 `_`；适用于 BRAN / EQUI 等任意根）。
- **JSON 额外字段**：`export_root_refno`，便于确认子树根。

delivery-code 开发环境路由 **`#/test-web-bran`** 默认加载  
`/bundles/test/instances_web_root_21909_5016.json`（单设备 EQUI 示例），也可用：  
`?bran=21909_5016`、`?instances=...`、`?glb=...`。

## 8. 实现步骤

### Step 1: 创建模块骨架
- 创建 `export_dbnum_instances_web.rs`
- 在 `mod.rs` 注册模块
- 定义数据结构和函数签名

### Step 2: 实现查询层
- 复用 `query_inst_relate_batch` 查询 inst_relate
- 复用 `query_insts_for_export` 获取几何实例 hash
- 实现 tubi_relate 查询（从现有代码抽取）
- 实现 `resolve_trans_to_matrices`（从 `query_trans_by_hashes` 简化）

### Step 3: 实现组装层
- 按 owner_type 分组（BRAN/HANG → bran_groups, EQUI → equi_groups）
- 构建 names 表
- 解析 trans hash → 矩阵
- 组合 world × geo 矩阵
- 构建 uniforms
- 组装完整 V2 JSON

### Step 4: 实现输出层
- 序列化为 pretty JSON
- 写入文件
- 返回统计信息

### Step 5: CLI 集成
- 在 `cli_modes.rs` 添加 `--export-dbnum-instances-web` 模式
- 连接参数解析

### Step 6: 验证
- 导出测试数据
- 与已部署 `instances.json` 结构对比
- 前端加载测试

## 9. 风险与注意事项

1. **has_neg 处理**：布尔运算结果的 geo_transform 已包含世界坐标，此时 `matrix = geo_trans`（不乘 world_trans）
2. **TUBI 矩阵**：tubi_relate 的 `world_trans` 已经是世界空间变换，直接使用
3. **trans 表数据格式**：`{ translation: [x,y,z], rotation: [x,y,z,w], scale: [x,y,z] }`，需用 `DMat4::from_scale_rotation_translation` 重建
4. **lod_mask**：若无法访问 mesh 目录，默认设为 7（全部 LOD 可用）
5. **名称查找**：inst_relate 可能不携带 name，需要容错处理
