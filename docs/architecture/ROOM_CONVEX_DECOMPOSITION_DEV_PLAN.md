# 房间计算凸分解精算：实现对齐与验收说明

> 版本：v1.1  
> 创建时间：2026-01-30  
> 更新时间：2026-01-30  
> 状态：已实现（本文档对齐代码）

## 1. 背景与问题

### 1.1 现有精算算法（旧）

旧算法基于 **AABB 关键点 + 射线投射点包含**：

```
候选构件 AABB → 提取 27 个关键点 → 点在封闭 TriMesh 内测试 → 50% 阈值投票
```

局限性：
- 细长构件（管/梁）用 AABB 关键点很容易漏判/误判。
- 凹形/复杂几何体（L/U 等）AABB 过于粗糙。
- 50% 投票阈值在“半进半出”场景不稳定。

### 1.2 改进目标（新）

引入 **凸分解（Convex Decomposition）** 作为精算几何近似，并采用“任意重叠”语义：

```
候选构件 → 凸分解（凸体列表）→ 任意重叠判定 → 构件在房间内
```

> 关键语义：**“构件在房间内” = 构件体积与房间体积存在交集**。  
> 仅用 “Convex vs Room 边界 TriMesh 相交” 会漏判“完全在房间内部但不碰壁”的构件，  
> 因此必须采用 **点在体内 OR 与边界相交** 的并行判定。

## 2. 整体架构（与现实现一致）

```
┌───────────────────────────────────────────────────────────────────────┐
│ Mesh/缓存阶段（可选预计算）                                            │
├───────────────────────────────────────────────────────────────────────┤
│ PlantMesh → 导出 GLB → (AIOS_PRECOMPUTE_CONVEX=1 时) build convex     │
│                                  ↓                                    │
│                      {base_mesh_dir}/convex/{geo_hash}_convex.rkyv     │
│ 说明：                                                                │
│ - 凸分解只针对 “Component(inst.geo_hash)”；Panel 仍使用 TriMesh。       │
│ - geo_hash=1/2/3 为单位几何，全库复用：默认不落盘（避免实例尺寸污染）。 │
└───────────────────────────────────────────────────────────────────────┘
                                 │
                                 ▼
┌───────────────────────────────────────────────────────────────────────┐
│ 房间计算阶段（粗算 + 精算）                                            │
├───────────────────────────────────────────────────────────────────────┤
│ 1) 粗算：SQLite RTree / spatial_index 对 panel_aabb 做候选 AABB 相交查询 │
│ 2) Panel：加载/缓存 TriMesh（已存在逻辑）                               │
│ 3) Component：加载凸分解 runtime（落盘读取；必要时可按需生成）           │
│ 4) 精算（任意重叠）：                                                   │
│    A) 点在体内：采样点经 Mat4 变换后，做 ray-cast 点包含（含 tol）       │
│    B) 边界相交：intersection_test(ConvexPolyhedron, TriMesh)            │
│ 5) 判定：A 或 B 任一成立 → 构件在房间内                                  │
└───────────────────────────────────────────────────────────────────────┘
```

## 3. 数据结构（落盘/运行时）

> 源码：`src/fast_model/convex_decomp.rs`

### 3.1 落盘格式（rkyv）

文件：`{base_mesh_dir}/convex/{geo_hash}_convex.rkyv`

```rust
pub const CONVEX_DECOMP_FILE_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
pub struct ConvexDecompositionFileV1 {
    pub version: u32,
    pub geo_hash: String,
    pub created_at: i64,
    pub params: ConvexDecompParamsV1,
    pub hulls: Vec<ConvexHullDataV1>,
}

#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
pub struct ConvexDecompParamsV1 {
    pub source: ConvexSourceV1,
    pub threshold: f64,
    pub mcts_iterations: u32,
    pub max_points: u32,
}

#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
pub enum ConvexSourceV1 {
    Unit,
    MiniAcd,
    Fallback,
}

#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
pub struct ConvexHullDataV1 {
    pub vertices: Vec<[f32; 3]>,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
}
```

### 3.2 运行时结构

```rust
pub struct ConvexRuntime {
    pub geo_hash: String,
    pub hulls: Vec<ConvexHullRuntime>,
}

pub struct ConvexHullRuntime {
    pub local_aabb: parry3d::bounding_volume::Aabb,
    pub vertices: Vec<[f32; 3]>,
    pub sample_points_local: Vec<parry3d::math::Point<f32>>,
}
```

采样点策略：**质心 + 均匀采样顶点**，数量上限由 `max_points` 控制。

## 4. 代码落点（入口与职责）

### 4.1 凸分解模块（核心）

文件：`src/fast_model/convex_decomp.rs`
- `load_or_build_convex_runtime(mesh_dir: &Path, geo_hash: &str) -> Result<Arc<ConvexRuntime>>`
  - 读取/缓存凸分解；当落盘缺失或损坏时，可按需生成（见运行时开关）。
- `build_and_save_convex_from_glb(base_mesh_dir: &Path, geo_hash: &str) -> Result<Arc<ConvexRuntime>>`
  - 从 GLB 读取三角网格，调用 miniacd 凸分解，构建并落盘。
- `component_overlaps_room(panel_meshes, panel_world_aabb, component_mat, component_hulls, tol) -> bool`
  - “任意重叠”判定：点在体内 OR 与边界相交。
  - 关键点：`component_mat: Mat4` 用于表达 inst.transform 中可能存在的 **非均匀缩放**（Isometry 不足）。

### 4.2 房间计算集成（粗算/细算）

文件：`src/fast_model/room_model.rs`
- `cal_room_refnos_with_options`：
  - `ROOM_RELATION_USE_CONVEX=1` 且启用 `--features convex-decomposition` 时，走凸分解细算分支。
  - 凸分解不可用时，可用 `ROOM_RELATION_ALLOW_AABB_FALLBACK=1` 回退旧算法。

### 4.3 Mesh 生成阶段预计算（可选）

文件：`src/fast_model/mesh_generate.rs`
- `gen_inst_meshes`：GLB 导出成功后，若 `AIOS_PRECOMPUTE_CONVEX=1` 则预计算凸分解落盘。

## 5. 编译与运行时开关（默认值以代码为准）

### 5.1 Feature Flag

启用凸分解编译：

```bash
cargo build --features "sqlite-index,convex-decomposition"
```

Cargo.toml 关键项（节选）：
- `convex-decomposition = ["dep:miniacd", "dep:glamx"]`
- `miniacd = { path = "../miniacd", optional = true }`

### 5.2 运行时环境变量

房间计算侧：
- `ROOM_RELATION_USE_CONVEX=1`：启用凸分解精算（默认关闭）。
- `ROOM_RELATION_ALLOW_AABB_FALLBACK=1`：凸分解不可用时回退旧算法（默认关闭）。
- `ROOM_RELATION_CONVEX_LAZY_BUILD=1`：允许“缺文件时按需生成凸分解”（默认关闭；避免房间计算阶段卡顿）。
- `FORCE_REGEN_CONVEX=1`：强制失效进程内 convex cache（用于调参/回归）。

凸分解参数：
- `CONVEX_DECOMP_THRESHOLD`：默认 0.05（越小 hull 越多，越慢但更贴形）。
- `CONVEX_DECOMP_MCTS_ITERATIONS`：默认 150。
- `ROOM_RELATION_CONVEX_MAX_POINTS`：默认 128（采样点上限，质心+采样顶点）。
- `CONVEX_DECOMP_VERBOSE`：默认 false（打印 miniacd 进度）。

预计算侧：
- `AIOS_PRECOMPUTE_CONVEX=1`：在 mesh 生成导出 GLB 成功后预计算并落盘（默认关闭）。

## 6. 测试与验收（SOP）

### 6.1 单元测试（建议 CI 覆盖）

```bash
cargo test -p aios-database --features "sqlite-index,convex-decomposition" convex_decomp::tests -- --nocapture
```

覆盖重点（见 `src/fast_model/convex_decomp.rs` 的 tests）：
- 完全在房间内部但不碰壁：仅靠边界相交会漏 → 必须由“点在体内”兜住
- 穿墙/贴边但采样点全在外：必须由“边界相交”兜住
- 非均匀缩放 Mat4 变换路径：采样点/凸体变换正确

### 6.2 集成回归（两条路径）

#### 路径 A：全流程最小闭环（生成模型 → 房间计算）

示例：`examples/room_calculation_demo.rs`

```bat
REM 可选：限制生成范围（逗号分隔 refno）
set DEBUG_REFNOS=17496/199296

REM 可选：预计算凸分解
set AIOS_PRECOMPUTE_CONVEX=1

REM 启用凸分解精算
set ROOM_RELATION_USE_CONVEX=1
set ROOM_RELATION_ALLOW_AABB_FALLBACK=1

cargo run --example room_calculation_demo --features "gen_model,sqlite-index,convex-decomposition" -- --nocapture
```

验收点：
- 生成后 `{meshes_path}/convex/*.rkyv` 数量增加（若开启预计算）。
- 房间计算可完成，且与关闭凸分解时的结果差异可解释（细长/复杂构件命中更稳定）。

#### 路径 B：单 panel 快速回归（只重建该 panel 的 room_relate）

示例：`examples/room_calc_by_panel_demo.rs`（注意：该示例不自动生成 mesh/索引）

```bat
set PANEL_REFNO=17496/199296
set DBOPTION_PATH=DbOption-room-pane17496-cache
set ROOM_RELATION_USE_CONVEX=1
set ROOM_RELATION_ALLOW_AABB_FALLBACK=1

cargo run --example room_calc_by_panel_demo --features "sqlite-index,convex-decomposition" -- --nocapture
```

### 6.3 可视化调试（OBJ 截图链路）

二进制：`aios-database`（`src/main.rs` 已提供 `--capture`）

```bat
REM 生成/重建指定 refno 模型，并导出 OBJ + 截图到 output/screenshots（可自定义目录）
cargo run --bin aios-database --features "gen_model,sqlite-index" -- ^
  --debug-model 17496/199296 --regen-model --capture output/screenshots --capture-views 3
```

输出：
- `output/screenshots/obj-cache/*.obj`
- `output/screenshots/*.png`（及 viewXX 额外视角）

## 7. 风险与注意事项

| 风险 | 影响 | 缓解 |
|---|---|---|
| 凸分解耗时 | 预计算/按需生成可能慢 | 默认关闭按需生成；需要时用 `AIOS_PRECOMPUTE_CONVEX=1` 预计算 |
| 文件占用 | convex 缓存增长 | f32 顶点 + AABB；必要时清理/按项目分目录 |
| 非水密 mesh | miniacd 可能失败或输出为空 | 输出为空时回退 “单凸包”；仍失败时可回退旧算法 |
| 参数敏感 | threshold/iterations 影响稳定性与耗时 | 用 `FORCE_REGEN_CONVEX=1` 配合单 panel 回归调参 |

## 8. 里程碑（现状）

- 已完成：
  - `convex_decomp` 模块（落盘/加载/缓存/重叠判定）
  - `room_model` 集成（`ROOM_RELATION_USE_CONVEX` 控制）
  - `mesh_generate` 可选预计算（`AIOS_PRECOMPUTE_CONVEX`）
  - 单测覆盖关键语义（点在内 / 边界相交 / Mat4 缩放）
- 可选增强（后续）：
  - 为特定 noun/类型配置凸分解参数（而非仅 env）
  - 针对极端大网格的降采样/分片策略
