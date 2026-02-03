# TUBI arrive/leave 端点不一致（cache-only 导出）排查 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 找出“cache-only 模式下导出的 tubi 未按 ARRIVE→LEAVE 端点连线”的根因，并给出可验证的最小修复方向。

**Architecture:** 以证据驱动：先复现并抽取“期望端点/实际端点”，再定位差异来自（1）unit cylinder 约定、（2）tubi 生成阶段 transform 计算分支（get_transform vs build_tubi_transform_from_segment）、（3）cache 中 tubi 记录缺字段（tubi_info_id/ptset_map）导致无法按 arrive/leave 还原。

**Tech Stack:** Rust（aios-database）、foyer instance_cache、OBJ 导出、辅助 example/scripts。

---

## 背景事实（已确认）

1. OBJ 导出默认走 **cache-only**：若未显式传 `--use-surrealdb`，`src/main.rs` 会强制 `db_option_ext.use_surrealdb=false`，导出期 instances 从 `output/instance_cache` 读取。
2. unit_cylinder_mesh 约定：**z ∈ [0..1]**（见 `examples/tmp_boolean_sanity.rs` 注释）；`cata_model.rs` 中 tubi aabb 也按 `z=[0..1]`（`unit_cyli_aabb = [-0.5,-0.5,0]..[0.5,0.5,1]`）。
3. tubi transform 计算有两路：`build_tubi_transform_from_segment(start,end)` 与 `current_tubing.get_transform()`（由 `dir_ok` 控制）。

---

## Task List

### Task 1：复现与对照（cache-only vs SurrealDB-only）

**Step 1.1：cache-only 导出（现状）**

Run:
```powershell
cargo run --bin aios-database -- --debug-model 24381/103385 --regen-model --export-obj
```

Expected:
- 控制台出现 “📦 cache-only：OBJ 实例数据从 foyer cache 读取…”
- 生成 `output/24381_103385.obj`（或按输出参数指定路径）

**Step 1.2：SurrealDB-only 对照导出（用于判断问题是否仅 cache 路径）**

Run:
```powershell
cargo run --bin aios-database -- --debug-model 24381/103385 --regen-model --export-obj --use-surrealdb
```

Expected:
- 不再出现 cache-only 提示（或提示 SurrealDB-only）
- 生成另一份 OBJ（建议用 `--export-obj-output` 指定不同文件名）

**Step 1.3：比对 OBJ 内的 TUBI 组**

Run:
```powershell
Select-String -Path output\\24381_103385*.obj -Pattern \"^g TUBI_\" | Select-Object -First 30
```

Expected:
- 能看到 `g TUBI_*` 分组（若无，说明导出侧根本没拿到 tubi 实例）

---

### Task 2：验证 cache 中是否具备 arrive/leave 还原所需字段（关键）

**Step 2.1：运行现成的拓扑还原工具（基于 tubi_info_id + ptset_map）**

Run:
```powershell
$env:ROOT_REFNO=\"24381/103385\"
$env:CACHE_DIR=\"output/instance_cache\"
cargo run --example inspect_bran_tubi_topology
```

Interpretation:
- 若输出 `segments=0` 且提示 `inst_tubi_map/tubi_info_id 缺失`，则说明 **cache 中的 tubi 记录不包含 tubi_info_id/ptset_map**（无法按 ARRIVE/LEAVE 点还原端点），这是“按预期绘制”失败的一级嫌疑点。

**Step 2.2：若 segments=0，补一个最小证据：列出 inst_tubi_map 命中与字段概况**

Files:
- Create: `examples/inspect_cache_tubi_refno.rs`

行为：
- 对 `ROOT_REFNO` 的 “self + descendants” 遍历；
- 在最新命中的 batch 中，打印：
  - 是否命中 inst_tubi_map
  - `tubi_info_id` 是否存在
  - `ptset_map.len()`
  - `world_transform.translation/rotation/scale`
  - `tubi_start_pt/tubi_end_pt`（若有）
  - `arrive_axis_pt/leave_axis_pt`（若有）

判定：
- 若命中 tubi，但 `tubi_info_id`/`ptset_map` 缺失，则根因偏向“**生成阶段没有把 arrive/leave 信息带入 cache**”；
- 若字段齐全但仍错位，则转入 Task 3/4。

---

### Task 3：验证 unit_cylinder_mesh 的真实端点定义（排除基础假设错误）

**Step 3.1：导出 unit_cylinder_mesh 并算 bbox**

Run:
```powershell
cargo run --example test_cylinder_mesh -- output/mesh_comparison
python scripts/tmp_obj_stats.py output/mesh_comparison/cylinder_manifold.obj
```

Expected:
- bbox 的 z 范围应接近 `[0,1]`（若为 `[-0.5,0.5]`，则现有 tubi transform 以 start 为 translation 会整体偏移半段长度）

---

### Task 4：定位 transform 分支（get_transform vs build_tubi_transform_from_segment）

**Step 4.1：仅加诊断日志，不改逻辑**

Files:
- Modify: `src/fast_model/cata_model.rs`（在 `dir_ok` 分支处打印一次）

建议日志（仅针对 `is_debug_branch`）：
- `dir_ok` 值
- `start_pt/end_pt`
- `get_transform.translation` 与 `build_tubi_transform_from_segment.translation` 对比
- 若两者一个是 start、一个是 midpoint，则基本锁定“get_transform 约定与 unit_cylinder_mesh 不一致”

**Step 4.2：复跑 Task 1.1，观察日志与 OBJ 是否对应**

---

### Task 5：形成最小修复方向（先不实现，给出可验证结论）

根据 Task 2~4 的证据，选择其一：

1) **缺字段路径**：BRAN/HANG tubing 写入 `inst_tubi_map` 时未填 `tubi_info_id/ptset_map`  
→ 方案：在 `tubi_shape_insts_data.insert_tubi(EleGeosInfo{...})` 补齐（至少 `tubi_info_id` + 足够的 ptset/或直接存 arrive/leave world 点），让导出与调试工具能基于 ARRIVE/LEAVE 还原。

2) **transform 约定不一致**：`current_tubing.get_transform()` 使用“中心对齐”等约定，与 unit_cylinder_mesh(z=0..1) 不匹配  
→ 方案：统一改为 `build_tubi_transform_from_segment(start,end)`，或修正 `get_transform()`（在 aios_core 侧）使其 translation/scale 与 unit cylinder 一致。

3) **坐标系混用**：start/end 为局部坐标却被当世界坐标（或相反）  
→ 方案：明确 start/end 的坐标系来源，必要时在生成阶段补乘 branch/world transform。

---

## Done Criteria（排查完成的定义）

- 能明确回答：错位来自“数据缺失”还是“transform 约定/坐标系错误”，并能用一条可复现命令 + 输出日志/数值对照证明。

