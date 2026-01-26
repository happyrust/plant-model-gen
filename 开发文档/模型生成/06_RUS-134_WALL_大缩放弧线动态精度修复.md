# RUS-134：WALL 弧线路径动态精度与曲线路径非单位化修复

## 背景

在 AvevaMarineSample 中，`WALL:25688_36116` 的几何落到 `inst_geo` 后，其 `param` 为 `PrimLoft`：

- 路径由两段半圆弧组成：`angle=PI` + `angle=PI`，组合应为完整圆环
- 圆弧使用真实半径：`radius=28000`（不再采用 `radius=1 + segment_transforms.scale=28000` 的单位化套路）
- 截面（SPRO）为真实尺寸：`(2150, 3500)`

该结构要求“路径圆弧采样段数”按 **真实弧长** 自适应，否则会明显折线化；并且过粗的网格会进一步影响后续布尔运算（Manifold 导入/求交稳定性）。

## 问题现象

修复前：弧长/段长的理论段数远大于默认上限，但被 `max_radial_segments` 限幅，导致精度不足（折线化）。

## 原因分析（第一性原理）

路径圆弧采样段数由 `compute_arc_segments(settings, arc_len, radius)` 计算：

- 若 `settings.target_segment_length` 存在：`segs = ceil(arc_len / target_segment_length)`
- 然后 **夹在** `[min_radial_segments, max_radial_segments]` 区间内（并做硬上限保护）
- 其中 `arc_len = |angle| * |radius|`（曲线路径不单位化后，通常不再依赖 `segment_transforms.scale`）

因此当默认 LOD 为 `L2` 且 `L2.csg_settings.max_radial_segments` 很小时，即便真实弧长很大，最终仍会被压到很小的段数。

## 修复方案

### Implementation Plan

1. 将当前使用的配置文件（`DbOption-ams.toml`）里，默认 LOD（L2）以及其他 LOD 的 `csg_settings.max_radial_segments` 提升到 512
2. 对带 `CURVE` 的 `SPINE`：禁用单位化（直接存真实圆弧几何），只对“简单直线 PrimLoft”启用单位化复用
3. 重新生成 `WALL:25688_36116` 模型，确认日志中弧线 `segs` 被提升并受 512 上限约束，且路径闭合

### Task List

- [x] 修改 `gen_model-dev/DbOption-ams.toml`：
  - [x] `L1.csg_settings.max_radial_segments = 512`
  - [x] `L2.csg_settings.max_radial_segments = 512`（本次关键：默认 LOD 为 L2）
  - [x] `L3.csg_settings.max_radial_segments = 512`
  - [x] `L4.csg_settings.max_radial_segments = 512`
- [x] 运行复现/验证命令（见下）
- [x] 用日志与数据库数据交叉验证（`inst_geo.param` 与 sweep 输出一致）
- [x] 曲线路径非单位化：`inst_geo.param.PrimLoft.path.segments[].Arc.radius=28000` 且 `segment_transforms=[]`

## 验证记录

### 复现/验证命令

```bash
cargo run --bin aios-database -- --config DbOption-ams --debug-model 25688_36116 --regen-model --export-obj
```

### 关键日志对比

- 修复后日志出现：
  - `[SweepSolid] ... segs=512`（受 512 上限约束）
  - `rings=1025`
  - `path_closed=true`

### 数据库核对（SurrealQL）

```sql
SELECT id, param.PrimLoft.path.segments, param.PrimLoft.segment_transforms
FROM inst_geo:`10328253156732916481`;
```

可见两段 `Arc.angle=PI`、`Arc.radius=28000` 且 `segment_transforms=[]`，与 sweep 日志相符。

## Thought

精度之事，非“半径小/大”本身所致，而在“采样受小上限约束”所致。  
曲线路径若单位化，方位/变换链路愈繁，排错愈难；故宁取“曲线用真实几何”，直线再行单位化复用。  
上限宜可配且足够大（512），而缓存失效宜由 mesh signature 驱动，毋令分段数污染 geo_hash，方可兼顾复用与质量。
