# 动态精度：复用 Hash 与缓存失效方案

> ⚠️ 说明：本仓库已于 2026-01-27 移除“单位几何按实例缩放提升细分”的动态精度逻辑，回归静态 LOD 配置；本文作为历史记录保留。\
> 当前实现仍保留 `mesh_sig.json` 作为网格缓存失效/跳过重建机制。

## 背景

当前的“动态精度”已引入：对单位几何（inst_geo 复用的 unit mesh）按 `geo_relate.trans.d.scale` 的最大缩放做细分提升，并将细分上限配置为 512。

随之而来的问题是：**复用用的 Hash（inst_geo id / geo_hash）是否应纳入分段数（segments）？** 若纳入，则会导致同一几何被拆成大量变体，复用率骤降；若不纳入，则需要一套“缓存失效”机制，保证当缩放/配置变化时能自动重建需要的 mesh。

结论：**不把分段数纳入 geo_hash；改做签名（signature）驱动的缓存失效。**

## 强制重建（调试链路）

联调/排错时，常需“即便签名未变，也要强制重建”以验证代码变更（例如 sweep/loft 闭环拓扑、frame 逻辑）是否真正生效。

约定如下：

- CLI 使用 `--regen-model` 时，会设置环境变量 `FORCE_REGEN_MESH=1`
- mesh 生成阶段若检测到 `FORCE_REGEN_MESH=1`，将跳过 `mesh_sig` 的命中判断，直接重建并覆盖输出

## 补充：Sweep/Loft（如 WALL）弧线分段也受同一套参数控制

对于 `PrimLoft` / `SweepSolid` 一类（典型：WALL 沿圆弧路径放样/扫掠），路径圆弧的采样段数（log 中的 `segs=...`）并非固定常数，而是按：

- `segs = ceil(arc_len / target_segment_length)`（当 `target_segment_length` 配置存在时）
- 并被 `min_radial_segments .. max_radial_segments` 夹住
- 其中 `arc_len` 来源于真实弧长：`|angle| * |radius|`；
- 若该路径曾采用“单位化 + segment_transforms.scale 还原”的旧链路，则 `arc_len` 需把平面缩放计入；
- 当前策略：**带 CURVE 的 SPINE 不再单位化**，避免方位/变换链路过深导致建模复杂化；仅对简单直线路径考虑单位化复用。

因此若默认 LOD（例如 `default_lod=L2`）的 `max_radial_segments` 仍是 18/30/60 之类的小值，则大半径/大缩放圆弧会被强行限幅，表现为“折线化/粗糙”。  
解决方式不是把分段数塞进 `geo_hash`，而是：

- **配置层**：把目标 LOD 的 `csg_settings.max_radial_segments` 提升到 512（或更合适的上限）
- **缓存层**：由 `mesh_sig.json`（包含 `max_radial_segments`/`target_segment_length` 等）驱动自动失效与重建

## 重要：导出/截图必须跟随 default_lod，否则会读到旧产物

在 `DbOption-ams.toml` 一类配置中，`mesh_precision.default_lod` 往往不是 L1（例如 AMS 默认是 `L2`）。  
若导出/截图链路硬编码读取 `lod_L1`，就会出现：

- mesh/布尔结果实际写在 `lod_L2`（新）
- 导出/截图却读 `lod_L1`（旧）

表现为“明明修了建模/精度/缓存，但截图/OBJ 仍不对”。  
该问题的修复见：`开发文档/模型生成/07_RUS-134_WALL_导出截图LOD选择与布尔结果一致性修复.md`。

## Implementation Plan

1. **保持 geo_hash 稳定**
   - geo_hash/inst_geo id 继续表达“几何参数本体”，不包含细分参数。
   - 细分参数属于渲染/网格化策略，应与 LOD/配置绑定，而非几何本体身份。

2. **引入 Mesh Signature（签名）**
   - 为每个 `inst_geo + lod` 计算一个签名，包含：
     - 当前 LOD（如 L1/L2/L3）
     - `LodMeshSettings` 的关键字段（radial/height/cap/min/max/target_segment_length/error_tolerance/non_scalable_factor）
     - `non_scalable_geo`（会影响 adaptive 计算）
     - 动态精度的 `scale_factor`（圆柱取 max(x,y)，球取 max(x,y,z)；量化存储）
     - 版本号（signature schema version）
   - 对浮点字段做量化（例如乘以 1e6 取整），避免浮点抖动导致无谓重建。

3. **落盘保存签名文件（不改 DB schema）**
   - 生成 `lod_XX/{mesh_filename}.glb` 时，同步写入 `lod_XX/{mesh_filename}.mesh_sig.json`。
   - `mesh_filename` 采用现有命名（`{inst_geo_id}_{lod}`），确保与现有导出/加载逻辑兼容。

4. **生成前判定是否需要重建**
   - 若 `glb` 存在且 `mesh_sig.json` 与本次计算签名一致，则直接跳过 mesh 生成。
   - 若不一致（配置变更/最大缩放变更/算法版本变更），则重建并覆盖写入新签名。

5. **修正内存缓存键的一致性（顺手修复）**
   - `EXIST_MESH_GEO_HASHES` / `inst_aabb_map` 的 key 应使用 inst_geo id（不带 LOD 后缀），与 preload/后续查找一致。
   - 这可避免“生成了但缓存命中不到”的隐性问题。

## Task List

- [x] 在 `src/fast_model/mesh_generate.rs` 增加 `MeshSignatureV1` 与（读/写/比较）工具函数
- [x] 在 `gen_inst_meshes_by_geo_ids`：计算签名 -> 存在则跳过 -> 否则生成并写入签名
- [x] 在 `gen_inst_meshes`（并发批任务内）：同上逻辑
- [x] 修正 `handle_csg_mesh` 内对 `EXIST_MESH_GEO_HASHES` / `inst_aabb_map` 的 key 使用（改为 inst_geo id）
- [x] 本地验证：
  - 对同一 refno 连续执行两次 `--regen-model`：
    - 第一次生成并写入 `.mesh_sig.json`
    - 第二次应出现“跳过生成”的日志，且输出文件时间戳不再变化

## Thought

以“几何身份”与“网格策略”二分之：
- geo_hash 属几何身份，贵在稳定，宜复用；
- segments 属网格策略，贵在可变，宜用签名驱动失效，而不宜污染身份哈希。
