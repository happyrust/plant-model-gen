# RUS-134 / WALL：导出与截图的 LOD 选择修复（避免读到旧 L1）

## 问题现象

对 `WALL (pe:25688_36116)` 做 `--regen-model --capture` 后：

- boolean worker 的调试 OBJ（`test_output/debug_25688_36116_result.obj`）显示为完整圆环；
- 但 `capture` 生成的 OBJ/PNG 看起来像“半环/截面不对”。

进一步核对发现：

- `inst_relate_bool:25688_36116` 已是 `status=Success`，并写出了布尔结果 `mesh_id='25688_36116'`；
- 同一个 `mesh_id` 的 GLB 文件在不同 LOD 目录里**时间戳不一致**：
  - `assets/meshes/lod_L1/25688_36116_L1.glb`（较旧）
  - `assets/meshes/lod_L2/25688_36116_L2.glb`（较新，且更接近 debug obj）

根因是：导出/截图链路**硬编码读取 L1**，而当前配置 `DbOption-ams.toml` 的 `mesh_precision.default_lod=L2`，布尔结果（以及多数 mesh 生成）默认写在 L2。  
于是：截图/OBJ 很容易读到旧的 L1 文件，表现为“改了代码但看起来没变化/仍然不对”。

## 修复方案

统一导出/截图链路的默认 LOD 选择逻辑：

- 不再硬编码 `L1`
- 改为跟随 `aios_core::mesh_precision::active_precision().default_lod`

这样：

- `--gen-lod Lx` 能同时影响 mesh 生成与导出/截图读取；
- `DbOption.toml` 的 `mesh_precision.default_lod` 也能生效；
- 避免读取到历史遗留的 `lod_L1` 输出。

## 代码改动点

- `src/fast_model/export_model/export_obj.rs`
  - `prepare_obj_export()`：`effective_mesh_dir` 从 `lod_L1` 改为 `lod_{active_precision.default_lod}`
- `src/fast_model/export_model/export_common.rs`
  - GLB 存在性检查目录：从 `lod_L1` 改为 `lod_{active_precision.default_lod}`

## 验证步骤（建议）

1. 重新生成并截图（默认 L2）：

   ```bash
   cargo run --bin aios-database -- --config DbOption-ams --debug-model 25688_36116 --regen-model --capture output/compare/wall_lod_fix --capture-views 4
   ```

2. 如需强制以 L1 验证（或对比旧/新）：

   ```bash
   cargo run --bin aios-database -- --config DbOption-ams --gen-lod L1 --debug-model 25688_36116 --regen-model --capture output/compare/wall_lod_L1 --capture-views 4
   ```

3. 观察点：
   - `output/.../obj-cache/WALL_25688_36116.obj` 的 AABB 是否符合“圆环在 XY、厚度在 Z（再叠加 world_trans.translation）”的预期；
   - PNG 多视角是否仍出现“半环”。

## 备注：与缓存/Hash 的关系

本问题不是“分段数太低导致粗糙”，而是“读错 LOD 文件导致回退到旧产物”。  
因此即便已经实现 `mesh_sig.json`、上限 512、CURVE 不单位化，若导出/截图仍固定读 L1，依然会出现“看起来没修复”的假象。

