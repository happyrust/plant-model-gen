# 周边物项查询 API 使用说明

## 概述

周边物项查询功能允许用户通过 **refno** 或 **position** 坐标查询周边的物项，结果按距离排序并包含专业信息。

## API 端点

### GET /api/sqlite-spatial/query

查询周边物项。

#### 查询参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| mode | string | 否 | 查询模式：`refno`、`position`、`bbox`（默认自动推断） |
| refno | string | 条件 | refno 模式必填，格式：`dbnum_refno`（如 `17496_123456`） |
| x | number | 条件 | position 模式必填，中心点 X 坐标（毫米） |
| y | number | 条件 | position 模式必填，中心点 Y 坐标（毫米） |
| z | number | 条件 | position 模式必填，中心点 Z 坐标（毫米） |
| radius | number | 条件 | position 模式必填，查询半径（毫米） |
| distance | number | 否 | 额外扩张距离（毫米，默认 0） |
| max_results | number | 否 | 最大返回数量（默认 5000，上限 10000） |
| nouns | string | 否 | noun 类型过滤，逗号分隔（如 `PIPE,EQUI,TUBI`） |
| include_self | boolean | 否 | refno 模式是否包含自身（默认 true） |
| shape | string | 否 | 查询形状：`cube`（默认）或 `sphere` |

#### 响应格式

```json
{
  "success": true,
  "results": [
    {
      "refno": "17496_123456",
      "noun": "PIPE",
      "spec_value": 1,
      "distance": 1250.5,
      "aabb": {
        "min": { "x": 1000, "y": 2000, "z": 3000 },
        "max": { "x": 1500, "y": 2500, "z": 3500 }
      }
    }
  ],
  "truncated": false,
  "query_bbox": {
    "min": { "x": 5000, "y": 15000, "z": 0 },
    "max": { "x": 15000, "y": 25000, "z": 10000 }
  }
}
```

## 使用示例

### 1. 通过 refno 查询周边 5000mm 内的管道

```bash
curl "http://localhost:8080/api/sqlite-spatial/query?mode=refno&refno=17496_123456&distance=5000&nouns=PIPE"
```

### 2. 通过坐标查询周边 3000mm 内的所有物项

```bash
curl "http://localhost:8080/api/sqlite-spatial/query?mode=position&x=10000&y=20000&z=3000&radius=3000"
```

### 3. 球形查询（精确距离过滤）

```bash
curl "http://localhost:8080/api/sqlite-spatial/query?mode=position&x=10000&y=20000&z=3000&radius=5000&shape=sphere"
```

## 前端使用

### 使用 API 函数

```typescript
import { queryNearbyByPosition, querySpatialIndex } from '@/api/genModelSpatialApi';

// 方式 1：通过坐标查询
const result = await queryNearbyByPosition(10000, 20000, 3000, 5000, {
  nouns: 'PIPE,EQUI',
  max_results: 100
});

// 方式 2：通过 refno 查询
const result = await querySpatialIndex({
  mode: 'refno',
  refno: '17496_123456',
  distance: 5000,
  nouns: 'PIPE'
});
```

### 使用 Composable

```typescript
import { useNearbyQuery } from '@/composables/useNearbyQuery';

const { loading, error, specGroups, totalCount, query } = useNearbyQuery();

// 查询
await query({
  mode: 'position',
  x: 10000,
  y: 20000,
  z: 3000,
  radius: 5000,
  nouns: ['PIPE', 'EQUI']
});

// 结果按专业分组
console.log(specGroups.value);
// [
//   { spec_value: 1, spec_name: '工艺管道 (P)', count: 25, items: [...] },
//   { spec_value: 2, spec_name: '电气 (E)', count: 15, items: [...] }
// ]
```

## 注意事项

1. **索引文件**：需要先运行 `import-spatial-index` 构建空间索引
2. **距离单位**：所有距离参数和返回值均为毫米（mm）
3. **结果排序**：结果按距离从近到远自动排序
4. **性能优化**：使用 SQLite RTree 索引，查询性能优秀
5. **专业映射**：spec_value 对应的专业名称需在前端维护映射表
