# RUS-134 WALL：闭环接缝缺口修复与强制重建

## 现象

- WALL（PrimLoft）为两段 180° 圆弧拼接的闭环 SPINE，理论应生成完整圆环。
- 实际截图/OBJ 可见在闭环接缝处存在稳定“缺口/断口”（多出现在圆环起点附近）。

## 关键事实（数据库实据）

- 目标：`pe:\`25688_36116\`` noun=`WALL`
- `pe:\`25688_36116\`` children（保序）：`[pe:\`25688_36117\`, pe:\`25688_36120\`]`
- `pe:\`25688_36120\`` noun=`SPINE` children（保序）：`[POINSP, CURVE, POINSP, CURVE, POINSP]`
  - POINSP: `POS=(28000,0,0) -> (-28000,0,0) -> (28000,0,0)`
  - CURVE: `CPOS=(0,0,0)`，走半圆（THRU）
- 实例几何：`inst_geo:\`10328253156732916481\`` param=`PrimLoft`
  - `path.segments=[Arc(angle=PI,radius=28000), Arc(angle=PI,radius=28000)]`
  - `segment_transforms=[]`（非单位化路径）
  - `profile.SPRO.verts=[(0,0),(2150,0),(2150,3500),(0,3500)]`

## 已排除项

- LOD 目录读错：导出/截图已改为使用 `mesh_precision.default_lod` 对应目录。
- 布尔残留：replace 模式已清理 `inst_relate_bool/inst_relate_cata_bool`，boolean worker 日志 `neg_targets=0`。

## 根因候选（以事实为准）

### 1) mesh 未重建（最常见）

即使开启 replace 模式，`mesh_worker` 若发现 `*.glb` + `*.mesh_sig.json` 且签名一致，会跳过重建。
当修复落在 rs-core 的 sweep/loft 生成逻辑时，若 mesh 未重建，就会“看起来还是旧问题”。

### 2) 闭环 ring 拓扑未正确连接（或未生效）

闭环路径通常会在采样末尾附加一个与起点位置近似重合的 ring；若：
- 仅依赖“末尾重复点”表达闭合，但没有在拓扑上把最后一圈与第一圈环向连接；
则会在接缝处形成真实缺口。

## 修复策略

### A. 强制重建（用于验证）

实现：当 CLI 使用 `--regen-model` 时，设置环境变量 `FORCE_REGEN_MESH=1`，使 mesh 生成阶段跳过 `mesh_sig` 的缓存命中逻辑。

代码：
- `src/main.rs`：遇到 `--regen-model` 设置 `FORCE_REGEN_MESH=1`
- `src/fast_model/mesh_generate.rs`：若 `FORCE_REGEN_MESH=1`，则不执行 `mesh_sig skip`

### B. 触发缓存失效（用于自动化）

提升 `MeshSignatureV1::VERSION`（v5）以失效旧的 `*.mesh_sig.json`，保证闭环修复能被自动重建。

## 验证方法

1) 运行生成 + 截图（多视角）：

```bash
cargo run --bin aios-database -- --config DbOption-ams --debug-model 25688_36116 --regen-model --capture output/compare/wall_rebuild_v5 --capture-views 4 --capture-width 1200 --capture-height 900
```

2) 人工看图：圆环接缝处不应出现“断口”。

3) 若仍有缺口：
- 需继续在 rs-core 的 sweep_mesh 闭环 frame/拓扑处追加“闭环一致性修正”（例如闭环帧的 twist 分摊），再重复 1)。

