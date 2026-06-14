# Tasks: 测量数据提供与前端捕捉闭环(spec 010)

> 后端任务(T001-T008)在本仓;前端任务(T009-T011)在 plant3d-web 仓,
> 对应其 spec 001 的 Phase 7/8/9,此处只追踪联动状态。

## T001 — 基线导出与 keypoint 能力侦察

- 跑现有 `--export-dbnum-instances-parquet --dbnum 250160 --verbose`,
  记录 6 表行数与 manifest 形态(before 基线)。
- debug 脚本对样本 geo_hash 调 `enhanced_key_points(IDENTITY)`,
  记录非空率、kind 分布、单 geo 最大点数;确认默认 feature 组合下
  是否依赖 truck(`#[cfg(feature = "truck")]` 分支)。

验收:

- before 基线与侦察数据写入本 spec 执行记录;
- 若默认构建关键点全空,先回填 Decisions 再继续(plan R1)。

## T002 — PrimitiveKeypointRow + schema + 导出循环

- `export_dbnum_instances_parquet.rs` 新增行结构、
  `primitive_keypoints_schema()`、`build_primitive_keypoints_batch()`;
- geo_hash 去重循环挂接 `enhanced_key_points(IDENTITY)`,
  keypoint_index 按返回序编号;
- 单 geo_hash >256 点按 priority 截断;
- 空集 / 截断分别计数。

验收:

- 样本导出产出 `primitive_keypoints.parquet`,rows>0;
- schema 与 `contracts/primitive-keypoints-parquet-contract.md` 一致。

## T003 — manifest 双写 + 单位元数据 + 统计

- `manifest.json` 与 `manifest_{dbnum}.json` 加
  `tables.primitive_keypoints`(web 版带 `{subdir}/` 前缀);
- 顶层加 `primitive_keypoint_unit`(LengthUnit 管线,
  `coordinate_space: "geo_local"`)与 `primitive_keypoint_export` 统计块;
- `ParquetExportStats` 扩 `primitive_keypoint_count`。

验收:

- 两份 manifest 均含新表与单位元数据,路径前缀正确;
- verbose 输出含 keypoint 统计行。

## T004 — keypoint CLI 抽查验证

- DuckDB/debug 脚本:kind 枚举分布断言(五类词表全覆盖校验);
- 任取 3 个 geo_hash,局部关键点 vs 该几何局部 AABB 包含性
  (容差 1mm);
- 脚本归档 `debug_scripts/`,命令写入执行记录。

验收:

- AC #2 通过;脚本可复跑。

## T005 — ptset 双通道一致性回归(只验不改)

- 样本站点 web_server 起服,`GET /api/pdms/ptset/{refno}` 与
  `ptsets.parquet`(经 refno→cata_hash join)双通道取点比对;
- point_number 集合一致、坐标差 ≤1e-3mm;
- 异常(如 API 走 cache 兜底导致 world_transform 差异)如实记录。

验收:

- AC #3 通过;结论作为前端 PTSET 源契约背书写入执行记录。

## T006 — 工作区回归

- `cargo check --workspace --all-targets`;
- 既有 parquet 导出相关单测/集成测试回归,基线行数不回退。

验收:

- exit code 0;已有 6 表行数与 before 基线一致。

## T007 — 跨仓契约文档同步

- `plant3d-web/specs/001-measurement-pick-sources/contracts/primitive-keypoints-parquet-contract.md`
  补 `priority` 列(additive、optional 语义)与 v1 `has_dir=false` 说明;
- 本仓 `contracts/primitive-keypoints-parquet-contract.md`(spec 010 副本)
  与前端版保持同步。

验收:

- 两仓契约文档无字段冲突;前端必需列未变更。

## T008 — CHANGELOG 与执行记录收口

- `CHANGELOG.md` 记录新表、manifest 字段、验证命令;
- spec 010 回填 before/after、验证输出、风险处置结论。

验收:

- 文档三处(spec 执行记录 / CHANGELOG / 契约)一致。

---

## T009 — [前端][联动] Primitive Key Point 源接入(plant3d-web T022-T025)

- manifest typing + DuckDB 查询(`useDbnoInstancesParquetLoader.ts`);
- 候选加载、单位换算、geometry→instance→global 变换组合
  (`useMeasurementPickSources.ts`);
- marker 渲染(`useMeasurementCandidateVisualizationThree.ts`);
- 包缺表/缺单位元数据时 unavailable 状态验证。

验收:

- 样本包驱动下 spec 001 US3 验收场景通过(本 spec AC #4)。

## T010 — [前端][联动] 测量记录 source metadata(plant3d-web T026-T029)

- 新测量点带 source 元数据,旧记录无迁移渲染;
- 格式化输出含来源标签。

验收:

- spec 001 US4 验收场景通过。

## T011 — [前端][联动] quickstart 全场景验证(plant3d-web T012/T015/T032)

- 默认 PTSET 行为、Mesh 兜底、多源确定性、四源全开综合场景;
- 结果记录到前端 spec 001,本 spec 执行记录留结论与链接。

验收:

- 前端 `npm run type-check` / `npm run lint` 通过;
  quickstart 场景全过。

---

## Dependencies

- T001 → T002 → T003 → T004/T005(可并行)→ T006 → T007/T008;
- T009 依赖 T002-T004 产物(样本包);T010 依赖 T009;T011 依赖 T009/T010;
- T005 与 T002-T004 无依赖,可提前并行。
