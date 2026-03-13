//! 房间计算：构件凸分解/凸体近似（Convex Decomposition）
//!
//! 目标：在房间关系计算的“细算”阶段，用凸体（Convex）替代 AABB 关键点采样，
//! 以提升细长/复杂几何的稳定性。
//!
//! 重要语义（与 ROOM_CONVEX_DECOMPOSITION_DEV_PLAN.md 对齐）：
//! - “构件在房间内”采用“任意重叠”：只要构件体积与房间体积有交集即可。
//! - 实现上必须同时满足两路判定：
//!   A) 点在体内：解决“完全在房间内部但不碰壁”的漏判（仅靠边界相交会漏）。
//!   B) 与边界相交：解决“穿墙/贴边但采样点都不在内”的漏判。
//!
//! 缓存键约定（用户已锁定）：仅使用 geo_hash，不纳入 LOD/mesh_signature。
//! 因此提供 FORCE_REGEN_CONVEX=1 强制失效机制。

use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
use glam::{Mat4, Vec3};
use once_cell::sync::OnceCell;
use parry3d::bounding_volume::{Aabb, BoundingVolume};
use parry3d::math::{Isometry, Point, Vector};
use parry3d::query::{PointQuery, Ray, RayCast, intersection_test};
use parry3d::shape::{ConvexPolyhedron, TriMesh};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const CONVEX_DECOMP_FILE_VERSION: u32 = 1;

#[derive(
    Clone,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
)]
pub struct ConvexDecompositionFileV1 {
    pub version: u32,
    pub geo_hash: String,
    pub created_at: i64,
    pub params: ConvexDecompParamsV1,
    pub hulls: Vec<ConvexHullDataV1>,
}

#[derive(
    Clone,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
)]
pub struct ConvexDecompParamsV1 {
    pub source: ConvexSourceV1,
    pub threshold: f64,
    pub mcts_iterations: u32,
    pub max_points: u32,
}

#[derive(
    Clone,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
)]
pub enum ConvexSourceV1 {
    Unit,
    MiniAcd,
    Fallback,
}

#[derive(
    Clone,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
)]
pub struct ConvexHullDataV1 {
    pub vertices: Vec<[f32; 3]>,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
}

pub struct ConvexRuntime {
    pub geo_hash: String,
    pub hulls: Vec<ConvexHullRuntime>,
}

pub struct ConvexHullRuntime {
    pub local_aabb: Aabb,
    pub vertices: Vec<[f32; 3]>,
    pub sample_points_local: Vec<Point<f32>>,
}

static CONVEX_CACHE: OnceCell<DashMap<String, Arc<tokio::sync::OnceCell<Arc<ConvexRuntime>>>>> =
    OnceCell::new();

fn cache() -> &'static DashMap<String, Arc<tokio::sync::OnceCell<Arc<ConvexRuntime>>>> {
    CONVEX_CACHE.get_or_init(DashMap::new)
}

pub fn normalize_base_mesh_dir(mesh_dir: &Path) -> PathBuf {
    let mut base = mesh_dir.to_path_buf();
    while let Some(last) = base.file_name().and_then(|n| n.to_str()) {
        if last.starts_with("lod_") {
            base.pop();
        } else {
            break;
        }
    }
    base
}

pub fn convex_file_path(base_mesh_dir: &Path, geo_hash: &str) -> PathBuf {
    base_mesh_dir
        .join("convex")
        .join(format!("{geo_hash}_convex.rkyv"))
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn is_unit_geo_hash(geo_hash: &str) -> bool {
    matches!(geo_hash, "1" | "2" | "3")
}

pub fn clear_convex_cache() {
    cache().clear();
}

pub async fn load_or_build_convex_runtime(
    mesh_dir: &Path,
    geo_hash: &str,
) -> Result<Arc<ConvexRuntime>> {
    let force_regen = env_bool("FORCE_REGEN_CONVEX", false);
    if force_regen {
        cache().remove(geo_hash);
    }

    let cell = cache()
        .entry(geo_hash.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new()))
        .clone();

    cell.get_or_try_init(|| async move {
        // 单位几何体（1/2/3）不能依赖磁盘文件：geo_hash 在全库复用，落盘会被实例尺寸污染。
        if is_unit_geo_hash(geo_hash) {
            return Ok(Arc::new(build_unit_runtime(geo_hash)?));
        }

        let base = normalize_base_mesh_dir(mesh_dir);
        let path = convex_file_path(&base, geo_hash);

        if !force_regen && path.exists() {
            if let Ok(rt) = load_runtime_from_file(&path).await {
                return Ok(Arc::new(rt));
            }
        }

        // 缺文件或读取失败：是否允许“按需生成”。
        //
        // 默认关闭：避免房间计算阶段出现突发的重计算卡顿；如需开启可显式设置
        // ROOM_RELATION_CONVEX_LAZY_BUILD=1。
        let lazy_build = env_bool("ROOM_RELATION_CONVEX_LAZY_BUILD", false);
        if !lazy_build {
            return Err(anyhow!(
                "凸分解文件不存在或不可用且禁用按需生成: geo_hash={}",
                geo_hash
            ));
        }

        // 缺文件或读取失败时的按需生成需要 miniacd（convex-decomposition）。
        #[cfg(feature = "convex-decomposition")]
        {
            let rt = build_and_save_convex_from_glb(&base, geo_hash).await?;
            Ok(rt)
        }
        #[cfg(not(feature = "convex-decomposition"))]
        {
            Err(anyhow!(
                "凸分解文件不存在或不可用且当前未启用 convex-decomposition（miniacd），无法按需生成: geo_hash={}",
                geo_hash
            ))
        }
    })
    .await
    .map(|v| v.clone())
}

async fn load_runtime_from_file(path: &Path) -> Result<ConvexRuntime> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<ConvexRuntime> {
        let data = std::fs::read(&path)
            .with_context(|| format!("读取凸分解文件失败: {}", path.display()))?;
        let file: ConvexDecompositionFileV1 =
            rkyv::from_bytes::<ConvexDecompositionFileV1, rkyv::rancor::Error>(&data)
                .map_err(|e| anyhow!("rkyv 反序列化失败: {:?}", e))?;
        if file.version != CONVEX_DECOMP_FILE_VERSION {
            return Err(anyhow!(
                "凸分解文件版本不匹配: path={} version={} expected={}",
                path.display(),
                file.version,
                CONVEX_DECOMP_FILE_VERSION
            ));
        }
        build_runtime_from_file(&file)
    })
    .await?
}

fn build_runtime_from_file(file: &ConvexDecompositionFileV1) -> Result<ConvexRuntime> {
    let max_points = file.params.max_points.max(4) as usize;
    let mut hulls = Vec::with_capacity(file.hulls.len());
    for h in &file.hulls {
        if h.vertices.len() < 4 {
            continue;
        }
        let local_aabb = Aabb::new(
            Point::new(h.aabb_min[0], h.aabb_min[1], h.aabb_min[2]),
            Point::new(h.aabb_max[0], h.aabb_max[1], h.aabb_max[2]),
        );
        let sample_points_local = sample_points_from_vertices(&h.vertices, max_points);
        hulls.push(ConvexHullRuntime {
            local_aabb,
            vertices: h.vertices.clone(),
            sample_points_local,
        });
    }
    if hulls.is_empty() {
        return Err(anyhow!(
            "凸分解文件有效 hull 为空: geo_hash={}",
            file.geo_hash
        ));
    }
    Ok(ConvexRuntime {
        geo_hash: file.geo_hash.clone(),
        hulls,
    })
}

fn sample_points_from_vertices(vertices: &[[f32; 3]], max_points: usize) -> Vec<Point<f32>> {
    if vertices.is_empty() || max_points == 0 {
        return Vec::new();
    }

    // 质心 + 均匀采样顶点
    let mut centroid = Point::new(0.0f32, 0.0, 0.0);
    for v in vertices {
        centroid.x += v[0];
        centroid.y += v[1];
        centroid.z += v[2];
    }
    let n = vertices.len() as f32;
    centroid.x /= n;
    centroid.y /= n;
    centroid.z /= n;

    let mut points = Vec::with_capacity(max_points.min(vertices.len() + 1));
    points.push(centroid);

    if max_points == 1 {
        return points;
    }

    let want = max_points - 1;
    if vertices.len() <= want {
        points.extend(vertices.iter().map(|v| Point::new(v[0], v[1], v[2])));
        return points;
    }

    let step = (vertices.len() / want).max(1);
    for i in 0..want {
        let idx = i * step;
        if idx >= vertices.len() {
            break;
        }
        let v = vertices[idx];
        points.push(Point::new(v[0], v[1], v[2]));
    }
    points
}

fn compute_aabb_from_vertices(vertices: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
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

fn build_unit_runtime(geo_hash: &str) -> Result<ConvexRuntime> {
    // 与导出侧一致：1/2/3 使用内置 unit_*_mesh，不依赖磁盘 GLB。
    use aios_core::geometry::csg::{unit_box_mesh, unit_cylinder_mesh, unit_sphere_mesh};
    use aios_core::mesh_precision::LodMeshSettings;

    let mesh = match geo_hash {
        "1" => unit_box_mesh(),
        "2" => unit_cylinder_mesh(&LodMeshSettings::default(), false),
        "3" => unit_sphere_mesh(),
        _ => return Err(anyhow!("非单位几何 geo_hash: {}", geo_hash)),
    };

    // 单位几何体本身是凸的：直接使用其所有顶点生成一个凸包（单 hull）。
    let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| [v.x, v.y, v.z]).collect();
    if verts.len() < 4 {
        return Err(anyhow!("单位几何顶点不足: geo_hash={}", geo_hash));
    }
    let (aabb_min, aabb_max) = compute_aabb_from_vertices(&verts);
    let local_aabb = Aabb::new(
        Point::new(aabb_min[0], aabb_min[1], aabb_min[2]),
        Point::new(aabb_max[0], aabb_max[1], aabb_max[2]),
    );

    let max_points = env_usize("ROOM_RELATION_CONVEX_MAX_POINTS", 128).max(4);
    let sample_points_local = sample_points_from_vertices(&verts, max_points);

    Ok(ConvexRuntime {
        geo_hash: geo_hash.to_string(),
        hulls: vec![ConvexHullRuntime {
            local_aabb,
            vertices: verts,
            sample_points_local,
        }],
    })
}

/// 从 base_mesh_dir 的 GLB 构建凸分解并落盘（{base}/convex/{geo_hash}_convex.rkyv）
///
/// 注意：该函数依赖 miniacd，仅在启用 `convex-decomposition` feature 时可用。
#[cfg(feature = "convex-decomposition")]
pub async fn build_and_save_convex_from_glb(
    base_mesh_dir: &Path,
    geo_hash: &str,
) -> Result<Arc<ConvexRuntime>> {
    let threshold = env_f64("CONVEX_DECOMP_THRESHOLD", 0.05);
    let mcts_iterations = env_u32("CONVEX_DECOMP_MCTS_ITERATIONS", 150);
    let max_points = env_u32("ROOM_RELATION_CONVEX_MAX_POINTS", 128).max(4);

    let glb_path = find_any_glb_path(base_mesh_dir, geo_hash).with_context(|| {
        format!(
            "未找到可用 GLB: base={} geo_hash={}",
            base_mesh_dir.display(),
            geo_hash
        )
    })?;

    let geo_hash_string = geo_hash.to_string();
    let mesh = tokio::task::spawn_blocking(move || {
        crate::fast_model::export_model::import_glb::import_glb_to_mesh(&glb_path)
            .with_context(|| format!("加载 GLB 失败: {}", glb_path.display()))
    })
    .await??;

    let (file, runtime) = tokio::task::spawn_blocking(
        move || -> Result<(ConvexDecompositionFileV1, ConvexRuntime)> {
            build_convex_from_plant_mesh(
                &geo_hash_string,
                threshold,
                mcts_iterations,
                max_points,
                &mesh,
            )
        },
    )
    .await??;

    // 写盘（rkyv）
    let out_path = convex_file_path(base_mesh_dir, geo_hash);
    let out_dir = out_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| base_mesh_dir.to_path_buf());
    tokio::task::spawn_blocking(move || -> Result<()> {
        std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("创建 convex 目录失败: {}", out_dir.display()))?;
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&file)
            .map_err(|e| anyhow!("rkyv 序列化失败: {:?}", e))?;
        std::fs::write(&out_path, &bytes)
            .with_context(|| format!("写入凸分解文件失败: {}", out_path.display()))?;
        Ok(())
    })
    .await??;

    Ok(Arc::new(runtime))
}

fn find_any_glb_path(base_mesh_dir: &Path, geo_hash: &str) -> Result<PathBuf> {
    // 与 room_model 的 LOD 探测一致：L0 -> L1 -> L2 -> L3
    for lod in ["L0", "L1", "L2", "L3"] {
        let dir = base_mesh_dir.join(format!("lod_{lod}"));
        let with_suffix = dir.join(format!("{geo_hash}_{lod}.glb"));
        if with_suffix.exists() {
            return Ok(with_suffix);
        }
        let legacy = dir.join(format!("{geo_hash}.glb"));
        if legacy.exists() {
            return Ok(legacy);
        }
    }
    Err(anyhow!("未找到 geo_hash={} 的 GLB（L0..L3）", geo_hash))
}

#[cfg(feature = "convex-decomposition")]
fn build_convex_from_plant_mesh(
    geo_hash: &str,
    threshold: f64,
    mcts_iterations: u32,
    max_points: u32,
    mesh: &aios_core::shape::pdms_shape::PlantMesh,
) -> Result<(ConvexDecompositionFileV1, ConvexRuntime)> {
    // PlantMesh: indices 为 u32 三角索引平铺
    if mesh.vertices.is_empty() || mesh.indices.len() < 3 {
        return Err(anyhow!("PlantMesh 为空: geo_hash={}", geo_hash));
    }

    let vertices_f32: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| [v.x, v.y, v.z]).collect();
    let faces: Vec<[u32; 3]> = mesh
        .indices
        .chunks(3)
        .filter_map(|tri| (tri.len() == 3).then(|| [tri[0], tri[1], tri[2]]))
        .collect();
    if faces.is_empty() {
        return Err(anyhow!("PlantMesh 三角面为空: geo_hash={}", geo_hash));
    }

    // miniacd 需要 glamx::DVec3（非 glam::DVec3）
    let verts_f64: Vec<glamx::DVec3> = vertices_f32
        .iter()
        .map(|v| glamx::dvec3(v[0] as f64, v[1] as f64, v[2] as f64))
        .collect();
    let input_mesh = miniacd::mesh::Mesh::new(verts_f64, faces);

    let config = miniacd::Config {
        threshold,
        mcts_iterations: mcts_iterations as usize,
        print: env_bool("CONVEX_DECOMP_VERBOSE", false),
        ..Default::default()
    };

    let parts = miniacd::run(input_mesh, &config);

    let mut hulls_file: Vec<ConvexHullDataV1> = Vec::new();
    let mut hulls_runtime: Vec<ConvexHullRuntime> = Vec::new();

    for p in parts {
        if p.vertices.len() < 4 {
            continue;
        }
        let verts: Vec<[f32; 3]> = p
            .vertices
            .iter()
            .map(|v| [v.x as f32, v.y as f32, v.z as f32])
            .collect();
        let (aabb_min, aabb_max) = compute_aabb_from_vertices(&verts);
        let local_aabb = Aabb::new(
            Point::new(aabb_min[0], aabb_min[1], aabb_min[2]),
            Point::new(aabb_max[0], aabb_max[1], aabb_max[2]),
        );
        let sample_points_local = sample_points_from_vertices(&verts, max_points as usize);

        hulls_file.push(ConvexHullDataV1 {
            vertices: verts.clone(),
            aabb_min,
            aabb_max,
        });
        hulls_runtime.push(ConvexHullRuntime {
            local_aabb,
            vertices: verts,
            sample_points_local,
        });
    }

    // miniacd 输出可能为空：回退为“单凸包”（对所有顶点取 convex hull）
    if hulls_runtime.is_empty() {
        let (aabb_min, aabb_max) = compute_aabb_from_vertices(&vertices_f32);
        let local_aabb = Aabb::new(
            Point::new(aabb_min[0], aabb_min[1], aabb_min[2]),
            Point::new(aabb_max[0], aabb_max[1], aabb_max[2]),
        );
        let sample_points_local = sample_points_from_vertices(&vertices_f32, max_points as usize);

        hulls_file.push(ConvexHullDataV1 {
            vertices: vertices_f32.clone(),
            aabb_min,
            aabb_max,
        });
        hulls_runtime.push(ConvexHullRuntime {
            local_aabb,
            vertices: vertices_f32.clone(),
            sample_points_local,
        });
    }

    let params = ConvexDecompParamsV1 {
        source: if is_unit_geo_hash(geo_hash) {
            ConvexSourceV1::Unit
        } else {
            ConvexSourceV1::MiniAcd
        },
        threshold,
        mcts_iterations,
        max_points,
    };

    let file = ConvexDecompositionFileV1 {
        version: CONVEX_DECOMP_FILE_VERSION,
        geo_hash: geo_hash.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        params,
        hulls: hulls_file,
    };
    let runtime = ConvexRuntime {
        geo_hash: geo_hash.to_string(),
        hulls: hulls_runtime,
    };
    Ok((file, runtime))
}

fn is_point_inside_mesh_raycast(point: &Point<f32>, tri_mesh: &TriMesh) -> bool {
    let identity = Isometry::identity();

    let ray_pos_z = Ray::new(*point, Vector::new(0.0, 0.0, 1.0));
    let ray_neg_z = Ray::new(*point, Vector::new(0.0, 0.0, -1.0));
    let hit_pos_z = tri_mesh.cast_ray(&identity, &ray_pos_z, f32::MAX, true);
    let hit_neg_z = tri_mesh.cast_ray(&identity, &ray_neg_z, f32::MAX, true);
    if hit_pos_z.is_some() && hit_neg_z.is_some() {
        return true;
    }

    let ray_pos_x = Ray::new(*point, Vector::new(1.0, 0.0, 0.0));
    let ray_neg_x = Ray::new(*point, Vector::new(-1.0, 0.0, 0.0));
    let hit_pos_x = tri_mesh.cast_ray(&identity, &ray_pos_x, f32::MAX, true);
    let hit_neg_x = tri_mesh.cast_ray(&identity, &ray_neg_x, f32::MAX, true);
    if hit_pos_x.is_some() && hit_neg_x.is_some() {
        return true;
    }

    let ray_pos_y = Ray::new(*point, Vector::new(0.0, 1.0, 0.0));
    let ray_neg_y = Ray::new(*point, Vector::new(0.0, -1.0, 0.0));
    let hit_pos_y = tri_mesh.cast_ray(&identity, &ray_pos_y, f32::MAX, true);
    let hit_neg_y = tri_mesh.cast_ray(&identity, &ray_neg_y, f32::MAX, true);
    hit_pos_y.is_some() && hit_neg_y.is_some()
}

fn is_point_inside_any_mesh(
    point: &Point<f32>,
    panel_meshes: &[Arc<TriMesh>],
    tolerance_sq: f32,
) -> bool {
    for mesh in panel_meshes {
        if is_point_inside_mesh_raycast(point, mesh.as_ref()) {
            return true;
        }
        let projection = mesh.project_point(&Isometry::identity(), point, true);
        let distance_sq = (projection.point - point).norm_squared();
        if distance_sq <= tolerance_sq {
            return true;
        }
    }
    false
}

/// “任意重叠”判定：点在体内 OR 与边界相交。
pub fn component_overlaps_room(
    panel_meshes: &[Arc<TriMesh>],
    panel_world_aabb: &Aabb,
    component_mat: &Mat4,
    component_hulls: &ConvexRuntime,
    tolerance: f32,
) -> bool {
    if panel_meshes.is_empty() || component_hulls.hulls.is_empty() {
        return false;
    }

    let tolerance_sq = tolerance * tolerance;
    let identity = Isometry::identity();

    for hull in &component_hulls.hulls {
        // 预过滤：world AABB 不交则跳过
        let world_hull_aabb = transform_aabb_by_mat4(&hull.local_aabb, component_mat);
        if !world_hull_aabb.intersects(panel_world_aabb) {
            continue;
        }

        // A) 点在体内（或足够接近表面）
        for p_local in &hull.sample_points_local {
            let p_world = transform_point_by_mat4(p_local, component_mat);
            if is_point_inside_any_mesh(&p_world, panel_meshes, tolerance_sq) {
                return true;
            }
        }

        // B) 与边界相交
        let Some(world_poly) = build_world_convex_polyhedron(&hull.vertices, component_mat) else {
            continue;
        };
        for panel in panel_meshes {
            if intersection_test(&identity, &world_poly, &identity, panel.as_ref()).unwrap_or(false)
            {
                return true;
            }
        }
    }

    false
}

fn transform_point_by_mat4(p: &Point<f32>, mat: &Mat4) -> Point<f32> {
    let v = mat.transform_point3(Vec3::new(p.x, p.y, p.z));
    Point::new(v.x, v.y, v.z)
}

fn transform_aabb_by_mat4(aabb: &Aabb, mat: &Mat4) -> Aabb {
    // 变换 8 个角点后取包围盒，支持非均匀缩放 + 旋转。
    let mins = aabb.mins;
    let maxs = aabb.maxs;
    let corners = [
        Point::new(mins.x, mins.y, mins.z),
        Point::new(maxs.x, mins.y, mins.z),
        Point::new(mins.x, maxs.y, mins.z),
        Point::new(maxs.x, maxs.y, mins.z),
        Point::new(mins.x, mins.y, maxs.z),
        Point::new(maxs.x, mins.y, maxs.z),
        Point::new(mins.x, maxs.y, maxs.z),
        Point::new(maxs.x, maxs.y, maxs.z),
    ];

    let mut out = Aabb::new_invalid();
    for c in corners {
        let wc = transform_point_by_mat4(&c, mat);
        out.take_point(wc);
    }
    out
}

fn build_world_convex_polyhedron(
    local_vertices: &[[f32; 3]],
    mat: &Mat4,
) -> Option<ConvexPolyhedron> {
    if local_vertices.len() < 4 {
        return None;
    }
    let points: Vec<Point<f32>> = local_vertices
        .iter()
        .map(|v| {
            let p = Point::new(v[0], v[1], v[2]);
            transform_point_by_mat4(&p, mat)
        })
        .collect();
    ConvexPolyhedron::from_convex_hull(&points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parry3d::shape::TriMeshFlags;

    fn create_test_cube_trimesh(min: Point<f32>, max: Point<f32>) -> TriMesh {
        let vertices = vec![
            Point::new(min.x, min.y, min.z),
            Point::new(max.x, min.y, min.z),
            Point::new(max.x, max.y, min.z),
            Point::new(min.x, max.y, min.z),
            Point::new(min.x, min.y, max.z),
            Point::new(max.x, min.y, max.z),
            Point::new(max.x, max.y, max.z),
            Point::new(min.x, max.y, max.z),
        ];

        let indices = vec![
            [0, 1, 2],
            [0, 2, 3],
            [4, 6, 5],
            [4, 7, 6],
            [0, 5, 1],
            [0, 4, 5],
            [2, 7, 3],
            [2, 6, 7],
            [0, 3, 7],
            [0, 7, 4],
            [1, 5, 6],
            [1, 6, 2],
        ];

        TriMesh::with_flags(
            vertices,
            indices,
            TriMeshFlags::ORIENTED | TriMeshFlags::MERGE_DUPLICATE_VERTICES,
        )
        .expect("create_test_cube_trimesh")
    }

    fn cube_room(min: [f32; 3], max: [f32; 3]) -> (Vec<Arc<TriMesh>>, Aabb) {
        let room = Arc::new(create_test_cube_trimesh(
            Point::new(min[0], min[1], min[2]),
            Point::new(max[0], max[1], max[2]),
        ));
        let panel_meshes = vec![room];
        let panel_aabb = Aabb::new(
            Point::new(min[0], min[1], min[2]),
            Point::new(max[0], max[1], max[2]),
        );
        (panel_meshes, panel_aabb)
    }

    fn box_vertices(min: [f32; 3], max: [f32; 3]) -> Vec<[f32; 3]> {
        vec![
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], max[1], min[2]],
            [min[0], max[1], min[2]],
            [min[0], min[1], max[2]],
            [max[0], min[1], max[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
        ]
    }

    fn only_corners_as_samples(verts: &[[f32; 3]]) -> Vec<Point<f32>> {
        verts.iter().map(|v| Point::new(v[0], v[1], v[2])).collect()
    }

    fn centroid_only_sample(verts: &[[f32; 3]]) -> Vec<Point<f32>> {
        let mut c = Point::new(0.0f32, 0.0, 0.0);
        for v in verts {
            c.x += v[0];
            c.y += v[1];
            c.z += v[2];
        }
        let n = verts.len().max(1) as f32;
        c.x /= n;
        c.y /= n;
        c.z /= n;
        vec![c]
    }

    fn runtime_single_hull(
        geo_hash: &str,
        verts: Vec<[f32; 3]>,
        sample_points_local: Vec<Point<f32>>,
    ) -> ConvexRuntime {
        let (aabb_min, aabb_max) = compute_aabb_from_vertices(&verts);
        let local_aabb = Aabb::new(
            Point::new(aabb_min[0], aabb_min[1], aabb_min[2]),
            Point::new(aabb_max[0], aabb_max[1], aabb_max[2]),
        );
        ConvexRuntime {
            geo_hash: geo_hash.to_string(),
            hulls: vec![ConvexHullRuntime {
                local_aabb,
                vertices: verts,
                sample_points_local,
            }],
        }
    }

    fn merge_aabb(a: &Aabb, b: &Aabb) -> Aabb {
        let mins = Point::new(
            a.mins.x.min(b.mins.x),
            a.mins.y.min(b.mins.y),
            a.mins.z.min(b.mins.z),
        );
        let maxs = Point::new(
            a.maxs.x.max(b.maxs.x),
            a.maxs.y.max(b.maxs.y),
            a.maxs.z.max(b.maxs.z),
        );
        Aabb::new(mins, maxs)
    }

    fn runtime_from_box(min: [f32; 3], max: [f32; 3]) -> ConvexRuntime {
        let verts = box_vertices(min, max);
        let local_aabb = Aabb::new(
            Point::new(min[0], min[1], min[2]),
            Point::new(max[0], max[1], max[2]),
        );
        let sample_points_local = sample_points_from_vertices(&verts, 32);
        ConvexRuntime {
            geo_hash: "test".to_string(),
            hulls: vec![ConvexHullRuntime {
                local_aabb,
                vertices: verts,
                sample_points_local,
            }],
        }
    }

    #[test]
    fn overlap_true_when_fully_inside_without_surface_intersection() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        let comp = runtime_from_box([2.0, 2.0, 2.0], [3.0, 3.0, 3.0]);
        let mat = Mat4::IDENTITY;

        // 若只用 intersection_test（边界相交），这里很可能为 false；本实现应为 true（点在内）。
        assert!(component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.001
        ));
    }

    #[test]
    fn overlap_false_when_separated() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        let comp = runtime_from_box([20.0, 20.0, 20.0], [21.0, 21.0, 21.0]);
        let mat = Mat4::IDENTITY;
        assert!(!component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.001
        ));
    }

    #[test]
    fn overlap_true_when_crossing_boundary_without_any_sample_point_inside() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        // 该构件与房间体积相交（0..10 x 0..1 x 0..1），但所有“采样点”（此处刻意只取 8 个角点）都在房间外。
        // 因此 A) 点在体内必为 false，必须依赖 B) 边界相交为 true。
        let min = [-1.0, -1.0, -1.0];
        let max = [11.0, 1.0, 1.0];
        let verts = box_vertices(min, max);
        let sample_points_local = only_corners_as_samples(&verts);
        let comp = runtime_single_hull("test", verts, sample_points_local);

        let mat = Mat4::IDENTITY;
        assert!(component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.001
        ));
    }

    #[test]
    fn overlap_true_under_non_uniform_scale_transform() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        let comp = runtime_from_box([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mat = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 0.5),
            glam::Quat::IDENTITY,
            Vec3::new(1.0, 1.0, 1.0),
        );

        // inst.geo_transform 可能包含非均匀缩放，这里验证 Mat4 路径能正确把采样点/凸体变换到世界坐标。
        assert!(component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.001
        ));
    }

    #[test]
    fn overlap_true_when_inside_only_centroid_sample() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        // Hull 完全位于房间内；采样点只给质心，强制依赖 A) 点在体内。
        let verts = box_vertices([2.0, 2.0, 2.0], [3.0, 3.0, 3.0]);
        let sample_points_local = centroid_only_sample(&verts);
        let comp = runtime_single_hull("test", verts, sample_points_local);
        let mat = Mat4::IDENTITY;

        assert!(component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.0
        ));
    }

    #[test]
    fn overlap_true_when_point_outside_but_within_tolerance_of_surface() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        // Hull 本体放在房间内部，保证 AABB 预过滤通过且 B) 边界相交不成立。
        let verts = box_vertices([2.0, 2.0, 2.0], [3.0, 3.0, 3.0]);

        // 采样点刻意放在房间外侧，但离 x=0 面非常近；应被 project_point + tolerance 判定为“在内”。
        let eps = 1.0e-3f32;
        let sample_points_local = vec![Point::new(-eps, 5.0, 5.0)];
        let comp = runtime_single_hull("test", verts, sample_points_local);
        let mat = Mat4::IDENTITY;

        assert!(component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            eps * 1.1
        ));
    }

    #[test]
    fn overlap_false_when_aabb_prefilter_blocks_even_if_sample_points_inside() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        // Hull 远离房间（AABB 不相交），但采样点故意放在房间内部；
        // 预期仍为 false：AABB 预过滤应最先把它筛掉。
        let verts = box_vertices([20.0, 20.0, 20.0], [21.0, 21.0, 21.0]);
        let sample_points_local = vec![Point::new(5.0, 5.0, 5.0)];
        let comp = runtime_single_hull("test", verts, sample_points_local);
        let mat = Mat4::IDENTITY;

        assert!(!component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.1
        ));
    }

    #[test]
    fn overlap_true_when_any_hull_overlaps() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        // hull1: far away -> false
        let verts1 = box_vertices([20.0, 20.0, 20.0], [21.0, 21.0, 21.0]);
        let (aabb1_min, aabb1_max) = compute_aabb_from_vertices(&verts1);
        let hull1 = ConvexHullRuntime {
            local_aabb: Aabb::new(
                Point::new(aabb1_min[0], aabb1_min[1], aabb1_min[2]),
                Point::new(aabb1_max[0], aabb1_max[1], aabb1_max[2]),
            ),
            vertices: verts1,
            sample_points_local: vec![Point::new(20.5, 20.5, 20.5)],
        };

        // hull2: inside -> true (A path)
        let verts2 = box_vertices([2.0, 2.0, 2.0], [3.0, 3.0, 3.0]);
        let (aabb2_min, aabb2_max) = compute_aabb_from_vertices(&verts2);
        let hull2 = ConvexHullRuntime {
            local_aabb: Aabb::new(
                Point::new(aabb2_min[0], aabb2_min[1], aabb2_min[2]),
                Point::new(aabb2_max[0], aabb2_max[1], aabb2_max[2]),
            ),
            sample_points_local: vec![Point::new(2.5, 2.5, 2.5)],
            vertices: verts2,
        };

        let comp = ConvexRuntime {
            geo_hash: "test".to_string(),
            hulls: vec![hull1, hull2],
        };
        let mat = Mat4::IDENTITY;
        assert!(component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.001
        ));
    }

    #[test]
    fn overlap_true_when_any_panel_contains_point() {
        // 两个不相交房间：一个在原点，一个在远处。
        let room1_min = [0.0, 0.0, 0.0];
        let room1_max = [10.0, 10.0, 10.0];
        let room2_min = [100.0, 100.0, 100.0];
        let room2_max = [110.0, 110.0, 110.0];

        let room1 = Arc::new(create_test_cube_trimesh(
            Point::new(room1_min[0], room1_min[1], room1_min[2]),
            Point::new(room1_max[0], room1_max[1], room1_max[2]),
        ));
        let room2 = Arc::new(create_test_cube_trimesh(
            Point::new(room2_min[0], room2_min[1], room2_min[2]),
            Point::new(room2_max[0], room2_max[1], room2_max[2]),
        ));
        let panel_meshes = vec![room1, room2];

        // 注意：panel_world_aabb 在生产中是多个 panel 的合并，这里也需要合并，否则会被 AABB 预过滤误伤。
        let aabb1 = Aabb::new(
            Point::new(room1_min[0], room1_min[1], room1_min[2]),
            Point::new(room1_max[0], room1_max[1], room1_max[2]),
        );
        let aabb2 = Aabb::new(
            Point::new(room2_min[0], room2_min[1], room2_min[2]),
            Point::new(room2_max[0], room2_max[1], room2_max[2]),
        );
        let panel_aabb = merge_aabb(&aabb1, &aabb2);

        // Hull 本体无所谓，这里用一个小盒子；采样点放进 room2，要求 any(panel) 返回 true。
        let verts = box_vertices([1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
        let sample_points_local = vec![Point::new(105.0, 105.0, 105.0)];
        let comp = runtime_single_hull("test", verts, sample_points_local);
        let mat = Mat4::IDENTITY;

        assert!(component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.001
        ));
    }

    #[test]
    fn overlap_true_under_rotation_and_non_uniform_scale_transform() {
        let (panel_meshes, panel_aabb) = cube_room([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);

        let comp = runtime_from_box([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let mat = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 0.5),
            glam::Quat::from_rotation_z(0.7),
            Vec3::new(5.0, 5.0, 5.0),
        );

        assert!(component_overlaps_room(
            &panel_meshes,
            &panel_aabb,
            &mat,
            &comp,
            0.001
        ));
    }
}
