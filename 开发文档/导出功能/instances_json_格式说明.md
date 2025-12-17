# Instances.json 导出格式说明

## 概述

`instances.json` 是 Prepack LOD 导出格式的核心文件，用于存储模型实例的层级结构、变换矩阵和渲染属性。该文件配合 GLB 几何体文件使用，支持前端高效渲染大规模工厂模型。

## 文件结构

```json
{
  "version": 2,
  "generated_at": "2025-12-17T03:19:33.881Z",
  "colors": [...],
  "bran_groups": [...],
  "equi_groups": [...],
  "ungrouped": [...]
}
```

---

## 顶层字段说明

| 字段 | 类型 | 说明 |
|------|------|------|
| `version` | `number` | 格式版本号，当前为 2 |
| `generated_at` | `string` | 生成时间（ISO 8601 格式） |
| `colors` | `array` | 颜色调色板，RGBA 格式 |
| `bran_groups` | `array` | 按 BRAN/HANG 分组的构件 |
| `equi_groups` | `array` | 按 EQUI 分组的构件 |
| `ungrouped` | `array` | 未分组的构件 |

---

## 分组结构 (bran_groups / equi_groups)

### BRAN 分组示例

```json
{
  "refno": "24381_46951",
  "noun": "BRAN",
  "name": "-CAM-S-1-M-5201",
  "children": [...],
  "tubings": [...]
}
```

### 字段说明

| 字段 | 类型 | 说明 |
|------|------|------|
| `refno` | `string` | BRAN/EQUI 的唯一标识符 |
| `noun` | `string` | 节点类型（BRAN/HANG/EQUI） |
| `name` | `string\|null` | 节点的 default full name |
| `children` | `array` | 子构件列表 |
| `tubings` | `array` | 管道实例列表（仅 BRAN 分组有） |

---

## 子构件结构 (children)

### 示例

```json
{
  "refno": "24381_46955",
  "noun": "TRNS",
  "name": "TRNS 1 OF BRAN /-CAM-S-1-M-5201",
  "color_index": 0,
  "lod_mask": 1,
  "spec_value": 4,
  "refno_transform": [...],
  "instances": [...]
}
```

### 字段说明

| 字段 | 类型 | 说明 |
|------|------|------|
| `refno` | `string` | 构件的唯一标识符（格式：`dbno_id`） |
| `noun` | `string` | 构件类型（如 TRNS, VALV, FITT 等） |
| `name` | `string\|null` | 构件的 default full name |
| `color_index` | `number` | 在 `colors` 数组中的索引 |
| `lod_mask` | `number` | LOD 级别掩码（位标志） |
| `spec_value` | `number\|null` | 规格值（来自 ZONE 的 spec_value） |
| `refno_transform` | `array[16]` | 构件的世界变换矩阵（列优先 4x4） |
| `instances` | `array` | 几何体实例列表 |

---

## 几何体实例结构 (instances)

### 示例

```json
{
  "geo_hash": "14092856918922709467",
  "geo_index": 1,
  "geo_transform": [1.0, 0.0, 0.0, 0.0, ...]
}
```

### 字段说明

| 字段 | 类型 | 说明 |
|------|------|------|
| `geo_hash` | `string` | 几何体的唯一哈希值 |
| `geo_index` | `number` | 在 GLB 文件中的 mesh 索引 |
| `geo_transform` | `array[16]` | 几何体相对于 refno 的局部变换矩阵 |

---

## 变换矩阵说明

### 双层变换架构

```
最终世界坐标 = refno_transform × geo_transform × 顶点坐标
```

- **refno_transform**：构件在世界坐标系中的位置和朝向
- **geo_transform**：几何体相对于构件原点的局部变换

### 矩阵格式

16 元素数组，列优先存储（Column-major order）：

```
[m00, m10, m20, m30,  // 第1列
 m01, m11, m21, m31,  // 第2列
 m02, m12, m22, m32,  // 第3列
 m03, m13, m23, m33]  // 第4列（平移分量）
```

---

## 管道实例结构 (tubings)

### 示例

```json
{
  "refno": "24381_46960",
  "noun": "TUBI",
  "name": "...",
  "geo_hash": "12345678901234567890",
  "geo_index": 5,
  "matrix": [...],
  "color_index": 1,
  "order": 0,
  "lod_mask": 1,
  "spec_value": 4
}
```

### 字段说明

| 字段 | 类型 | 说明 |
|------|------|------|
| `refno` | `string` | TUBI 的唯一标识符 |
| `noun` | `string` | 固定为 "TUBI" |
| `name` | `string\|null` | TUBI 的 default full name |
| `geo_hash` | `string` | 几何体哈希值 |
| `geo_index` | `number` | GLB 中的 mesh 索引 |
| `matrix` | `array[16]` | 世界变换矩阵 |
| `color_index` | `number` | 颜色索引 |
| `order` | `number` | TUBI 在 BRAN 中的顺序 |
| `lod_mask` | `number` | LOD 级别掩码 |
| `spec_value` | `number\|null` | 规格值 |

---

## 颜色调色板 (colors)

### 格式

```json
"colors": [
  [0.752, 0.752, 0.752, 1.0],  // 索引 0
  [0.0, 0.5, 1.0, 1.0],        // 索引 1
  ...
]
```

每个颜色为 RGBA 数组，值范围 0.0-1.0。

---

## LOD 掩码 (lod_mask)

位标志表示该构件在哪些 LOD 级别可用：

| 位 | LOD 级别 | 说明 |
|----|----------|------|
| 0 | L1 | 高精度 |
| 1 | L2 | 中精度 |
| 2 | L3 | 低精度 |
| ... | ... | ... |

示例：`lod_mask = 3` 表示在 L1 和 L2 级别都可用。

---

## 前端渲染使用示例

```javascript
// 加载 instances.json
const data = await fetch('instances.json').then(r => r.json());

// 遍历 BRAN 分组
for (const bran of data.bran_groups) {
  for (const child of bran.children) {
    const refnoMatrix = new Matrix4().fromArray(child.refno_transform);
    
    for (const inst of child.instances) {
      const geoMatrix = new Matrix4().fromArray(inst.geo_transform);
      const worldMatrix = refnoMatrix.clone().multiply(geoMatrix);
      
      // 渲染 mesh
      const mesh = glbMeshes[inst.geo_index];
      mesh.matrix.copy(worldMatrix);
      scene.add(mesh.clone());
    }
  }
}
```

---

## 相关文件

| 文件 | 说明 |
|------|------|
| `lod_L1.glb` | LOD 1 级别的几何体合集 |
| `lod_L2.glb` | LOD 2 级别的几何体合集 |
| `manifest.json` | 导出统计信息 |
| `geometry_manifest.json` | 几何体索引映射 |

