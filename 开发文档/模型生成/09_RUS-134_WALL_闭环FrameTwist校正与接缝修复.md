# RUS-134 WALL：闭环 Frame Twist 校正与接缝修复

## 背景与现象

WALL（PrimLoft）沿闭环 SPINE（两段 180° 圆弧）生成圆环实体。现阶段已做到：

- CURVE SPINE 不单位化（保留真实半径/弧长），避免截面/方位链路错乱
- LOD 导出/截图跟随 `mesh_precision.default_lod`
- 动态精度上限 `max_radial_segments=512`，圆弧采样更细
- replace 模式清理 booled 关系，避免旧布尔结果混入

但截图中闭环接缝处仍可能出现稳定“接缝/缺口观感”（多数情况下更像 shading seam，而非真实拓扑断裂）。

## 数据与日志要点（事实）

在 debug-model 日志中可见：

- `path_closed=true start_end_dist≈0.009mm`（闭环判定成立）
- `frame_tan_alignment: min_dot≈1 neg_cnt=0`（切线与 rot.z 对齐，避免中段翻转）

因此“中段翻面/截面扭转”已被 RMF 修复覆盖；剩余接缝更可能来自：

## 根因假设（需验证）

### 闭环的 Holonomy / Twist（首尾帧绕切线存在净旋转）

Rotation-Minimizing Frame（平行传输）在闭合曲线下可能产生净 twist（数学上称 holonomy）。

- 虽然位置闭合、切线闭合，但 `first_frame.right` 与 `last_frame.right` 可能存在绕切线的夹角
- 侧面环向连接时，若首尾帧截面朝向不一致，会形成：
  - 光照/法线不连续的“shading seam”（视觉上像“缺口/断口”）
  - 在某些剔除/法线敏感的 viewer 中更明显

## 实施方案（KISS）

在 `sample_path_frames_sync()` 生成 frames 后，若判定为闭环：

1) 计算首尾帧在“切线轴”上的相对旋转角 `delta`（以 right 轴为基准）
2) 将 `delta` 以线性比例分摊到每个采样点：第 i 帧额外旋转 `-delta * (i/(n-1))`
3) 使最后一帧与第一帧的截面朝向对齐，从而消除接缝处的法线/光照不连续

约束：

- 仅在 `path_closed=true` 且 `t0·tN > 0.99` 时启用（避免误修）
- `|delta|` 很小时跳过（避免无谓扰动）

## Task List

- [ ] 在 `rs-core/src/geometry/sweep_mesh.rs` 的 `sample_path_frames_sync()` 添加闭环 twist 校正
- [ ] debug-model 下打印 `closed_twist_deg`，便于确认是否存在显著 twist
- [ ] 用 `--regen-model --capture-views 4` 重新生成截图，人工确认接缝是否消失

