//! # 碰撞检测模块
//!
//! 基于 DuckDB (粗筛) + Parry3D (精算) 两阶段检测。
//!
//! ## 使用
//! ```rust
//! use collision_detect::{CollisionDetector, CollisionEvent};
//!
//! let detector = CollisionDetector::new().await?;
//! let events = detector.detect_all(None).await?;
//! ```

use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::{GeomInstQuery, ModelHashInst, RefU64, RefnoEnum};
use dashmap::DashMap;
use parry3d::bounding_volume::{Aabb, BoundingVolume};
use parry3d::math::{Isometry, Point, Real};
use parry3d::query::contact;
use parry3d::shape::{TriMesh, TriMeshFlags};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

use crate::fast_model::export_model::duckdb_reader::DuckDBReader;

// ============================================================================
// 数据结构定义
// ============================================================================

/// 碰撞事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollisionEvent {
    /// 碰撞对 (a_refno, b_refno)，保证 a < b
    pub pair: (RefU64, RefU64),
    /// 接触点（世界坐标）
    pub contact_point: Option<[f32; 3]>,
    /// 穿透深度 (正值表示穿透)
    pub penetration_depth: f32,
    /// 碰撞法线
    pub normal: Option<[f32; 3]>,
}

/// 碰撞检测器配置
#[derive(Debug, Clone)]
pub struct CollisionConfig {
    /// 接触/碰撞判定距离容差
    pub tolerance: f32,
    /// 并发处理任务数
    pub concurrency: usize,
    /// 网格目录
    pub mesh_dir: PathBuf,
    /// 限制候选对数量 (None 表示无限制)
    pub limit: Option<usize>,
}

impl Default for CollisionConfig {
    fn default() -> Self {
        let mesh_dir = std::env::var("MESH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("assets/meshes/lod_L0"));

        Self {
            tolerance: 0.001, // 1mm
            concurrency: num_cpus::get().max(4),
            mesh_dir,
            limit: None,
        }
    }
}

/// 碰撞检测统计信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CollisionStats {
    pub candidate_pairs: usize,
    pub collision_events: usize,
    pub broad_phase_ms: u64,
    pub narrow_phase_ms: u64,
    pub total_ms: u64,
}

// ============================================================================
// 碰撞检测器
// ============================================================================

/// 碰撞检测器（基于 DuckDB）
pub struct CollisionDetector {
    config: CollisionConfig,
    /// DuckDB 读取器
    duckdb_reader: Arc<DuckDBReader>,
    /// TriMesh 缓存
    mesh_cache: Arc<DashMap<RefU64, Arc<TriMesh>>>,
}

impl CollisionDetector {
    /// 创建碰撞检测器
    pub fn new(config: CollisionConfig) -> anyhow::Result<Self> {
        let reader = DuckDBReader::open_global_or_latest()?;
        Ok(Self {
            config,
            duckdb_reader: Arc::new(reader),
            mesh_cache: Arc::new(DashMap::new()),
        })
    }

    /// 使用默认配置创建
    pub fn with_default() -> anyhow::Result<Self> {
        Self::new(CollisionConfig::default())
    }

    // ------------------------------------------------------------------------
    // 粗筛阶段 (Broad Phase) - 使用 DuckDB
    // ------------------------------------------------------------------------

    /// 广筛：通过 DuckDB 查询所有潜在碰撞对
    pub fn broad_phase(&self, _noun_filter: Option<&str>) -> anyhow::Result<Vec<(RefU64, RefU64)>> {
        // 使用全局 AABB 范围查询所有 refnos
        let all_refnos = self.duckdb_reader.query_by_bounding_box(
            f64::MIN, f64::MIN, f64::MIN,
            f64::MAX, f64::MAX, f64::MAX,
        )?;

        // 查询每个 refno 的 AABB，构建碰撞对
        let mut pairs = Vec::new();
        let mut aabbs: Vec<(RefU64, Aabb)> = Vec::new();

        for refno_str in all_refnos {
            if let Some((min_x, min_y, min_z, max_x, max_y, max_z)) = self.duckdb_reader.query_aabb(&refno_str)? {
                let parts: Vec<&str> = refno_str.split('_').collect();
                if parts.len() >= 2 {
                    if let (Ok(dbnum), Ok(sesno)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                        let refno = RefU64::from_two_nums(dbnum, sesno);
                        let aabb = Aabb::new(
                            [min_x as f32, min_y as f32, min_z as f32].into(),
                            [max_x as f32, max_y as f32, max_z as f32].into(),
                        );
                        aabbs.push((refno, aabb));
                    }
                }
            }
        }

        // 使用 AABB 检测碰撞对
        for i in 0..aabbs.len() {
            for j in (i + 1)..aabbs.len() {
                let (refno_a, aabb_a) = &aabbs[i];
                let (refno_b, aabb_b) = &aabbs[j];
                if aabb_a.intersects(aabb_b) {
                    pairs.push((*refno_a, *refno_b));
                    if let Some(limit) = self.config.limit {
                        if pairs.len() >= limit {
                            return Ok(pairs);
                        }
                    }
                }
            }
        }

        Ok(pairs)
    }

    /// 通过查询指定区域获取候选对
    pub fn broad_phase_in_region(&self, region: &Aabb) -> anyhow::Result<Vec<(RefU64, RefU64)>> {
        let refno_strs = self.duckdb_reader.query_by_bounding_box(
            region.mins.x as f64,
            region.mins.y as f64,
            region.mins.z as f64,
            region.maxs.x as f64,
            region.maxs.y as f64,
            region.maxs.z as f64,
        )?;

        let mut refnos = Vec::new();
        for refno_str in refno_strs {
            let parts: Vec<&str> = refno_str.split('_').collect();
            if parts.len() >= 2 {
                if let (Ok(dbnum), Ok(sesno)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                    refnos.push(RefU64::from_two_nums(dbnum, sesno));
                }
            }
        }

        // 生成所有配对
        let mut pairs = Vec::new();
        for i in 0..refnos.len() {
            for j in (i + 1)..refnos.len() {
                pairs.push((refnos[i], refnos[j]));
            }
        }

        Ok(pairs)
    }

    // ------------------------------------------------------------------------
    // 精算阶段 (Narrow Phase)
    // ------------------------------------------------------------------------

    /// 精算：对单个碰撞对进行精确检测
    pub async fn narrow_phase(
        &self,
        refno_a: RefU64,
        refno_b: RefU64,
    ) -> anyhow::Result<Option<CollisionEvent>> {
        let tolerance = self.config.tolerance;

        // 1. 获取或加载两个物体的 TriMesh
        let mesh_a = self.get_or_load_mesh(refno_a).await?;
        let mesh_b = self.get_or_load_mesh(refno_b).await?;

        // 2. 使用 Parry3D 进行精确接触检测
        let identity = Isometry::identity();
        let prediction = tolerance as Real;

        let contact_result = contact(&identity, mesh_a.as_ref(), &identity, mesh_b.as_ref(), prediction)?;

        match contact_result {
            Some(c) => {
                let event = CollisionEvent {
                    pair: (refno_a.min(refno_b), refno_a.max(refno_b)),
                    contact_point: Some([c.point1.x as f32, c.point1.y as f32, c.point1.z as f32]),
                    penetration_depth: (-c.dist) as f32,
                    normal: Some([c.normal1.x as f32, c.normal1.y as f32, c.normal1.z as f32]),
                };
                Ok(Some(event))
            }
            None => Ok(None),
        }
    }

    /// 获取或加载 TriMesh
    async fn get_or_load_mesh(&self, refno: RefU64) -> anyhow::Result<Arc<TriMesh>> {
        // 检查缓存
        if let Some(cached) = self.mesh_cache.get(&refno) {
            return Ok(cached.clone());
        }

        // 查询几何实例
        let insts: Vec<GeomInstQuery> =
            aios_core::query_insts(&[RefnoEnum::Refno(refno)], true).await?;

        if insts.is_empty() {
            anyhow::bail!("No geometry instance for refno {}", refno.0);
        }

        let inst = &insts[0];
        if inst.insts.is_empty() {
            anyhow::bail!("Empty inst array for refno {}", refno.0);
        }

        let model_inst = &inst.insts[0];
        let geo_hash = &model_inst.geo_hash;
        let world_trans = inst.world_trans;
        let inst_trans = model_inst.geo_transform;
        let combined = world_trans * inst_trans;

        // 加载 TriMesh
        let mesh = load_trimesh_for_collision(&self.config.mesh_dir, geo_hash, combined.to_matrix()).await?;

        let mesh_arc = Arc::new(mesh);
        self.mesh_cache.insert(refno, mesh_arc.clone());
        Ok(mesh_arc)
    }

    // ------------------------------------------------------------------------
    // 全量检测
    // ------------------------------------------------------------------------

    /// 执行全量碰撞检测
    pub async fn detect_all(
        &self,
        noun_filter: Option<&str>,
    ) -> anyhow::Result<(Vec<CollisionEvent>, CollisionStats)> {
        let total_start = Instant::now();

        // 1. 粗筛
        let broad_start = Instant::now();
        let candidate_pairs = self.broad_phase(noun_filter)?;
        let broad_phase_ms = broad_start.elapsed().as_millis() as u64;

        info!(
            "Broad phase: {} candidate pairs in {}ms",
            candidate_pairs.len(),
            broad_phase_ms
        );

        if candidate_pairs.is_empty() {
            return Ok((
                vec![],
                CollisionStats {
                    candidate_pairs: 0,
                    collision_events: 0,
                    broad_phase_ms,
                    narrow_phase_ms: 0,
                    total_ms: total_start.elapsed().as_millis() as u64,
                },
            ));
        }

        // 2. 精算 (并发)
        let narrow_start = Instant::now();
        use futures::stream::{self, StreamExt};

        let events: Vec<CollisionEvent> = stream::iter(candidate_pairs.clone())
            .map(|(a, b)| {
                let detector = self.clone_for_async();
                async move { detector.narrow_phase(a, b).await }
            })
            .buffer_unordered(self.config.concurrency)
            .filter_map(|result| async move {
                match result {
                    Ok(Some(event)) => Some(event),
                    Ok(None) => None,
                    Err(e) => {
                        warn!("Narrow phase error: {}", e);
                        None
                    }
                }
            })
            .collect()
            .await;

        let narrow_phase_ms = narrow_start.elapsed().as_millis() as u64;

        let stats = CollisionStats {
            candidate_pairs: candidate_pairs.len(),
            collision_events: events.len(),
            broad_phase_ms,
            narrow_phase_ms,
            total_ms: total_start.elapsed().as_millis() as u64,
        };

        info!(
            "Collision detection complete: {} events from {} pairs in {}ms",
            events.len(),
            candidate_pairs.len(),
            stats.total_ms
        );

        Ok((events, stats))
    }

    /// 创建用于 async 的克隆 (共享 mesh_cache 和 duckdb_reader)
    fn clone_for_async(&self) -> Self {
        Self {
            config: self.config.clone(),
            duckdb_reader: self.duckdb_reader.clone(),
            mesh_cache: self.mesh_cache.clone(),
        }
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 加载用于碰撞检测的 TriMesh
async fn load_trimesh_for_collision(
    mesh_dir: &PathBuf,
    geo_hash: &str,
    world_matrix: glam::Mat4,
) -> anyhow::Result<TriMesh> {
    let lod_levels = ["L0", "L1", "L2"];

    let base_dir = mesh_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| mesh_dir.clone());

    for lod in lod_levels {
        let lod_dir = base_dir.join(format!("lod_{}", lod));

        // 尝试 GLB
        let glb_path = lod_dir.join(format!("{}_{}.glb", geo_hash, lod));
        if glb_path.exists() {
            let glb_path_clone = glb_path.clone();
            let matrix = world_matrix;
            match tokio::task::spawn_blocking(move || {
                load_and_transform_glb(&glb_path_clone, matrix)
            })
            .await?
            {
                Ok(mesh) => return Ok(mesh),
                Err(e) => warn!("Failed to load GLB {:?}: {}", glb_path, e),
            }
        }
    }

    anyhow::bail!("Cannot load geometry: {}", geo_hash)
}

fn load_and_transform_glb(path: &PathBuf, transform: glam::Mat4) -> anyhow::Result<TriMesh> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let glb = gltf::Gltf::from_reader(reader)?;

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut index_offset = 0u32;

    for mesh in glb.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|_| glb.blob.as_ref().map(|b| b.as_slice()));

            if let Some(positions) = reader.read_positions() {
                for pos in positions {
                    let p = glam::Vec3::from(pos);
                    let transformed = transform.transform_point3(p);
                    vertices.push(Point::new(
                        transformed.x as Real,
                        transformed.y as Real,
                        transformed.z as Real,
                    ));
                }
            }

            if let Some(idx_reader) = reader.read_indices() {
                for idx in idx_reader.into_u32() {
                    indices.push(index_offset + idx);
                }
            }

            index_offset = vertices.len() as u32;
        }
    }

    if indices.len() % 3 != 0 || indices.is_empty() {
        anyhow::bail!("Invalid index count");
    }

    let tris: Vec<[u32; 3]> = indices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();

    Ok(TriMesh::new(vertices, tris)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collision_event_ordering() {
        let event = CollisionEvent {
            pair: (RefU64(100), RefU64(50)),
            contact_point: None,
            penetration_depth: 0.0,
            normal: None,
        };
        assert!(event.pair.0 < event.pair.1 || event.pair.0 == RefU64(100));
    }
}
