# Contract: primitive_keypoints.parquet(后端实现视角)

> 与 `plant3d-web/specs/001-measurement-pick-sources/contracts/primitive-keypoints-parquet-contract.md`
> 为同一契约的两仓副本;字段冲突时以联合评审后的最新版为准,
> 任何变更必须双仓同步(spec 010 T007)。

## Purpose

为前端测量 Primitive Key Point 源提供几何模板级关键点数据。
数据按 `geo_hash` 去重(模板一份,实例共享),坐标为几何模板局部空间。

## File

`primitive_keypoints.parquet`,位于 dbnum 导出目录,与既有 6 表同级。
文件可选:缺失时前端报"源不可用",其余源不受影响。

## Manifest Entry

`manifest.json` 与 `manifest_{dbnum}.json` 同步声明
(web 版 file 带 `{subdir}/` 前缀):

```json
{
  "tables": {
    "primitive_keypoints": {
      "file": "primitive_keypoints.parquet",
      "rows": 12345,
      "key": ["geo_hash", "keypoint_index"]
    }
  },
  "primitive_keypoint_unit": {
    "source_unit": "mm",
    "target_unit": "mm",
    "conversion_factor": 1.0,
    "coordinate_space": "geo_local"
  },
  "primitive_keypoint_export": {
    "keypoint_geo_hashes": 100,
    "empty_keypoint_geo_hashes": 5,
    "truncated_keypoint_geo_hashes": 0
  }
}
```

`primitive_keypoint_unit` 缺失 = 契约不完整,前端不得启用该源。

## Schema

| Column | Type | Required | Description |
|--------|------|----------|-------------|
| `geo_hash` | Utf8 | yes | 几何模板标识,与 `geo_instances.parquet.geo_hash` 同源同算法 |
| `keypoint_index` | Int32 | yes | 模板内稳定序号(0 起,`enhanced_key_points` 返回序) |
| `kind` | Utf8 | yes | `Endpoint` / `Midpoint` / `Center` / `Intersection` / `SurfacePoint` |
| `local_x` | Float64 | yes | 几何模板局部 X(单位见 manifest) |
| `local_y` | Float64 | yes | 几何模板局部 Y |
| `local_z` | Float64 | yes | 几何模板局部 Z |
| `has_dir` | Boolean | yes | v1 恒 `false`(方向能力未导出) |
| `dir_x` | Float64 | yes | `has_dir=true` 时方向 X,否则 0 |
| `dir_y` | Float64 | yes | 同上 |
| `dir_z` | Float64 | yes | 同上 |
| `priority` | Int32 | no | 后端捕捉优先级建议(小=高);前端可忽略 |
| `source` | Utf8 | no | 固定 `geo_param.enhanced_key_points` |

## Producer Rules(后端)

1. 数据源:`PdmsGeoParam::enhanced_key_points(&Transform::IDENTITY)`,
   覆盖 14 种图元;`Unknown` / `CompoundShape` 返回空,不写行。
2. 局部坐标**不应用** geo_transform / instance transform / world transform
   ——前端按 Consumer Rules 组合。
3. 单 geo_hash 超过 256 点:按 priority 升序保留前 256 行,
   geo_hash 计入 `truncated_keypoint_geo_hashes`。
4. kind 词表本期锁定五类;新增值必须先双仓更新契约。

## Consumer Rules(前端,resolution 顺序)

1. refno/objectId → instance geometry rows → `geo_hash`;
2. 按 `geo_hash` 查本表;
3. local 坐标 × `primitive_keypoint_unit.conversion_factor`;
4. × geometry transform(geo_instances/transforms 表);
5. × instance/world transform;
6. × viewer global model matrix;
7. 产出 scene-space 候选,参与 display/snap。

未知 kind:按 `SurfacePoint` 渲染,不赋予高优先捕捉。

## Error Handling

- manifest 无 `primitive_keypoints` 条目 → 源不可用;
- 文件缺失 / 读取失败 → 源不可用;
- `primitive_keypoint_unit` 缺失 → 源不可用(不得猜默认);
- 某 geo_hash 无行 → 该几何无候选(正常,非错误);
- 变换缺失/非法 → 跳过该候选并记 source warning。
