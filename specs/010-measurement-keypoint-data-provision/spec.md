# Feature Specification: 测量数据提供与前端捕捉闭环(spec 010)

## User Need

打通 plant3d-web 测量功能的**全部点源数据链路**:后台(本仓)负责提供
ptset 关键点、实例位置、图元关键点(primitive key points)等测量点源数据,
前端(`D:\work\plant-code\plant3d-web`)负责测量交互与多源捕捉(snap)。

前端 spec `plant3d-web/specs/001-measurement-pick-sources` 已定义四类测量点源
(mesh_pick_point / ptset / position / primitive_key_point),其前端 MVP
(resolver、复选框矩阵、PTSET/Mesh/Position 三源)已落地;当前阻塞点是
**后端没有导出 primitive key points 数据**,导致前端第四源(primitive_key_point)
与测量记录溯源(US4)无法继续。

## Current State(已探明,2026-06-13)

### 后台(本仓)已有能力

- **ptset HTTP API**(`src/web_api/ptset_api.rs`):
  `GET /api/pdms/ptset/{refno}`、`POST /api/pdms/ptset/batch-query`;
  SurrealDB `inst_relate->inst_info` 真源优先,`output/instance_cache`
  per-refno 兜底;响应含 `world_transform`(4x4 列主序)、`batch_id`、
  `unit_info`(mm→mm, factor=1.0)。
- **Parquet 导出包**(`src/fast_model/export_model/export_dbnum_instances_parquet.rs`):
  `instances / ptsets / geo_instances / tubings / transforms / aabb` 6 表
  + `manifest.json` + web 兼容 `manifest_{dbnum}.json`;
  `ptsets.parquet` 按 `cata_hash` 复用局部坐标关键点
  (point_number/pt/dir/ref_dir/pbore/pwidth/pheight/pconnect),
  manifest 带 `ptset_unit`(source/target/conversion_factor/coordinate_space)。
- **图元关键点能力**(依赖仓 rs-core-mbd-trait,已就绪未导出):
  - `PdmsGeoParam::key_points() -> Vec<RsVec3>`:14 种图元
    (Box/LSnout/Dish/Sphere/CTorus/RTorus/Pyramid/LPyramid/SCylinder/
    LCylinder/Revolution/Extrusion/Polyhedron/Loft)的几何局部关键点;
  - `PdmsGeoParam::enhanced_key_points(&Transform) -> Vec<(Vec3, String, u8)>`:
    带语义类型(Endpoint/Midpoint/Center/Intersection/SurfacePoint)
    与捕捉优先级(u8)的增强关键点;
  - `GeoInstance::key_points()`:局部点经 `geo_transform` 变换。
  - 生成管线已在 `pdms_inst.rs` 用 `key_points()` 算 AABB 与 vec3_pool,
    数据通路成熟。

### 前端(plant3d-web)已有能力与缺口

spec `001-measurement-pick-sources` 的 tasks.md 状态:

- 已完成(Phase 1-5):`useMeasurementPickSources.ts` 统一 resolver
  (优先级排序/屏幕距离/稳定 tie-break)、MeasurementPanel 显示+捕捉
  复选框矩阵、PTSET 源迁移、Mesh Pick Point 源、Position 源。
- 未完成:
  - **Phase 6(T019-T021)= 本仓工作**:导出 `primitive_keypoints.parquet`
    + manifest 条目 + 单位元数据;
  - Phase 7(T022-T025):前端 Primitive Key Point 源接入(依赖 Phase 6);
  - Phase 8(T026-T029):测量记录 source metadata 与旧记录兼容;
  - 验证项 T012/T015/T031/T032。

## Gaps(挡住测量闭环的三件事)

1. **图元关键点数据不出库**:`key_points()` / `enhanced_key_points()` 只在
   生成管线内部使用,Parquet 包与 HTTP API 都拿不到,前端第四源无数据。
2. **契约双方未锁定**:前端契约
   (`plant3d-web/specs/001-measurement-pick-sources/contracts/primitive-keypoints-parquet-contract.md`)
   要求 `geo_hash + keypoint_index + kind + local 坐标 + 单位元数据`,
   后端尚无对应 schema 实现与验证命令;`kind`/`priority` 语义映射未定。
3. **前端收尾任务无后端依据**:Phase 7/8 必须基于真实包数据验证
   (unavailable 状态、单位换算、变换组合),当前无样本包可验。

## Decisions

| 问题 | 决策 |
|------|------|
| Q1 关键点数据通道 | A:**Parquet 包优先**(与 ptsets.parquet 同模式,按 geo_hash 去重,离线可用、体积可控);HTTP API 不在本期范围,留作 Backlog(前端 resolver 已按包数据设计) |
| Q2 keypoint 坐标系 | A:**几何模板局部坐标**(geo-local,未应用 geo_transform),由前端按契约组合 geometry transform → instance/world transform → viewer global matrix;与 ptsets.parquet 的 local 模式一致 |
| Q3 kind 与优先级来源 | A:用 `enhanced_key_points(IDENTITY)` 取 (点, kind, priority);kind 直接落列(Endpoint/Midpoint/Center/Intersection/SurfacePoint),priority 作为扩展列 `priority`(Int32)导出,前端可用可不用 |
| Q4 方向向量 | A:v1 全部 `has_dir=false`(现有 key_points 能力不含方向);列仍按契约保留,后续图元方向能力就绪后填充 |
| Q5 单位 | A:几何局部坐标为 mm;manifest 写 `primitive_keypoint_unit { source_unit, target_unit, conversion_factor }`,与现有 `ptset_unit` 同结构同换算管线 |
| Q6 去重粒度 | A:按 `geo_hash`(几何模板)一份,不按 world instance 复制;实例多份共享同模板关键点(契约 Backend Note 已要求) |
| Q7 前端任务归属 | A:Phase 7/8 任务仍记在前端 spec 001 的 tasks.md;本 spec 把它们列为跨仓联动验收项,不重复展开实现细节 |

## Scope

1. **`export_dbnum_instances_parquet.rs`:新增 `primitive_keypoints.parquet` 导出**
   - 遍历导出范围内 `GeoInstance`,按 `geo_hash` 去重;
   - 每 geo_hash 调 `geo_param.enhanced_key_points(IDENTITY)` 得
     (local_pos, kind, priority),按稳定顺序编 `keypoint_index`;
   - schema 见 `contracts/primitive-keypoints-parquet-contract.md`
     (geo_hash / keypoint_index / kind / local_x/y/z / has_dir /
     dir_x/y/z / priority / source);
   - 空关键点的 geo_hash 不写行;统计 `keypoint_geo_hashes` /
     `empty_keypoint_geo_hashes` 进 manifest。
2. **manifest 双写对齐**
   - `manifest.json` 与 web 兼容 `manifest_{dbnum}.json` 同步新增
     `tables.primitive_keypoints`(file/rows/key)与
     `primitive_keypoint_unit`(source_unit/target_unit/conversion_factor),
     web 版文件路径带 `{subdir}/` 前缀(与现有 6 表一致)。
3. **CLI 验证命令**
   - 复用本仓 CLI/JSON 验证工作流:导出后用 DuckDB/CLI 抽查
     `primitive_keypoints.parquet` 行数、kind 分布、样例 geo_hash 的
     local 坐标与 mesh AABB 的包含关系(局部点应落在该几何局部 AABB 内,
     容差 1mm)。
4. **ptset 数据提供现状固化(只验证不改造)**
   - 以 quicktest 站点样本回归 `GET /api/pdms/ptset/{refno}` 与
     `ptsets.parquet` 数据一致性(同 refno → cata_hash → 点数与坐标一致);
   - 结论写入本 spec 执行记录,作为前端 PTSET 源的数据契约背书。
5. **跨仓联动验收(前端,plant3d-web spec 001 Phase 7/8)**
   - 用本仓导出的样本包驱动前端 T022-T025(Primitive Key Point 源)、
     T026-T029(测量记录 source metadata)、T012/T015/T032(quickstart 验证);
   - 前端改动在 plant3d-web 仓库提交,本 spec 只追踪联动验收结论。

## Non-Goals

- 不新增 keypoint HTTP API(Parquet 包通道足够;API 列入 Backlog)。
- 不为 TUBI 隐式管段生成 keypoint(tubings.parquet 已含端点坐标,
  前端 position/ptset 源已覆盖该场景)。
- 不做关键点方向向量推导(v1 `has_dir=false`)。
- 不改 `key_points()` / `enhanced_key_points()` 的几何算法本身;
  发现的图元关键点缺陷记录后另开 spec。
- 不做测量记录的后端持久化(前端 viewer store 既有机制不变)。

## Known Constraints / Risks

- `enhanced_key_points()` 含 `#[cfg(feature = "truck")]` 分支
  (brep shell 顶点),默认 feature 下部分图元可能返回空——导出统计需
  区分"图元无关键点"与"feature 未启用",验证时确认本仓构建 feature 组合。
- Polyhedron/Loft/Extrusion 等复杂图元关键点数可能很大,需统计 P99 行数,
  必要时按 kind 截断(优先 Endpoint/Center)并在 manifest 标记截断策略。
- geo_hash 与前端 `geo_instances.parquet` 的 `geo_hash` 必须同源同算法
  (现有导出已保证;新表只引用不重算)。
- 前端 viewer global matrix 与 dbnum 包变换的组合顺序以前端契约第 5-7 步
  为准,后端不做任何预变换。

## Acceptance Criteria

1. 对样本 dbnum(250160,quicktest 站点同源数据)导出:
   `primitive_keypoints.parquet` 产出,行 schema 与契约一致,
   manifest 双文件均含 `tables.primitive_keypoints` 与
   `primitive_keypoint_unit`,`rows > 0`。
2. CLI 抽查:任取 3 个 geo_hash,其全部 local 关键点落在该几何
   局部 AABB 内(容差 1mm);kind 枚举值 100% 落在五类词表内。
3. ptset 一致性回归:同 refno 经 HTTP API 与 ptsets.parquet 两通道
   取点,point_number 集合一致、坐标差 ≤1e-3mm。
4. 前端联动:plant3d-web 加载样本包后,Primitive Key Point 源
   display/snap 可用,US3 验收场景(spec 001)通过;包缺失该表时
   前端报"源不可用"且其余源不受影响。
5. 全链路可复跑:导出命令、验证命令、样本路径写入 spec 执行记录;
   `cargo check --workspace --all-targets` 通过。
