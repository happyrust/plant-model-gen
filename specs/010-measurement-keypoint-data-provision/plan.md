# Implementation Plan: 测量数据提供与前端捕捉闭环(spec 010)

## Approach

后端只补"最后一块数据":把已有的 `enhanced_key_points()` 能力沿
ptsets.parquet 的成熟模式导出为 `primitive_keypoints.parquet`,
manifest 同步声明表与单位元数据;前端按既有 spec 001 契约消费,
不引入新通道、新坐标系、新变换约定。

关键设计约束(与前端契约对齐):

- 局部坐标 + geo_hash 去重(模板级,不按实例复制);
- 单位元数据显式声明(缺失=契约不完整,前端拒绝启用该源);
- 表缺失=源不可用,前端其余源不受影响(可独立发布后端包)。

## Phase 0 — 现状基线与样本确认

1. 确认样本数据源:`quicktest-250160-8080` 站点同源的 dbnum=250160
   导出范围(spec 009 已实测 Parquet 包完备,808 实例)。
2. 跑一次现有导出,记录 6 表行数与 manifest 形态作为 before 基线:

   ```powershell
   cargo run --release --bin aios-database -- `
     --export-dbnum-instances-parquet --dbnum 250160 --verbose
   ```

3. 侦察 `enhanced_key_points()` 在本仓默认 feature 组合下的真实行为:
   写临时 debug 脚本对样本 geo_hash 调用,确认非空率与 kind 分布
   (区分"truck feature 未启用导致全空"与"图元本身无关键点")。
   若默认构建全空,本 spec 升级为先在 Cargo.toml 启用所需 feature
   并评估编译/体积代价,结论回填 spec Decisions。

## Phase 1 — PrimitiveKeypointRow 与导出实现

`src/fast_model/export_model/export_dbnum_instances_parquet.rs`:

1. 新增 `PrimitiveKeypointRow` 结构与 `primitive_keypoints_schema()`:

   | 列 | 类型 | 说明 |
   |----|------|------|
   | geo_hash | Utf8 | 与 geo_instances.parquet 同源 |
   | keypoint_index | Int32 | 模板内稳定序(enhanced_key_points 返回序) |
   | kind | Utf8 | Endpoint/Midpoint/Center/Intersection/SurfacePoint |
   | local_x/y/z | Float64 | 几何模板局部坐标(mm) |
   | has_dir | Boolean | v1 恒 false |
   | dir_x/y/z | Float64 | v1 恒 0 |
   | priority | Int32 | 捕捉优先级(enhanced_key_points 第三元) |
   | source | Utf8 | 固定 `geo_param.enhanced_key_points` |

2. 在既有 geo 去重循环(已维护 geo_hash → GeoInstance 映射处)挂接:
   每个新 geo_hash 调 `geo_param.enhanced_key_points(&Transform::IDENTITY)`,
   产出行集;空集计入 `empty_keypoint_geo_hashes`。
3. 行数防爆:单 geo_hash 关键点 > 256 时按 priority 升序(数值小=优先)
   截断到 256 并计数 `truncated_keypoint_geo_hashes`(进 manifest)。
4. `write_parquet` 落盘 + `total_bytes` 累加,与其余 6 表一致。

## Phase 2 — manifest 双写与单位元数据

1. `manifest.json` 与 `manifest_{dbnum}.json` 的 `tables` 同步加:

   ```json
   "primitive_keypoints": {
     "file": "primitive_keypoints.parquet",
     "rows": <n>,
     "key": ["geo_hash", "keypoint_index"]
   }
   ```

   web 版 file 带 `{subdir}/` 前缀(对齐现有 6 表写法)。
2. 顶层加 `primitive_keypoint_unit`:复用 `ptset_unit` 的
   LengthUnit/UnitConverter 管线,`coordinate_space: "geo_local"`。
3. 加 `primitive_keypoint_export` 统计块:
   `keypoint_geo_hashes / empty_keypoint_geo_hashes /
   truncated_keypoint_geo_hashes`。
4. `ParquetExportStats` 扩 `primitive_keypoint_count` 字段,
   verbose 输出打印。

## Phase 3 — CLI 验证与 ptset 一致性回归

1. **keypoint 抽查**(DuckDB CLI 或 debug_scripts):
   - 行数/kind 枚举校验:`SELECT kind, count(*) GROUP BY kind`,
     断言 kind ∈ 五类词表;
   - 局部 AABB 包含性:任取 3 个 geo_hash,join mesh 局部 AABB
     (或 `aabb.parquet` 对应几何),断言点在框内(容差 1mm)。
2. **ptset 双通道一致性**:
   - 启动样本站点 web_server,`GET /api/pdms/ptset/{refno}` 取点;
   - DuckDB 查 `ptsets.parquet` 按该 refno 的 cata_hash 取点;
   - 断言 point_number 集合一致、坐标差 ≤1e-3mm;
   - 脚本与输出归档 `debug_scripts/`,结论回填 spec。
3. `cargo check --workspace --all-targets` + 既有导出相关测试回归。

## Phase 4 — 前端联动验收(plant3d-web,跨仓)

前置:Phase 1-3 产物(样本包含 primitive_keypoints.parquet)就位。

1. 前端按 spec 001 tasks Phase 7 实施 T022-T025:
   manifest typing + DuckDB 查询(`useDbnoInstancesParquetLoader.ts`)、
   候选加载/单位换算/变换组合(`useMeasurementPickSources.ts`)、
   marker 渲染、unavailable 状态验证。
2. 前端按 Phase 8 实施 T026-T029(测量记录 source metadata 兼容)。
3. quickstart 场景(T012/T015/T032)用样本包走一遍,
   结果记录到前端 spec 001;本 spec 执行记录只留结论与链接。
4. 验收口径:spec 010 Acceptance Criteria #4。

## Phase 5 — 文档与收口

- `CHANGELOG.md` 记录新表与 manifest 变更;
- 前端契约文档若有字段增量(priority 列),同步更新
  `plant3d-web/specs/001-measurement-pick-sources/contracts/primitive-keypoints-parquet-contract.md`
  (additive,不破坏既有必需列);
- spec 010 回填执行记录(before/after 行数、验证命令、联动结论)。

## Risks

- R1:默认 feature 下 `enhanced_key_points` 大面积为空 →
  Phase 0 先侦察;若需开 truck feature,评估编译时间与产物体积,
  不可接受时降级为仅导出解析式图元(Cylinder/Box/Dish/Sphere/CTorus 等
  有手写 key_points 实现的族)并在 manifest 标注覆盖范围。
- R2:复杂图元(Polyhedron/Loft)关键点爆行 → 256 截断 + 统计标记;
  截断策略对测量捕捉影响有限(优先保留 Endpoint/Center)。
- R3:kind 词表后端新增值导致前端 unknown → 契约规定前端对未知 kind
  按 SurfacePoint 渲染且不参与高优先捕捉;本期词表锁定五类。
- R4:前端联动周期长 → Phase 1-3 可独立交付(包向后兼容,
  旧前端忽略新表),Phase 4 异步推进。

## Rollback

新表与 manifest 字段均为 additive:回滚即停止写新表,
旧 manifest 消费方(前端 6 表加载)不受影响;无 schema 迁移。

## Done Definition

- `primitive_keypoints.parquet` + manifest 双写落地,样本验证通过
  (AC #1-#3);
- 前端 Primitive Key Point 源用真实包数据走通 US3 场景(AC #4);
- 验证命令与执行记录可复跑(AC #5);
- CHANGELOG 与跨仓契约文档同步。
