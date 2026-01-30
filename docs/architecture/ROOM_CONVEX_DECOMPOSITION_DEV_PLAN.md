# 房间计算凸分解精算开发方案

> 版本：v1.0 | 创建时间：2026-01-30 | 状态：待实现

## 1. 背景与问题

### 1.1 当前精算算法

当前房间计算使用 **关键点检测 + 射线投射** 方案：

```
候选构件 AABB → 提取 27 个关键点 → 射线投射测试 → 50% 阈值投票
```

**核心代码位置**：
- `src/fast_model/room_model.rs:1189-1203` - `extract_geom_key_points`
- `src/fast_model/room_model.rs:1250-1279` - `is_geom_in_panel`
- `src/fast_model/room_model.rs:1306-1339` - `is_point_inside_mesh_raycast`

### 1.2 当前方案的局限性

| 问题 | 描述 | 影响 |
|-----|------|------|
| 细长构件误判 | 管道、梁等细长构件的 AABB 关键点无法反映实际形状 | 可能遗漏或误判 |
| 复杂形状不适用 | L型、U型等非凸几何体的 AABB 过于简化 | 判断不准确 |
| 50% 阈值问题 | 构件一半在房间内、一半在外时，判定结果不稳定 | 边界情况不可靠 |

### 1.3 改进目标

使用 **凸分解 (Convex Decomposition)** 替代关键点检测：

```
候选构件 → 凸分解凸包列表 → （点在体内 OR 与边界相交）→ 构件在房间内
```

**优势**：
- 精确反映几何形状
- 凸包相交测试算法成熟高效
- 对细长/复杂几何体友好

> ⚠️ 重要校正：仅做 “Convex vs Panel TriMesh 的边界相交” 会漏判  
> 典型场景：构件完全在房间内部但不碰壁，此时与 TriMesh 表面不相交。  
> 因此精算必须采用 **“点在体内 OR 与边界相交”** 的并行判定。

---

## 2. 技术方案

### 2.1 整体架构

```
┌─────────────────────────────────────────────────────────────────┐
│                       Mesh 生成阶段                              │
├─────────────────────────────────────────────────────────────────┤
│  geo_param → PlantMesh → miniacd 凸分解 → ConvexDecomposition   │
│                              ↓                                   │
│                   {geo_hash}_convex.rkyv (缓存)                  │
│  注意：只为 Component 生成凸分解，Panel 不需要                    │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                       房间计算阶段                               │
├─────────────────────────────────────────────────────────────────┤
│  1. 粗算: SQLite RTree AABB 相交查询                            │
│  2. Panel: 加载现有 TriMesh (已有代码)                          │
│  3. Component: 加载凸分解 ConvexPolyhedron                      │
│  4. 精算A: 点在体内（ray-cast + surface distance tol）           │
│  5. 精算B: parry3d::intersection_test(TriMesh, ConvexPolyhedron) │
│  6. 判定: 任一成立（体内 OR 边界相交）→ 构件在房间内             │
└─────────────────────────────────────────────────────────────────┘
```

**关键设计决策**：
- **Panel 使用 TriMesh**：房间边界需要精确表示，TriMesh 可准确描述凹形空间
- **Component 使用 ConvexPolyhedron**：构件凸分解后加速相交测试
- **判定策略**：必须同时做  
  - **体内判定**：采样点经实例变换后，用 ray-cast 判断是否在房间封闭体内（含 tolerance）  
  - **边界相交**：`intersection_test(panel_trimesh, component_hull)` 捕获穿墙/贴边等情况

### 2.2 凸分解库：miniacd

**仓库位置**：`D:\work\plant-code\miniacd`

> 注意：miniacd 的 `Mesh.vertices` 使用 `glamx::DVec3`（非 `glam::DVec3`），集成时需引入 glamx。

**核心 API**：
```rust
// 输入：三角网格
pub struct Mesh {
    pub vertices: Vec<DVec3>,    // f64 精度顶点
    pub faces: Vec<[u32; 3]>     // 三角形索引
}

// 配置
pub struct Config {
    pub threshold: f64,          // 凹度阈值 (默认 0.1，越小凸包越多)
    pub mcts_iterations: usize,  // MCTS 迭代次数 (默认 150)
    pub mcts_depth: usize,       // 搜索深度 (默认 3)
    // ...
}

// 执行凸分解
pub fn run(input: Mesh, config: &Config) -> Vec<Mesh>
```

**算法特点**：
- 基于 CoACD (MCTS) 算法
- 支持并行计算 (rayon)
- 输入需要水密网格

### 2.3 数据结构设计

#### 2.3.1 凸分解结果

```rust
/// 凸分解结果（按 geo_hash 存储）
#[derive(
    Clone, Debug,
    serde::Serialize, serde::Deserialize,
    rkyv::Archive, rkyv::Deserialize, rkyv::Serialize,
)]
pub struct ConvexDecomposition {
    /// 几何体哈希（与 LOD mesh 共用）
    pub geo_hash: String,
    /// 凸包列表
    pub hulls: Vec<ConvexHullData>,
    /// 创建时间戳
    pub created_at: i64,
}

/// 单个凸包数据
#[derive(
    Clone, Debug,
    serde::Serialize, serde::Deserialize,
    rkyv::Archive, rkyv::Deserialize, rkyv::Serialize,
)]
pub struct ConvexHullData {
    /// 顶点列表 (f32 精度，节省空间)
    pub vertices: Vec<[f32; 3]>,
    /// 三角形索引（可选）
    /// 注意：运行时只需顶点即可用 `ConvexPolyhedron::from_convex_hull` 重建凸体；
    /// 为节省体积可不落盘 indices。
    pub indices: Option<Vec<[u32; 3]>>,
    /// 局部 AABB (用于预过滤)
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
}
```

#### 2.3.2 运行时凸包

```rust
impl ConvexHullData {
    /// 转换为 parry3d ConvexPolyhedron（用于碰撞检测）
    pub fn to_convex_polyhedron(&self) -> Option<ConvexPolyhedron> {
        let points: Vec<Point3<f32>> = self.vertices.iter()
            .map(|v| Point3::new(v[0], v[1], v[2]))
            .collect();
        ConvexPolyhedron::from_convex_hull(&points)
    }

    /// 转换为 parry3d Aabb（用于预过滤）
    pub fn to_aabb(&self) -> Aabb {
        Aabb::new(
            Point3::new(self.aabb_min[0], self.aabb_min[1], self.aabb_min[2]),
            Point3::new(self.aabb_max[0], self.aabb_max[1], self.aabb_max[2]),
        )
    }
}
```

### 2.4 存储方案

```
assets/meshes/
├── lod_L0/
│   ├── 12345678_L0.glb
│   └── ...
├── lod_L1/
│   └── ...
├── lod_L2/
│   └── ...
└── convex/                          # 新增目录
    ├── 12345678_convex.rkyv         # rkyv 零拷贝序列化
    ├── 87654321_convex.rkyv
    └── ...
```

**文件命名规则**：`{geo_hash}_convex.rkyv`

**序列化格式**：rkyv（零拷贝反序列化，性能更优）

**rkyv 优势**：
- 零拷贝反序列化：直接访问序列化数据，无需完整解析
- 与项目现有序列化方案一致（rs-core 已广泛使用 rkyv）
- 极低延迟的数据加载

---

## 3. 实现步骤

### 3.1 Step 1: 添加依赖

**文件**：`Cargo.toml`

```toml
[features]
default = ["sqlite-index"]
convex-decomposition = ["dep:miniacd"]

[dependencies]
# 凸分解库（可选）
miniacd = { path = "../miniacd", optional = true }

# rkyv 序列化（已有）
rkyv = { version = "0.8.12", features = ["hashbrown-0_15"] }
```

### 3.2 Step 2: 创建凸分解模块

**新建文件**：`src/fast_model/convex_decomp.rs`

```rust
//! 凸分解模块
//!
//! 为房间计算提供基于凸分解的精确碰撞检测。

use anyhow::{Context, Result};
use dashmap::DashMap;
use once_cell::sync::OnceCell;
use parry3d::bounding_volume::Aabb;
use parry3d::math::Point;
use parry3d::shape::ConvexPolyhedron;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// 凸分解结果
#[derive(
    Clone, Debug,
    Serialize, Deserialize,
    rkyv::Archive, rkyv::Deserialize, rkyv::Serialize,
)]
pub struct ConvexDecomposition {
    pub geo_hash: String,
    pub hulls: Vec<ConvexHullData>,
    pub created_at: i64,
}

/// 单个凸包数据
#[derive(
    Clone, Debug,
    Serialize, Deserialize,
    rkyv::Archive, rkyv::Deserialize, rkyv::Serialize,
)]
pub struct ConvexHullData {
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<[u32; 3]>,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
}

impl ConvexHullData {
    /// 从顶点计算 AABB
    pub fn compute_aabb(vertices: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for v in vertices {
            for i in 0..3 {
                min[i] = min[i].min(v[i]);
                max[i] = max[i].max(v[i]);
            }
        }
        (min, max)
    }

    /// 转换为 parry3d ConvexPolyhedron
    pub fn to_convex_polyhedron(&self) -> Option<ConvexPolyhedron> {
        let points: Vec<Point<f32>> = self.vertices.iter()
            .map(|v| Point::new(v[0], v[1], v[2]))
            .collect();
        ConvexPolyhedron::from_convex_hull(&points)
    }

    /// 转换为 parry3d Aabb
    pub fn to_aabb(&self) -> Aabb {
        Aabb::new(
            Point::new(self.aabb_min[0], self.aabb_min[1], self.aabb_min[2]),
            Point::new(self.aabb_max[0], self.aabb_max[1], self.aabb_max[2]),
        )
    }
}

// ============================================================================
// 缓存管理
// ============================================================================

static CONVEX_CACHE: OnceCell<DashMap<String, Arc<ConvexDecomposition>>> = OnceCell::new();

fn get_cache() -> &'static DashMap<String, Arc<ConvexDecomposition>> {
    CONVEX_CACHE.get_or_init(DashMap::new)
}

/// 清空凸分解缓存
pub fn clear_convex_cache() {
    if let Some(cache) = CONVEX_CACHE.get() {
        cache.clear();
    }
}

/// 获取缓存统计
pub fn get_cache_stats() -> (usize, usize) {
    let cache = get_cache();
    let count = cache.len();
    let memory_estimate = cache.iter()
        .map(|e| e.value().hulls.iter().map(|h| h.vertices.len() * 12).sum::<usize>())
        .sum();
    (count, memory_estimate)
}

// ============================================================================
// 加载与保存
// ============================================================================

/// 加载凸分解结果（带缓存）
pub async fn load_convex_decomposition(
    mesh_dir: &Path,
    geo_hash: &str,
) -> Result<Arc<ConvexDecomposition>> {
    let cache = get_cache();

    // 缓存命中
    if let Some(cached) = cache.get(geo_hash) {
        return Ok(cached.clone());
    }

    // 从文件加载
    let convex_path = mesh_dir.join("convex").join(format!("{}_convex.rkyv", geo_hash));
    let path_clone = convex_path.clone();

    let decomp = tokio::task::spawn_blocking(move || -> Result<ConvexDecomposition> {
        let data = std::fs::read(&path_clone)
            .with_context(|| format!("读取凸分解文件失败: {:?}", path_clone))?;

        // rkyv 零拷贝反序列化
        let archived = unsafe { rkyv::access_unchecked::<ArchivedConvexDecomposition>(&data) };
        let decomp: ConvexDecomposition = archived.deserialize(&mut rkyv::Infallible)
            .map_err(|e| anyhow::anyhow!("rkyv 反序列化失败: {:?}", e))?;
        Ok(decomp)
    }).await??;

    let arc = Arc::new(decomp);
    cache.insert(geo_hash.to_string(), arc.clone());
    Ok(arc)
}

/// 保存凸分解结果
pub fn save_convex_decomposition(
    mesh_dir: &Path,
    decomp: &ConvexDecomposition,
) -> Result<()> {
    let convex_dir = mesh_dir.join("convex");
    std::fs::create_dir_all(&convex_dir)?;

    let path = convex_dir.join(format!("{}_convex.rkyv", decomp.geo_hash));

    // rkyv 序列化
    let data = rkyv::to_bytes::<rkyv::rancor::Error>(decomp)
        .map_err(|e| anyhow::anyhow!("rkyv 序列化失败: {:?}", e))?;
    std::fs::write(&path, &data)?;

    Ok(())
}

// ============================================================================
// 凸分解生成
// ============================================================================

#[cfg(feature = "convex-decomposition")]
pub mod generator {
    use super::*;
    use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
    use aios_core::shape::pdms_shape::PlantMesh;
    use glam::DVec3;

    /// 凸分解配置
    #[derive(Clone, Debug)]
    pub struct ConvexDecompConfig {
        /// 凹度阈值（越小凸包越多，精度越高）
        pub threshold: f64,
        /// MCTS 迭代次数
        pub mcts_iterations: usize,
        /// 是否打印进度
        pub print_progress: bool,
    }

    impl Default for ConvexDecompConfig {
        fn default() -> Self {
            Self {
                threshold: 0.05,      // 较低阈值，更精确
                mcts_iterations: 150,
                print_progress: false,
            }
        }
    }

    /// 为 mesh 生成凸分解
    pub fn generate_from_mesh(
        vertices: &[[f32; 3]],
        indices: &[[u32; 3]],
        geo_hash: &str,
        config: &ConvexDecompConfig,
    ) -> Result<ConvexDecomposition> {
        use miniacd::{Config as MiniAcdConfig, Mesh as MiniAcdMesh};

        // 转换为 miniacd 格式 (f64)
        let verts: Vec<DVec3> = vertices.iter()
            .map(|v| DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64))
            .collect();
        let faces: Vec<[u32; 3]> = indices.to_vec();

        let input_mesh = MiniAcdMesh::new(verts, faces);

        let miniacd_config = MiniAcdConfig {
            threshold: config.threshold,
            mcts_iterations: config.mcts_iterations,
            print: config.print_progress,
            ..Default::default()
        };

        // 执行凸分解
        let hulls = miniacd::run(input_mesh, &miniacd_config);

        // 转换为我们的格式
        let hull_data: Vec<ConvexHullData> = hulls.into_iter()
            .map(|h| {
                let vertices: Vec<[f32; 3]> = h.vertices.iter()
                    .map(|v| [v.x as f32, v.y as f32, v.z as f32])
                    .collect();
                let (aabb_min, aabb_max) = ConvexHullData::compute_aabb(&vertices);
                ConvexHullData {
                    vertices,
                    indices: h.faces,
                    aabb_min,
                    aabb_max,
                }
            })
            .collect();

        Ok(ConvexDecomposition {
            geo_hash: geo_hash.to_string(),
            hulls: hull_data,
            created_at: chrono::Utc::now().timestamp(),
        })
    }

    /// 简单几何体快速凸包生成（无需 miniacd）
    pub fn generate_simple_convex_hull(
        geo_param: &PdmsGeoParam,
        geo_hash: &str,
    ) -> Option<ConvexDecomposition> {
        match geo_param {
            PdmsGeoParam::PrimBox(b) => Some(box_to_convex(b, geo_hash)),
            PdmsGeoParam::PrimSCylinder(c) => Some(cylinder_to_convex(c, geo_hash)),
            PdmsGeoParam::PrimSphere(s) => Some(sphere_to_convex(s, geo_hash)),
            // 其他简单几何体...
            _ => None, // 需要完整凸分解
        }
    }

    fn box_to_convex(b: &aios_core::prim_geo::pdms_box::PrimBox, geo_hash: &str) -> ConvexDecomposition {
        // 盒子本身就是凸的，生成 8 个顶点
        let half = [b.xlen / 2.0, b.ylen / 2.0, b.zlen / 2.0];
        let vertices = vec![
            [-half[0], -half[1], -half[2]],
            [ half[0], -half[1], -half[2]],
            [ half[0],  half[1], -half[2]],
            [-half[0],  half[1], -half[2]],
            [-half[0], -half[1],  half[2]],
            [ half[0], -half[1],  half[2]],
            [ half[0],  half[1],  half[2]],
            [-half[0],  half[1],  half[2]],
        ];
        let (aabb_min, aabb_max) = ConvexHullData::compute_aabb(&vertices);

        ConvexDecomposition {
            geo_hash: geo_hash.to_string(),
            hulls: vec![ConvexHullData {
                vertices,
                indices: vec![], // 凸包不需要索引，parry3d 会重建
                aabb_min,
                aabb_max,
            }],
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    fn cylinder_to_convex(
        c: &aios_core::prim_geo::cylinder::SCylinder,
        geo_hash: &str
    ) -> ConvexDecomposition {
        // 圆柱凸包：上下底面采样点
        const SEGMENTS: usize = 16;
        let r = c.pdia / 2.0;
        let h = c.phei;

        let mut vertices = Vec::with_capacity(SEGMENTS * 2);
        for i in 0..SEGMENTS {
            let angle = (i as f32) * std::f32::consts::TAU / (SEGMENTS as f32);
            let x = r * angle.cos();
            let y = r * angle.sin();
            vertices.push([x, y, 0.0]);      // 底面
            vertices.push([x, y, h]);        // 顶面
        }

        let (aabb_min, aabb_max) = ConvexHullData::compute_aabb(&vertices);

        ConvexDecomposition {
            geo_hash: geo_hash.to_string(),
            hulls: vec![ConvexHullData {
                vertices,
                indices: vec![],
                aabb_min,
                aabb_max,
            }],
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    fn sphere_to_convex(
        s: &aios_core::prim_geo::sphere::Sphere,
        geo_hash: &str
    ) -> ConvexDecomposition {
        // 球的凸包：icosahedron 近似
        let r = s.pdia / 2.0;
        let phi = (1.0 + 5.0_f32.sqrt()) / 2.0; // 黄金比例

        let vertices: Vec<[f32; 3]> = vec![
            [-1.0,  phi, 0.0], [ 1.0,  phi, 0.0], [-1.0, -phi, 0.0], [ 1.0, -phi, 0.0],
            [0.0, -1.0,  phi], [0.0,  1.0,  phi], [0.0, -1.0, -phi], [0.0,  1.0, -phi],
            [ phi, 0.0, -1.0], [ phi, 0.0,  1.0], [-phi, 0.0, -1.0], [-phi, 0.0,  1.0],
        ].into_iter()
            .map(|[x, y, z]| {
                let len = (x*x + y*y + z*z).sqrt();
                [x * r / len, y * r / len, z * r / len]
            })
            .collect();

        let (aabb_min, aabb_max) = ConvexHullData::compute_aabb(&vertices);

        ConvexDecomposition {
            geo_hash: geo_hash.to_string(),
            hulls: vec![ConvexHullData {
                vertices,
                indices: vec![],
                aabb_min,
                aabb_max,
            }],
            created_at: chrono::Utc::now().timestamp(),
        }
    }
}

// ============================================================================
// 碰撞检测
// ============================================================================

use parry3d::math::Isometry;
use parry3d::query::intersection_test;
use parry3d::shape::TriMesh;

/// 判断构件凸包是否与 Panel TriMesh 相交
///
/// 策略：任意一个凸包与 Panel mesh 相交即返回 true
pub fn component_intersects_panel(
    component_hulls: &[ConvexPolyhedron],
    component_transform: &Isometry<f32>,
    panel_mesh: &TriMesh,
    panel_transform: &Isometry<f32>,
) -> bool {
    for hull in component_hulls {
        if intersection_test(component_transform, hull, panel_transform, panel_mesh)
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

/// 判断构件凸包是否与 Panel TriMesh 相交（带 AABB 预过滤）
pub fn component_intersects_panel_with_aabb_filter(
    component_hulls: &[(Aabb, ConvexPolyhedron)],
    component_transform: &Isometry<f32>,
    panel_mesh: &TriMesh,
    panel_aabb: &Aabb,
    panel_transform: &Isometry<f32>,
) -> bool {
    let world_panel_aabb = panel_aabb.transform_by(panel_transform);

    for (hull_aabb, hull) in component_hulls {
        // 变换构件凸包 AABB
        let world_hull_aabb = hull_aabb.transform_by(component_transform);

        // AABB 预过滤
        if !world_hull_aabb.intersects(&world_panel_aabb) {
            continue;
        }

        // 精确相交测试：ConvexPolyhedron vs TriMesh
        if intersection_test(component_transform, hull, panel_transform, panel_mesh)
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}
```

### 3.3 Step 3: 修改 room_model.rs

**文件**：`src/fast_model/room_model.rs`

新增凸分解精算函数：

```rust
#[cfg(feature = "convex-decomposition")]
use crate::fast_model::convex_decomp::{
    load_convex_decomposition,
    component_intersects_panel_with_aabb_filter,
    ConvexDecomposition,
};
use parry3d::shape::TriMesh;

/// 使用凸分解判断构件是否在房间内
///
/// Panel 使用 TriMesh（精确边界），Component 使用 ConvexPolyhedron（加速检测）
#[cfg(feature = "convex-decomposition")]
async fn is_component_in_room_convex(
    mesh_dir: &Path,
    component_geo_hashes: &[String],
    component_transform: &Isometry<f32>,
    panel_mesh: &TriMesh,
    panel_aabb: &Aabb,
    panel_transform: &Isometry<f32>,
) -> bool {
    for geo_hash in component_geo_hashes {
        let Ok(decomp) = load_convex_decomposition(mesh_dir, geo_hash).await else {
            continue;
        };

        // 转换为 parry3d 凸包
        let comp_hulls: Vec<(Aabb, ConvexPolyhedron)> = decomp.hulls.iter()
            .filter_map(|h| {
                h.to_convex_polyhedron().map(|poly| (h.to_aabb(), poly))
            })
            .collect();

        // 判断构件凸包是否与 Panel mesh 相交
        if component_intersects_panel_with_aabb_filter(
            &comp_hulls,
            component_transform,
            panel_mesh,
            panel_aabb,
            panel_transform,
        ) {
            return true;
        }
    }
    false
}

/// 加载 Panel 的 TriMesh（复用现有代码）
///
/// 注意：Panel 不需要凸分解，直接使用原始 TriMesh
#[cfg(feature = "convex-decomposition")]
async fn load_panel_trimesh(
    mesh_dir: &Path,
    panel_geom_insts: &[GeomInstQuery],
) -> Result<(TriMesh, Aabb)> {
    // 复用现有的 Panel mesh 加载逻辑
    // 参考 load_panel_meshes 函数
    load_panel_meshes_as_trimesh(mesh_dir, panel_geom_insts).await
}
```

修改 `cal_room_refnos_with_options` 函数：

```rust
async fn cal_room_refnos_with_options(
    mesh_dir: &PathBuf,
    panel_refno: RefnoEnum,
    exclude_refnos: &HashSet<RefnoEnum>,
    options: RoomComputeOptions,
) -> anyhow::Result<HashSet<RefnoEnum>> {
    // ... 步骤 1-3 保持不变 ...\n
    // 步骤 4: 精算
    #[cfg(feature = "convex-decomposition")]
    {
        // 加载 Panel TriMesh（复用现有代码，Panel 不需要凸分解）
        let (panel_mesh, panel_aabb) = load_panel_trimesh(&mesh_dir, &panel_geom_insts).await?;

        // 使用凸分解精算：Component 凸包 vs Panel TriMesh
        let within_refnos = stream::iter(candidates)
            .map(|candidate_refno| {
                let mesh_dir = mesh_dir.clone();
                let panel_mesh = panel_mesh.clone();
                let panel_aabb = panel_aabb.clone();
                let candidate_geom_map = candidate_geom_map.clone();

                async move {
                    let Some(geom) = candidate_geom_map.get(&candidate_refno) else {
                        return None;
                    };

                    let geo_hashes: Vec<String> = geom.insts.iter()
                        .map(|i| i.geo_hash.clone())
                        .collect();

                    let transform = geom.world_trans.to_isometry();

                    // Component 凸包 vs Panel TriMesh 相交测试
                    if is_component_in_room_convex(
                        &mesh_dir,
                        &geo_hashes,
                        &transform,
                        &panel_mesh,
                        &panel_aabb,
                        &Isometry::identity(),
                    ).await {
                        Some(candidate_refno)
                    } else {
                        None
                    }
                }
            })
            .buffer_unordered(options.candidate_concurrency.max(1))
            .filter_map(|x| async { x })
            .collect::<HashSet<_>>()
            .await;

        return Ok(within_refnos);
    }

    // Fallback: 关键点检测（现有逻辑）
    // ... 保持不变 ...
}
```

### 3.4 Step 4: 在 mesh 生成时调用凸分解

**文件**：`src/fast_model/mesh_generate.rs`

```rust
#[cfg(feature = "convex-decomposition")]
use crate::fast_model::convex_decomp::{
    generator::{generate_from_mesh, generate_simple_convex_hull, ConvexDecompConfig},
    save_convex_decomposition,
};

/// 生成并保存凸分解
#[cfg(feature = "convex-decomposition")]
async fn generate_and_save_convex(
    mesh_dir: &Path,
    geo_hash: &str,
    geo_param: &PdmsGeoParam,
    mesh: &PlantMesh,
) -> Result<()> {
    // 1. 尝试简单几何体快速路径
    if let Some(decomp) = generate_simple_convex_hull(geo_param, geo_hash) {
        save_convex_decomposition(mesh_dir, &decomp)?;
        return Ok(());
    }

    // 2. 复杂几何体使用 miniacd
    let (vertices, indices) = mesh.to_vertices_indices_f32();

    let config = ConvexDecompConfig::default();
    let decomp = generate_from_mesh(&vertices, &indices, geo_hash, &config)?;

    save_convex_decomposition(mesh_dir, &decomp)?;
    Ok(())
}
```

在 mesh 生成流程中调用：

```rust
// 在保存 LOD mesh 之后
if save_result.is_ok() {
    #[cfg(feature = "convex-decomposition")]
    {
        if let Err(e) = generate_and_save_convex(
            &mesh_dir,
            &geo_hash_str,
            &geo_param,
            &mesh,
        ).await {
            warn!("生成凸分解失败: geo_hash={}, error={}", geo_hash_str, e);
        }
    }
}
```

### 3.5 Step 5: 导出模块

**文件**：`src/fast_model/mod.rs`

```rust
#[cfg(feature = "convex-decomposition")]
pub mod convex_decomp;
```

---

## 4. 配置与使用

### 4.1 Feature Flag

```bash
# 启用凸分解功能编译
cargo build --features convex-decomposition

# 同时启用多个 feature
cargo build --features "sqlite-index,convex-decomposition"
```

### 4.2 环境变量

```bash
# 凸分解阈值（可选，默认 0.05）
export CONVEX_DECOMP_THRESHOLD=0.05

# 是否打印凸分解进度（可选，默认 false）
export CONVEX_DECOMP_VERBOSE=false
```

### 4.3 DbOption.toml 配置（可选扩展）

```toml
# 房间计算配置
[room_calc]
# 使用凸分解精算（需要 convex-decomposition feature）
use_convex_decomposition = true
# 凸分解阈值
convex_threshold = 0.05
```

---

## 5. 测试与验证

### 5.1 单元测试

```rust
#[cfg(test)]
#[cfg(feature = "convex-decomposition")]
mod tests {
    use super::*;

    #[test]
    fn test_box_convex_hull() {
        let decomp = generator::box_to_convex(
            &PrimBox { xlen: 2.0, ylen: 2.0, zlen: 2.0, ..Default::default() },
            "test_box",
        );
        assert_eq!(decomp.hulls.len(), 1);
        assert_eq!(decomp.hulls[0].vertices.len(), 8);
    }

    #[test]
    fn test_convex_intersection() {
        // 创建两个相交的凸包
        let hull_a = ConvexPolyhedron::from_convex_hull(&[
            Point::new(0.0, 0.0, 0.0),
            Point::new(2.0, 0.0, 0.0),
            Point::new(1.0, 2.0, 0.0),
            Point::new(1.0, 1.0, 2.0),
        ]).unwrap();

        let hull_b = ConvexPolyhedron::from_convex_hull(&[
            Point::new(1.0, 0.0, 0.0),
            Point::new(3.0, 0.0, 0.0),
            Point::new(2.0, 2.0, 0.0),
            Point::new(2.0, 1.0, 2.0),
        ]).unwrap();

        let intersects = convex_hulls_intersect(
            &[hull_a],
            &Isometry::identity(),
            &[hull_b],
            &Isometry::identity(),
        );

        assert!(intersects);
    }
}
```

### 5.2 集成测试

```bash
# 1. 生成凸分解数据
cargo run --features convex-decomposition --example gen_model -- \
    --db-option DbOption-test.toml

# 2. 验证凸分解文件
ls -la assets/meshes/convex/*.rkyv | head -20

# 3. 运行房间计算
AIOS_ROOM_DEBUG=1 \
cargo run --features "sqlite-index,convex-decomposition" \
    --example room_calc_by_panel_demo

# 4. 对比测试（关闭凸分解）
AIOS_ROOM_DEBUG=1 \
cargo run --features sqlite-index \
    --example room_calc_by_panel_demo
```

### 5.3 性能基准测试

```rust
#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::Instant;

    #[test]
    fn bench_convex_intersection() {
        let hulls = create_test_hulls(100);

        let start = Instant::now();
        for _ in 0..1000 {
            convex_hulls_intersect(&hulls, &Isometry::identity(), &hulls, &Isometry::identity());
        }
        let elapsed = start.elapsed();

        println!("1000 次凸包相交测试耗时: {:?}", elapsed);
        println!("平均每次: {:?}", elapsed / 1000);
    }
}
```

---

## 6. 风险与注意事项

### 6.1 风险点

| 风险 | 影响 | 缓解措施 |
|-----|------|---------|
| 凸分解计算耗时 | mesh 生成变慢 | 预计算 + 并行处理 |
| 凸分解文件增加存储 | 磁盘占用增加 | 估计每个几何体 1-10KB |
| 非水密 mesh 导致失败 | 凸分解失败 | 使用 AABB fallback |
| feature flag 编译隔离 | 测试覆盖不足 | CI 配置多 feature 组合 |

### 6.2 Fallback 策略

当凸分解不可用时自动回退：

1. **编译时**：未启用 `convex-decomposition` feature
2. **运行时**：凸分解文件不存在
3. **异常**：凸分解加载/解析失败

回退到现有的关键点检测方案。

### 6.4 特殊情况：标准单位几何（geo_hash=1/2/3）

项目内存在“单位几何体复用”的约定：geo_hash=1/2/3 在全库复用，实例尺寸通过 transform.scale 还原。  
因此凸分解 **不得按实例尺寸落盘**（否则会污染同 geo_hash 的其他实例），建议：
- 运行时直接用 `unit_*_mesh` 构造“单位凸体/凸包”，再由实例 transform（含缩放）还原；
- 或直接跳过落盘，仅在运行时使用。

### 6.3 性能预期

| 操作 | 当前方案 | 凸分解方案 | 备注 |
|-----|---------|-----------|------|
| 预处理 | 0 | 100ms~1s/几何体 | 一次性 |
| 文件加载 | 1~5ms | 1~5ms | 相当 |
| 精算判定 | 0.1~1ms/关键点 | 0.01~0.1ms/凸包对 | 更快 |
| 总体精算 | N/A | 预期提升 2~5x | 含 AABB 预过滤 |

---

## 7. 里程碑计划

| 阶段 | 内容 | 交付物 |
|-----|------|--------|
| M1 | 凸分解模块基础设施 | `convex_decomp.rs`、数据结构、缓存 |
| M2 | 简单几何体快速路径 | Box/Cylinder/Sphere 直接生成 |
| M3 | miniacd 集成 | 复杂几何体凸分解 |
| M4 | 房间计算集成 | 修改 `room_model.rs` |
| M5 | mesh 生成集成 | 修改 `mesh_generate.rs` |
| M6 | 测试与优化 | 单元测试、集成测试、性能调优 |

---

## 8. 附录

### 8.1 相关文件路径

```
gen_model-dev/
├── Cargo.toml                              # 添加依赖
├── src/
│   └── fast_model/
│       ├── mod.rs                          # 导出模块
│       ├── convex_decomp.rs                # 【新增】凸分解模块
│       ├── room_model.rs                   # 修改精算算法
│       └── mesh_generate.rs                # 添加凸分解生成
└── docs/
    └── architecture/
        └── ROOM_CONVEX_DECOMPOSITION_DEV_PLAN.md  # 本文档

miniacd/                                     # 凸分解库
├── Cargo.toml
└── src/
    ├── lib.rs                              # 核心 API
    ├── mesh.rs                             # Mesh 数据结构
    └── mcts.rs                             # MCTS 算法
```

### 8.2 参考资料

- [CoACD: Collision-Aware Approximate Convex Decomposition](https://github.com/SarahWeiii/CoACD)
- [parry3d 碰撞检测库](https://parry.rs/)
- [miniacd 实现](../../../miniacd/README.md)
