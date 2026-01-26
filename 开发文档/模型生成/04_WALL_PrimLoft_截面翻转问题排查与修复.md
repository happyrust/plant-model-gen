# WALL（PrimLoft）截面翻转问题排查与修复

## 现象

- WALL 生成的扫掠体在路径中段出现“截面不对/疑似翻面/扭转”，两端截面法向表现异常（背面剔除下更明显）。
- 日志可见 `frame_check[last] tan·t≈-1`，即 `PathSample.tangent` 与 `rot.z_axis` 近似反向。

## 关键数据（SurrealDB 实据）

- 目标：`pe:\`25688_36116\``，noun=`WALL`
- `children`（保序）：`[pe:\`25688_36117\`, pe:\`25688_36120\`]`
- 实例几何：`inst_geo:\`6752136354495698267\``，`param=PrimLoft`
  - `profile.SPRO.verts = [(0,0),(2150,0),(2150,3500),(0,3500)]`
  - `path.segments = [Arc(angle=PI,radius=1), Arc(angle=PI,radius=1)]`
  - `segment_transforms.scale = [28000,28000,28000]`
  - `plax = [0,1,0]`

## 根因

PrimLoft/SweepSolid 的扫掠网格依赖 `sample_path_frames_sync()` 生成一串 `PathSample { tangent, rot }`。

原先 frame 推进逻辑在某些“切线累计旋转接近 180°”的情况下，会出现：

- `tangent` 已正确更新到新方向，但 `rot.z_axis` 沿用旧方向（或发生手性翻转）
- 于是同一条路径上，前半段 `tan·t≈+1`，后半段 `tan·t≈-1`
- 结果：截面环的绕序/法向在路径中段发生翻转，表现为截面不对、局部翻面/扭转

该 WALL 的路径由两段 PI 圆弧拼接，恰落入此类边界情形；日志中 `frame_tan_alignment` 的 `min_dot=-1` 即为佐证。

## 修复方案

### 1) frame 生成改为 Rotation-Minimizing Frame（RMF）

在 `rs-core/src/geometry/sweep_mesh.rs` 的 `sample_path_frames_sync()` 中：

- 以“最小旋转”方式更新坐标系：将上一帧的 `right` 投影到新切线 `t2` 的法平面，再重建 `up = t2 × right`
- 强制保证 `rot.z_axis == tangent`（同向），消除中段翻转

修复后日志应满足：

- `frame_tan_alignment: min_dot≈1, neg_cnt=0`
- `frame_check[first] tan·t≈1`
- `frame_check[last] tan·t≈1`

### 2) 缓存失效（避免旧 mesh 复用）

由于 sweep frame 算法更改会影响网格几何形状，需强制旧缓存失效：

- `gen_model-dev/src/fast_model/mesh_generate.rs`：`MeshSignatureV1::VERSION` 提升到 `3`

## 验证方法

命令（示例）：

```bash
cargo run --bin aios-database -- --config DbOption-ams --debug-model 25688_36116 --regen-model --capture output/compare/current_fix3 --capture-width 1200 --capture-height 900
```

关注：

- 日志：`[SweepSolid] frame_tan_alignment ... min_dot≈1`
- 截图：`output/compare/current_fix3/25688_36116.png`（截面应连续、无扭转、无中段翻面）

