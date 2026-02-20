//! foyer cache 专用的 Manifold 布尔运算实现（不写入 SurrealDB；必要时读取 pe_transform/local_trans）
//!
//! 语义：
//! - 布尔输入优先来自当前 `mesh_precision.default_lod` 对应的常规 GLB；
//! - 按需生成/复用一个 Manifold 友好的共享顶点网格：`assets/meshes/{geo_hash}_m.glb`；
//! - 布尔结果输出回“重复顶点”的 PlantMesh（flat shading 语义），再导出成 GLB，供渲染/导出链路复用。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::{fs, io};

use aios_core::csg::manifold::{ManifoldMeshRust, ManifoldRust};
use aios_core::geometry::csg::{unit_box_mesh, unit_cylinder_mesh, unit_sphere_mesh, UNIT_MESH_SCALE};
use aios_core::geometry::{EleGeosInfo, EleInstGeosData, GeoBasicType};
use aios_core::get_db_option;
use aios_core::mesh_precision::LodMeshSettings;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::RefnoEnum;
use glam::{DMat4, Vec3};

use crate::fast_model::export_model::export_glb::export_single_mesh_to_glb;
use crate::fast_model::{debug_model_debug, debug_model_warn};
use crate::fast_model::instance_cache::InstanceCacheManager;

fn env_bool(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheBoolFrame {
    /// 在正实体自身 local space 做布尔（当前默认实现）
    PosLocal,
    /// 在 world space 做布尔（用于排查/规避坐标系不一致）
    World,
}

fn cache_bool_frame() -> CacheBoolFrame {
    match std::env::var("CACHE_BOOL_FRAME")
        .ok()
        .unwrap_or_else(|| "pos_local".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "world" => CacheBoolFrame::World,
        _ => CacheBoolFrame::PosLocal,
    }
}

fn debug_cache_bool_refno() -> Option<RefnoEnum> {
    let s = std::env::var("DEBUG_CACHE_BOOL_REFNO").ok()?;
    let r = RefnoEnum::from(s.as_str());
    if r.is_valid() {
        Some(r)
    } else {
        None
    }
}

fn fmt_aabb(aabb: &parry3d::bounding_volume::Aabb) -> String {
    format!(
        "min=({:.3},{:.3},{:.3}) max=({:.3},{:.3},{:.3})",
        aabb.mins.x, aabb.mins.y, aabb.mins.z, aabb.maxs.x, aabb.maxs.y, aabb.maxs.z
    )
}

/// NGMR 负实体穿墙投影：当负实体 AABB 在某个轴上完全不与正实体 AABB 相交时，
/// 将负实体沿该轴平移到正实体 AABB 中心，确保布尔差集能正确切出孔洞。
///
/// 背景：NGMR（跨元件负几何关系）中，carrier（如 FITT 管件）的世界坐标原点
/// 通常在 target（如 STWALL 墙体）的穿墙方向上有偏移，导致负实体（切割圆柱）
/// 在 pos local space 中的穿墙轴坐标超出墙体厚度范围，布尔差集无法生效。
fn reproject_neg_to_pos_aabb(
    neg: &ManifoldRust,
    pos_aabb: &parry3d::bounding_volume::Aabb,
    refno: RefnoEnum,
    neg_idx: usize,
) -> Option<ManifoldRust> {
    let neg_mesh = neg.get_mesh();
    let neg_aabb = neg_mesh.cal_aabb()?;

    // 检查每个轴是否相交
    let axes = [
        (neg_aabb.mins.x, neg_aabb.maxs.x, pos_aabb.mins.x, pos_aabb.maxs.x, "X"),
        (neg_aabb.mins.y, neg_aabb.maxs.y, pos_aabb.mins.y, pos_aabb.maxs.y, "Y"),
        (neg_aabb.mins.z, neg_aabb.maxs.z, pos_aabb.mins.z, pos_aabb.maxs.z, "Z"),
    ];

    let mut dx = 0.0f32;
    let mut dy = 0.0f32;
    let mut dz = 0.0f32;
    let mut need_reproject = false;

    for (neg_min, neg_max, pos_min, pos_max, axis_name) in &axes {
        let no_overlap = neg_max < pos_min || neg_min > pos_max;
        if no_overlap {
            // 将负实体中心对齐到正实体中心
            let neg_center = (neg_min + neg_max) * 0.5;
            let pos_center = (pos_min + pos_max) * 0.5;
            let delta = pos_center - neg_center;
            match *axis_name {
                "X" => dx = delta,
                "Y" => dy = delta,
                "Z" => dz = delta,
                _ => {}
            }
            need_reproject = true;
            debug_model_debug!(
                "[BOOL_REPROJECT] refno={} neg[{}] axis={} no_overlap: neg=[{:.1},{:.1}] pos=[{:.1},{:.1}] delta={:.1}",
                refno, neg_idx, axis_name, neg_min, neg_max, pos_min, pos_max, delta
            );
        }
    }

    if !need_reproject {
        return None;
    }

    // 提取顶点，平移，重建 ManifoldRust
    let mut vertices = neg_mesh.vertices.clone();
    for chunk in vertices.chunks_exact_mut(3) {
        chunk[0] += dx;
        chunk[1] += dy;
        chunk[2] += dz;
    }

    let verts3: Vec<Vec3> = vertices
        .chunks(3)
        .map(|v| Vec3::new(v[0], v[1], v[2]))
        .collect();

    let m = ManifoldRust::from_vertices_indices(&verts3, &neg_mesh.indices, DMat4::IDENTITY, true);
    if m.get_mesh().indices.is_empty() {
        debug_model_debug!(
            "[BOOL_REPROJECT] refno={} neg[{}] 投影后 manifold 为空，保留原始",
            refno, neg_idx
        );
        return None;
    }

    debug_model_debug!(
        "[BOOL_REPROJECT] refno={} neg[{}] 投影成功: dx={:.1} dy={:.1} dz={:.1} new_aabb={}",
        refno, neg_idx, dx, dy, dz,
        m.get_mesh().cal_aabb().map(|a| fmt_aabb(&a)).unwrap_or_else(|| "None".to_string())
    );

    Some(m)
}

fn mesh_base_dir() -> PathBuf {
    get_db_option()
        .meshes_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/meshes"))
}

/// 根据 mesh_id 和当前 LOD 配置构建完整的 mesh 文件路径
///
/// 返回：`{base_dir}/lod_{LOD}/{mesh_id}_{LOD}.glb`
fn build_lod_mesh_path(base_dir: &Path, mesh_id: &str) -> PathBuf {
    let default_lod = aios_core::mesh_precision::active_precision().default_lod;

    // 先溯源到不含 lod_ 的基础目录
    let mut clean_base = base_dir.to_path_buf();
    while let Some(last_component) = clean_base.file_name().and_then(|n| n.to_str()) {
        if last_component.starts_with("lod_") {
            clean_base.pop();
        } else {
            break;
        }
    }

    let lod_dir_name = format!("lod_{:?}", default_lod);
    let lod_filename = format!("{}_{:?}.glb", mesh_id, default_lod);
    clean_base.join(lod_dir_name).join(lod_filename)
}

fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// 布尔结果写回路径（与主流程一致，仍写入 default_lod 目录）。
fn boolean_glb_path(mesh_id: &str) -> PathBuf {
    let mut path = build_lod_mesh_path(&mesh_base_dir(), mesh_id);
    path.set_extension("glb");
    path
}

/// 构建 cache-only 布尔专用的 manifold mesh GLB 路径（带 `_m.glb` 后缀）。
///
/// 注意：按用户约定（方案 B），不带 LOD，格式为：`{base_dir}/{geo_hash}_m.glb`。
/// 但其内容语义绑定 `mesh_precision.default_lod` 对应的“常规 GLB”（用于判断是否需要重建）。
fn manifold_glb_path(geo_hash: &str) -> PathBuf {
    let base_dir = mesh_base_dir();

    // 溯源到不含 lod_ 的基础目录
    let mut clean_base = base_dir.clone();
    while let Some(last_component) = clean_base.file_name().and_then(|n| n.to_str()) {
        if last_component.starts_with("lod_") {
            clean_base.pop();
        } else {
            break;
        }
    }

    clean_base.join(format!("{geo_hash}_m.glb"))
}

#[inline]
fn validate_manifold_result(manifold: ManifoldRust, id: &str) -> anyhow::Result<ManifoldRust> {
    let mesh = manifold.get_mesh();
    if mesh.indices.is_empty() {
        return Err(anyhow::anyhow!("Manifold mesh 为空: id={}", id));
    }
    if let Some(aabb) = mesh.cal_aabb() {
        let ext_mag = aabb.extents().magnitude();
        if ext_mag.is_finite() && ext_mag < 1e-6 {
            return Err(anyhow::anyhow!(
                "Manifold mesh 可能为空（哨兵 cube）: id={} ext_mag={:.3e}",
                id,
                ext_mag
            ));
        }
    } else {
        return Err(anyhow::anyhow!("Manifold mesh AABB 无效: id={}", id));
    }
    Ok(manifold)
}

/// 导出 ManifoldRust 为 manifold 格式的 GLB（共享顶点，无法线）
///
/// 与 `ManifoldRust::export_to_glb` 不同，此函数保留共享顶点拓扑，
/// 便于后续布尔运算时直接加载使用
fn export_manifold_mesh_to_glb(manifold: &ManifoldRust, path: &Path) -> anyhow::Result<()> {
    let mesh = manifold.get_mesh();
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        return Err(anyhow::anyhow!("Manifold mesh 为空，无法导出"));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    aios_core::fast_model::export_model::export_glb::export_raw_buffers_to_glb(
        &mesh.vertices,
        &[], // 不包含法线，后续加载时会重新计算
        &mesh.indices,
        path,
    )?;

    Ok(())
}

/// 加载或生成 manifold mesh 的 GLB 文件
///
/// 按需生成：如果 `_m.glb` 文件已存在且未过期则直接加载，否则从当前 `mesh_precision.default_lod`
/// 对应的“常规 GLB”转换并保存；若源 GLB 缺失则回退到 geo_param（避免不必要中断）。
fn load_or_generate_manifold_glb(
    geo_param: &PdmsGeoParam,
    geo_hash: u64,
    mat: DMat4,
    more_precision: bool,
) -> anyhow::Result<ManifoldRust> {
    // 标准单位几何体（1/2/3）始终使用内置生成，不缓存
    if matches!(geo_hash, 1 | 2 | 3) {
        let unit_mesh = match geo_hash {
            1 => unit_box_mesh(),
            2 => unit_cylinder_mesh(&LodMeshSettings::default(), false),
            3 => unit_sphere_mesh(),
            _ => unreachable!(),
        };
        let m = ManifoldRust::from_vertices_indices(&unit_mesh.vertices, &unit_mesh.indices, mat, more_precision);
        return validate_manifold_result(m, &geo_hash.to_string());
    }

    let manifold_path = manifold_glb_path(&geo_hash.to_string());

    // 绑定 default_lod 的“常规 GLB”
    let src_glb = {
        let mut p = build_lod_mesh_path(&mesh_base_dir(), &geo_hash.to_string());
        p.set_extension("glb");
        p
    };

    let force_replace = std::env::var("FORCE_REPLACE_MESH")
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false);

    let need_rebuild = || -> bool {
        if force_replace {
            return true;
        }
        if !manifold_path.exists() {
            return true;
        }
        let Ok(m_meta) = fs::metadata(&manifold_path) else {
            return true;
        };
        let Ok(m_time) = m_meta.modified() else {
            return true;
        };
        if let Ok(src_meta) = fs::metadata(&src_glb) {
            if let Ok(src_time) = src_meta.modified() {
                return m_time < src_time;
            }
        }
        false
    };

    if !need_rebuild() {
        debug_model_debug!(
            "cache_manifold_glb: 复用已有 _m.glb: geo_hash={} path={}",
            geo_hash,
            manifold_path.display()
        );
        return ManifoldRust::import_glb_to_manifold(&manifold_path, mat, more_precision);
    }

    debug_model_debug!(
        "cache_manifold_glb: 重建 _m.glb: geo_hash={} src={} dst={}",
        geo_hash,
        src_glb.display(),
        manifold_path.display()
    );

    let mut manifold_identity = if src_glb.exists() {
        match ManifoldRust::import_glb_to_manifold(&src_glb, DMat4::IDENTITY, false)
            .and_then(|m| validate_manifold_result(m, &geo_hash.to_string()))
        {
            Ok(m) => m,
            Err(e) => {
                debug_model_warn!(
                    "cache_manifold_glb: 从常规 GLB 导入失败，回退 geo_param: geo_hash={} err={}",
                    geo_hash,
                    e
                );
                crate::fast_model::manifold_bool::load_manifold_from_geo_param(
                    geo_param,
                    geo_hash,
                    DMat4::IDENTITY,
                    false,
                )?
            }
        }
    } else {
        crate::fast_model::manifold_bool::load_manifold_from_geo_param(
            geo_param,
            geo_hash,
            DMat4::IDENTITY,
            false,
        )?
    };

    if manifold_identity.get_mesh().indices.is_empty() {
        manifold_identity = if src_glb.exists() {
            ManifoldRust::import_glb_to_manifold(&src_glb, DMat4::IDENTITY, true)
                .and_then(|m| validate_manifold_result(m, &geo_hash.to_string()))?
        } else {
            crate::fast_model::manifold_bool::load_manifold_from_geo_param(
                geo_param,
                geo_hash,
                DMat4::IDENTITY,
                true,
            )?
        };
    }

    ensure_parent_dir(&manifold_path)?;
    if let Err(e) = export_manifold_mesh_to_glb(&manifold_identity, &manifold_path) {
        debug_model_warn!(
            "cache_manifold_glb: 保存 _m.glb 失败，直接返回内存 manifold: geo_hash={} err={}",
            geo_hash,
            e
        );
        let mesh = manifold_identity.get_mesh();
        let verts3: Vec<Vec3> = mesh
            .vertices
            .chunks(3)
            .map(|v| Vec3::new(v[0], v[1], v[2]))
            .collect();
        let m = ManifoldRust::from_vertices_indices(&verts3, &mesh.indices, mat, more_precision);
        return validate_manifold_result(m, &geo_hash.to_string());
    }

    ManifoldRust::import_glb_to_manifold(&manifold_path, mat, more_precision)
}

/// 将 Manifold 的共享顶点 mesh 转为“重复顶点”的 PlantMesh（flat shading 语义）。
///
/// - ManifoldMeshRust: vertices 为 [x,y,z,...] 扁平数组，indices 为三角形索引
/// - 输出 PlantMesh：每个三角形 3 个独立顶点（不共享），indices 顺序递增
fn manifold_to_normal_mesh(mesh: ManifoldMeshRust) -> PlantMesh {
    let tri_count = mesh.indices.len() / 3;

    let mut out = PlantMesh::default();
    out.vertices.reserve(tri_count * 3);
    out.normals.reserve(tri_count * 3);
    out.uvs.reserve(tri_count * 3);
    out.indices.reserve(tri_count * 3);

    let get_v = |idx: u32| -> Vec3 {
        let base = idx as usize * 3;
        if base + 2 >= mesh.vertices.len() {
            return Vec3::ZERO;
        }
        Vec3::new(mesh.vertices[base], mesh.vertices[base + 1], mesh.vertices[base + 2])
    };

    for tri in mesh.indices.chunks(3) {
        if tri.len() != 3 {
            break;
        }
        let v0 = get_v(tri[0]);
        let v1 = get_v(tri[1]);
        let v2 = get_v(tri[2]);

        let face_n = (v1 - v0).cross(v2 - v0);
        let n = if face_n.length_squared() > 1e-10 {
            face_n.normalize()
        } else {
            Vec3::Y
        };

        let base = out.vertices.len() as u32;
        out.vertices.extend_from_slice(&[v0, v1, v2]);
        out.normals.extend_from_slice(&[n, n, n]);
        out.uvs.extend_from_slice(&[[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]);
        out.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    out
}

#[inline]
fn log_load_manifold_failed(scene: &str, refno: RefnoEnum, mesh_id: &str, err: &anyhow::Error) {
    eprintln!(
        "[bool][{}] load_manifold 失败: refno={} mesh_id={} err={}",
        scene, refno, mesh_id, err
    );
}

/// 基于 foyer 缓存的布尔运算（不访问 SurrealDB）
///
/// # 参数
/// - `cache_manager`: 实例缓存管理器
/// - `filter_refnos`: 可选的 refno 过滤集合，仅处理该集合内的 refno（用于 debug_model 模式）
pub async fn run_boolean_worker_from_cache_manager(
    cache_manager: &InstanceCacheManager,
    filter_refnos: Option<&HashSet<RefnoEnum>>,
) -> anyhow::Result<usize> {
    fn is_source_level_manifold_error(err: &anyhow::Error) -> bool {
        let msg = err.to_string();
        msg.contains("Manifold mesh 为空")
            || msg.contains("No such file or directory")
            || msg.contains("系统找不到指定的文件")
            || msg.contains("(os error 2)")
    }

    fn aabb_contains(
        outer: &parry3d::bounding_volume::Aabb,
        inner: &parry3d::bounding_volume::Aabb,
    ) -> bool {
        outer.mins.x <= inner.mins.x
            && outer.mins.y <= inner.mins.y
            && outer.mins.z <= inner.mins.z
            && outer.maxs.x >= inner.maxs.x
            && outer.maxs.y >= inner.maxs.y
            && outer.maxs.z >= inner.maxs.z
    }

    fn aabb_intersects(a: &parry3d::bounding_volume::Aabb, b: &parry3d::bounding_volume::Aabb) -> bool {
        !(a.maxs.x < b.mins.x
            || a.mins.x > b.maxs.x
            || a.maxs.y < b.mins.y
            || a.mins.y > b.maxs.y
            || a.maxs.z < b.mins.z
            || a.mins.z > b.maxs.z)
    }

    fn is_pos_geo_for_boolean(t: &GeoBasicType) -> bool {
        // cache bool_worker 应对齐导出语义：正实体=可见的 Pos/Compound（以及可能出现的 DesiPos/CatePos）
        matches!(
            t,
            &GeoBasicType::Pos
                | &GeoBasicType::Compound
                | &GeoBasicType::DesiPos
                | &GeoBasicType::CatePos
        )
    }

    fn local_mat_for_inst(inst: &aios_core::geometry::EleInstGeo) -> DMat4 {
        // inst.geo_transform 是 carrier 局部坐标；unit mesh 需按约定缩放修正。
        let mut tf = inst.geo_transform;
        if inst.geo_param.is_reuse_unit() {
            tf.scale /= UNIT_MESH_SCALE;
        }
        tf.to_matrix().as_dmat4()
    }

    fn world_mat_for_info(info: &EleGeosInfo) -> DMat4 {
        // 统一走 EleGeosInfo 的封装：避免部分 cache-only 数据里 `world_transform` 字段
        // 不是“最终世界变换”（例如需要补齐 owner 链/策略计算/transform_cache 命中）时，
        // NGMR/NEG 的相对变换计算错位，导致差集无效（典型：孔洞没有被切出来）。
        info.get_ele_world_transform().to_matrix().as_dmat4()
    }

    fn diff_with_guards(
        mut pos_union: ManifoldRust,
        negs: &[ManifoldRust],
        refno: RefnoEnum,
        label: &str,
    ) -> ManifoldRust {
        if negs.is_empty() {
            return pos_union;
        }

        for (i, neg) in negs.iter().enumerate() {
            if pos_union.get_mesh().indices.is_empty() {
                break;
            }

            let before = pos_union.clone();
            let before_aabb = before.get_mesh().cal_aabb();
            let neg_aabb = neg.get_mesh().cal_aabb();

            let mut after = before.clone();
            after.inner = after.inner.difference(&neg.inner);

            if after.get_mesh().indices.is_empty() {
                match (&before_aabb, &neg_aabb) {
                    (Some(before_aabb), Some(neg_aabb)) => {
                        let intersects = aabb_intersects(before_aabb, neg_aabb);
                        let contains = aabb_contains(neg_aabb, before_aabb);
                        // 经验：差集结果被异常清空时，跳过该负实体，避免“整块消失”。
                        if !intersects || !contains {
                            eprintln!(
                                "[bool][cache] ⚠️({}) 差集结果被异常清空，跳过该负实体: refno={} neg_idx={} intersects={} contains={}",
                                label, refno, i, intersects, contains
                            );
                            pos_union = before;
                            continue;
                        }
                    }
                    _ => {
                        eprintln!(
                            "[bool][cache] ⚠️({}) 差集结果被清空且无法计算 AABB，跳过该负实体: refno={} neg_idx={}",
                            label, refno, i
                        );
                        pos_union = before;
                        continue;
                    }
                }
            }

            pos_union = after;
        }

        // 兜底：若逐个 subtract 仍退化为空，尝试 union-neg 再做差（某些情况下更稳定）
        if pos_union.get_mesh().indices.is_empty() {
            let neg_union =
                ManifoldRust::batch_boolean(negs, aios_core::csg::manifold::ManifoldOpType::Union);
            let mut union_diff = pos_union.clone();
            union_diff.inner = union_diff.inner.difference(&neg_union.inner);
            if !union_diff.get_mesh().indices.is_empty() {
                return union_diff;
            }
        }

        pos_union
    }

    let dbnums = cache_manager.list_dbnums();
    if dbnums.is_empty() {
        println!("[boolean_worker_cache] 缓存为空，跳过布尔运算");
        return Ok(0);
    }

    // ── 阶段 1：从“布尔目标索引”收集目标（不做全量扫描，不加载 inst_geos） ──
    let mut inst_info_map: HashMap<RefnoEnum, EleGeosInfo> = HashMap::new();
    let mut neg_relate_map: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::new();
    let mut ngmr_relate_map: HashMap<RefnoEnum, Vec<(RefnoEnum, RefnoEnum)>> = HashMap::new();
    let mut target_locations: HashMap<RefnoEnum, u32> = HashMap::new();
    let mut cata_targets: HashSet<RefnoEnum> = HashSet::new();
    // 记录每个 refno 的 inst_key，阶段 2 按需加载 geos
    let mut refno_inst_keys: HashMap<RefnoEnum, String> = HashMap::new();
    let mut indexed_targets = 0usize;

    for dbnum in &dbnums {
        let refnos = cache_manager.list_boolean_targets(*dbnum);
        indexed_targets += refnos.len();
        for refno in refnos {
            let Some(cached) = cache_manager.get_inst_info(*dbnum, refno).await else {
                continue;
            };

            target_locations.entry(refno).or_insert(*dbnum);
            if cached.info.has_cata_neg {
                cata_targets.insert(refno);
            }
            if !cached.neg_relates.is_empty() {
                for carrier in &cached.neg_relates {
                    target_locations.entry(*carrier).or_insert(*dbnum);
                }
            }
            if !cached.ngmr_neg_relates.is_empty() {
                for (carrier, ngmr_geom_refno) in &cached.ngmr_neg_relates {
                    target_locations.entry(*carrier).or_insert(*dbnum);
                    target_locations.entry(*ngmr_geom_refno).or_insert(*dbnum);
                }
            }

            refno_inst_keys.insert(refno, cached.inst_key);
            inst_info_map.entry(refno).or_insert(cached.info);
            if !cached.neg_relates.is_empty() {
                neg_relate_map.entry(refno).or_insert(cached.neg_relates);
            }
            if !cached.ngmr_neg_relates.is_empty() {
                ngmr_relate_map.entry(refno).or_insert(cached.ngmr_neg_relates);
            }
        }
    }

    if indexed_targets == 0 {
        println!(
            "[boolean_worker_cache] 布尔运算完成: 0 个（无布尔目标索引，跳过）"
        );
        return Ok(0);
    }

    let mut targets: HashSet<RefnoEnum> = HashSet::new();
    targets.extend(cata_targets.iter().copied());
    targets.extend(
        neg_relate_map
            .keys()
            .copied()
            .filter(|r| inst_info_map.contains_key(r)),
    );
    targets.extend(
        ngmr_relate_map
            .keys()
            .copied()
            .filter(|r| inst_info_map.contains_key(r)),
    );

    if let Some(filter_set) = filter_refnos {
        let before_count = targets.len();
        targets.retain(|r| filter_set.contains(r));
        if before_count != targets.len() {
            println!(
                "[boolean_worker_cache] debug_model 过滤: {} -> {} 个目标",
                before_count,
                targets.len()
            );
        }
    }

    if targets.is_empty() {
        println!("[boolean_worker_cache] 布尔运算完成: 0 个（无布尔目标，跳过 geos 加载）");
        return Ok(0);
    }
    let total_targets = targets.len();

    // ── 阶段 2：仅为有布尔目标的 refno 加载 inst_geos ──
    let mut inst_geos_map: HashMap<String, EleInstGeosData> = HashMap::new();
    // 收集所有需要的 inst_key（targets + 它们的 neg_relate 引用的 refno）
    let mut needed_refnos: HashSet<RefnoEnum> = targets.clone();
    for refno in &targets {
        if let Some(negs) = neg_relate_map.get(refno) {
            needed_refnos.extend(negs.iter().copied());
        }
        if let Some(ngmrs) = ngmr_relate_map.get(refno) {
            for (a, b) in ngmrs {
                needed_refnos.insert(*a);
                needed_refnos.insert(*b);
            }
        }
    }
    for refno in &needed_refnos {
        let Some(inst_key) = refno_inst_keys.get(refno) else {
            continue;
        };
        if inst_geos_map.contains_key(inst_key) {
            continue;
        }
        let dbnum = target_locations.get(refno).copied()
            .or_else(|| dbnums.first().copied())
            .unwrap_or(0);
        if let Some(geos) = cache_manager.get_inst_geos(dbnum, inst_key).await {
            inst_geos_map.insert(inst_key.clone(), geos.geos_data);
        }
    }

    let mut processed = 0usize;
    let heartbeat_secs = std::env::var("AIOS_BOOL_PROGRESS_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(15);
    let bool_start = std::time::Instant::now();
    let mut last_heartbeat = bool_start;
    println!(
        "[boolean_worker_cache] 布尔任务开始: targets={}, heartbeat={}s",
        total_targets, heartbeat_secs
    );
    // 缓存“源级失败”的 geo_hash（文件缺失/导入后空网格），避免在不同 refno 上重复高成本重试。
    let mut source_failed_geo_hashes: HashMap<u64, String> = HashMap::new();
    let mut source_failed_geo_hashes_warned: HashSet<u64> = HashSet::new();

    let mut try_load_manifold = |scene: &str,
                                 refno: RefnoEnum,
                                 geo_param: &PdmsGeoParam,
                                 geo_hash: u64,
                                 mat: DMat4,
                                 more_precision: bool|
     -> Option<ManifoldRust> {
        if let Some(cached_reason) = source_failed_geo_hashes.get(&geo_hash) {
            if source_failed_geo_hashes_warned.insert(geo_hash) {
                eprintln!(
                    "[bool][{}] 跳过已知失败几何: refno={} mesh_id={} reason={}",
                    scene, refno, geo_hash, cached_reason
                );
            }
            return None;
        }

        match load_or_generate_manifold_glb(geo_param, geo_hash, mat, more_precision) {
            Ok(m) => Some(m),
            Err(e) => {
                if is_source_level_manifold_error(&e) {
                    source_failed_geo_hashes.insert(geo_hash, e.to_string());
                }
                log_load_manifold_failed(scene, refno, &geo_hash.to_string(), &e);
                None
            }
        }
    };

    let mut maybe_heartbeat = |refno: RefnoEnum, stage: &str, processed_now: usize| {
        let now = std::time::Instant::now();
        if now.duration_since(last_heartbeat).as_secs() >= heartbeat_secs {
            println!(
                "[boolean_worker_cache] 心跳: processed={}/{} current_refno={} stage={} elapsed={}s",
                processed_now,
                total_targets,
                refno,
                stage,
                bool_start.elapsed().as_secs()
            );
            last_heartbeat = now;
        }
    };

    for refno in targets {
        maybe_heartbeat(refno, "target_start", processed);
        let Some(info) = inst_info_map.get(&refno) else {
            continue;
        };
        let inst_key = info.get_inst_key();
        let Some(inst_geos) = inst_geos_map.get(&inst_key) else {
            continue;
        };

        let debug_refno = debug_cache_bool_refno();
        let debug_this = debug_refno.is_some_and(|r| r == refno);
        let debug_dump = debug_this && env_bool("DEBUG_CACHE_BOOL_DUMP");
        let frame = cache_bool_frame();

        let pos_world_mat = world_mat_for_info(info);
        let inverse_pos_world = pos_world_mat.inverse();

        debug_model_debug!(
            "[BOOL_WORLD] refno={} pos_world_mat:\n  col0=({:.3},{:.3},{:.3},{:.3})\n  col1=({:.3},{:.3},{:.3},{:.3})\n  col2=({:.3},{:.3},{:.3},{:.3})\n  col3=({:.3},{:.3},{:.3},{:.3})",
            refno,
            pos_world_mat.x_axis.x, pos_world_mat.x_axis.y, pos_world_mat.x_axis.z, pos_world_mat.x_axis.w,
            pos_world_mat.y_axis.x, pos_world_mat.y_axis.y, pos_world_mat.y_axis.z, pos_world_mat.y_axis.w,
            pos_world_mat.z_axis.x, pos_world_mat.z_axis.y, pos_world_mat.z_axis.z, pos_world_mat.z_axis.w,
            pos_world_mat.w_axis.x, pos_world_mat.w_axis.y, pos_world_mat.w_axis.z, pos_world_mat.w_axis.w
        );

        // 正实体：使用局部变换（pos local space）加载，优先复用 _m.glb 缓存
        let mut pos_manifolds = Vec::new();
        // 记录第一个 Pos 几何体的 geo_local_mat，用于将负实体也映射到同一坐标空间。
        // 背景：正实体 mesh 以 geo_transform 空间加载（含 unit mesh 缩放），
        // 而负实体原先以 inverse(world_transform) 空间加载，两者不一致。
        // 修正：负实体也需要经过 inverse(pos_geo_local_mat) 映射到 geo_transform 空间。
        let mut pos_geo_local_mat: Option<DMat4> = None;
        for inst in &inst_geos.insts {
            if !is_pos_geo_for_boolean(&inst.geo_type) {
                continue;
            }
            let mat_local = local_mat_for_inst(inst);
            if pos_geo_local_mat.is_none() {
                pos_geo_local_mat = Some(mat_local);
            }
            let mat = if frame == CacheBoolFrame::World {
                pos_world_mat * mat_local
            } else {
                mat_local
            };
            debug_model_debug!(
                "[BOOL_POS_TF] refno={} geo_hash={} geo_type={:?} unit_flag={} geo_transform.t=({:.3},{:.3},{:.3}) geo_transform.s=({:.3},{:.3},{:.3}) mat_local.col3=({:.3},{:.3},{:.3}) mat_local.col0_len={:.3} mat_local.col1_len={:.3} mat_local.col2_len={:.3}",
                refno, inst.geo_hash, inst.geo_type, inst.geo_param.is_reuse_unit(),
                inst.geo_transform.translation.x, inst.geo_transform.translation.y, inst.geo_transform.translation.z,
                inst.geo_transform.scale.x, inst.geo_transform.scale.y, inst.geo_transform.scale.z,
                mat_local.w_axis.x, mat_local.w_axis.y, mat_local.w_axis.z,
                mat_local.x_axis.length(), mat_local.y_axis.length(), mat_local.z_axis.length()
            );
            maybe_heartbeat(refno, "cache_pos", processed);
            if let Some(m) =
                try_load_manifold("cache_pos", refno, &inst.geo_param, inst.geo_hash, mat, false)
            {
                pos_manifolds.push(m);
            }
        }
        if pos_manifolds.is_empty() {
            continue;
        }

        // 计算从 world_transform 局部空间到 geo_transform 空间的映射。
        // 当 pos_geo_local_mat ≠ identity 时（如 unit mesh 缩放），
        // 负实体需要额外经过 inverse(pos_geo_local_mat) 才能与正实体在同一空间。
        let inverse_pos_geo_local = pos_geo_local_mat
            .map(|m| m.inverse())
            .unwrap_or(DMat4::IDENTITY);

        if debug_this {
            eprintln!(
                "[bool][cache][dbg] target={} type={} owner={} world.t=({:.3},{:.3},{:.3})",
                refno,
                inst_geos.type_name,
                info.owner_refno,
                info.world_transform.translation.x,
                info.world_transform.translation.y,
                info.world_transform.translation.z
            );
            eprintln!("[bool][cache][dbg] frame={:?}", frame);
        }

        // 负实体：通过关系表（neg_relate/ngmr_relate）定位切割几何，并转换到 pos local space
        let mut neg_manifolds: Vec<ManifoldRust> = Vec::new();

        // 元件库（CATE）“本体孔洞”负实体：只应包含 CataNeg。
        if info.has_cata_neg {
            for inst in &inst_geos.insts {
                if inst.geo_type != GeoBasicType::CataNeg {
                    continue;
                }
                let mat_local = local_mat_for_inst(inst);
                let mat = if frame == CacheBoolFrame::World {
                    pos_world_mat * mat_local
                } else {
                    mat_local
                };
                maybe_heartbeat(refno, "cache_cata_neg", processed);
                if let Some(m) = try_load_manifold(
                    "cache_cata_neg",
                    refno,
                    &inst.geo_param,
                    inst.geo_hash,
                    mat,
                    true,
                ) {
                    neg_manifolds.push(m);
                }
            }
        }

        if let Some(carriers) = neg_relate_map.get(&refno) {
            let mut uniq_carriers: HashSet<RefnoEnum> = HashSet::new();
            uniq_carriers.extend(carriers.iter().copied());
            for carrier_refno in uniq_carriers {
                let Some(carrier_info) = inst_info_map.get(&carrier_refno) else {
                    continue;
                };
                let carrier_key = carrier_info.get_inst_key();
                let Some(carrier_geos) = inst_geos_map.get(&carrier_key) else {
                    continue;
                };
                let carrier_world_mat = world_mat_for_info(carrier_info);

                for inst in &carrier_geos.insts {
                    if inst.geo_type != GeoBasicType::Neg {
                        continue;
                    }

                    let geo_local_mat = local_mat_for_inst(inst);

                    // 计算负实体在正实体 geo_transform 空间中的变换：
                    // neg_world = carrier_world * geo_local
                    // mat = inverse(pos_geo_local) * inverse(pos_world) * neg_world
                    // 其中 inverse(pos_geo_local) 将 world_transform 局部空间映射到 geo_transform 空间，
                    // 确保负实体与正实体在同一坐标空间（含 unit mesh 缩放）。
                    let neg_world_mat = carrier_world_mat * geo_local_mat;
                    let mat = if frame == CacheBoolFrame::World {
                        neg_world_mat
                    } else {
                        inverse_pos_geo_local * inverse_pos_world * neg_world_mat
                    };
                    maybe_heartbeat(refno, "cache_neg", processed);
                    if let Some(m) = try_load_manifold(
                        "cache_neg",
                        refno,
                        &inst.geo_param,
                        inst.geo_hash,
                        mat,
                        true,
                    ) {
                        neg_manifolds.push(m);
                    }
                }
            }
        }

        if let Some(pairs) = ngmr_relate_map.get(&refno) {
            let mut uniq_pairs: HashSet<(RefnoEnum, RefnoEnum)> = HashSet::new();
            uniq_pairs.extend(pairs.iter().copied());
            for (carrier_refno, ngmr_geom_refno) in uniq_pairs {
                let Some(carrier_info) = inst_info_map.get(&carrier_refno) else {
                    continue;
                };
                let carrier_key = carrier_info.get_inst_key();
                let Some(carrier_geos) = inst_geos_map.get(&carrier_key) else {
                    continue;
                };
                let carrier_world_mat = world_mat_for_info(carrier_info);

                for inst in &carrier_geos.insts {
                    if inst.geo_type != GeoBasicType::CataCrossNeg {
                        continue;
                    }
                    // CataCrossNeg 在缓存中按 geom_refno（即 ngmr_geom_refno）区分
                    if inst.refno != ngmr_geom_refno {
                        continue;
                    }
                    let geo_local_mat = local_mat_for_inst(inst);

                    // 计算负实体在正实体 geo_transform 空间中的变换：
                    // neg_world = carrier_world * geo_local
                    // mat = inverse(pos_geo_local) * inverse(pos_world) * neg_world
                    let neg_world_mat = carrier_world_mat * geo_local_mat;
                    let mat = if frame == CacheBoolFrame::World {
                        neg_world_mat
                    } else {
                        inverse_pos_geo_local * inverse_pos_world * neg_world_mat
                    };
                    // 临时调试：打印 NGMR 变换信息
                    debug_model_debug!(
                        "[NGMR_BOOL_DBG] target={} carrier={} geo_local.t=({:.3},{:.3},{:.3}) mat.t=({:.3},{:.3},{:.3})",
                        refno, carrier_refno,
                        geo_local_mat.w_axis.x, geo_local_mat.w_axis.y, geo_local_mat.w_axis.z,
                        mat.w_axis.x, mat.w_axis.y, mat.w_axis.z
                    );
                    // 打印 pos_world 和 carrier_world 信息
                    debug_model_debug!(
                        "[NGMR_BOOL_DBG] pos_world.t=({:.3},{:.3},{:.3}) carrier_world.t=({:.3},{:.3},{:.3})",
                        pos_world_mat.w_axis.x, pos_world_mat.w_axis.y, pos_world_mat.w_axis.z,
                        carrier_world_mat.w_axis.x, carrier_world_mat.w_axis.y, carrier_world_mat.w_axis.z
                    );
                    // 打印 carrier 完整世界矩阵（旋转+平移），用于精确推导坐标变换
                    debug_model_debug!(
                        "[NGMR_BOOL_DBG] carrier_world_mat:\n  col0=({:.3},{:.3},{:.3},{:.3})\n  col1=({:.3},{:.3},{:.3},{:.3})\n  col2=({:.3},{:.3},{:.3},{:.3})\n  col3=({:.3},{:.3},{:.3},{:.3})",
                        carrier_world_mat.x_axis.x, carrier_world_mat.x_axis.y, carrier_world_mat.x_axis.z, carrier_world_mat.x_axis.w,
                        carrier_world_mat.y_axis.x, carrier_world_mat.y_axis.y, carrier_world_mat.y_axis.z, carrier_world_mat.y_axis.w,
                        carrier_world_mat.z_axis.x, carrier_world_mat.z_axis.y, carrier_world_mat.z_axis.z, carrier_world_mat.z_axis.w,
                        carrier_world_mat.w_axis.x, carrier_world_mat.w_axis.y, carrier_world_mat.w_axis.z, carrier_world_mat.w_axis.w
                    );
                    // 打印 neg_world_mat 完整矩阵
                    debug_model_debug!(
                        "[NGMR_BOOL_DBG] neg_world_mat:\n  col0=({:.3},{:.3},{:.3},{:.3})\n  col1=({:.3},{:.3},{:.3},{:.3})\n  col2=({:.3},{:.3},{:.3},{:.3})\n  col3=({:.3},{:.3},{:.3},{:.3})",
                        neg_world_mat.x_axis.x, neg_world_mat.x_axis.y, neg_world_mat.x_axis.z, neg_world_mat.x_axis.w,
                        neg_world_mat.y_axis.x, neg_world_mat.y_axis.y, neg_world_mat.y_axis.z, neg_world_mat.y_axis.w,
                        neg_world_mat.z_axis.x, neg_world_mat.z_axis.y, neg_world_mat.z_axis.z, neg_world_mat.z_axis.w,
                        neg_world_mat.w_axis.x, neg_world_mat.w_axis.y, neg_world_mat.w_axis.z, neg_world_mat.w_axis.w
                    );
                    // 打印最终 mat 完整矩阵
                    debug_model_debug!(
                        "[NGMR_BOOL_DBG] final_mat:\n  col0=({:.3},{:.3},{:.3},{:.3})\n  col1=({:.3},{:.3},{:.3},{:.3})\n  col2=({:.3},{:.3},{:.3},{:.3})\n  col3=({:.3},{:.3},{:.3},{:.3})",
                        mat.x_axis.x, mat.x_axis.y, mat.x_axis.z, mat.x_axis.w,
                        mat.y_axis.x, mat.y_axis.y, mat.y_axis.z, mat.y_axis.w,
                        mat.z_axis.x, mat.z_axis.y, mat.z_axis.z, mat.z_axis.w,
                        mat.w_axis.x, mat.w_axis.y, mat.w_axis.z, mat.w_axis.w
                    );
                    if debug_this {
                        eprintln!(
                            "[bool][cache][dbg]  ngmr carrier={} carrier.type={} carrier.world.t=({:.3},{:.3},{:.3}) geom_refno={} geo_hash={} inst.local.t=({:.3},{:.3},{:.3}) rel.t=({:.3},{:.3},{:.3})",
                            carrier_refno,
                            carrier_geos.type_name,
                            carrier_info.world_transform.translation.x,
                            carrier_info.world_transform.translation.y,
                            carrier_info.world_transform.translation.z,
                            ngmr_geom_refno,
                            inst.geo_hash,
                            inst.geo_transform.translation.x,
                            inst.geo_transform.translation.y,
                            inst.geo_transform.translation.z,
                            mat.w_axis.x,
                            mat.w_axis.y,
                            mat.w_axis.z
                        );
                    }
                    maybe_heartbeat(refno, "cache_ngmr", processed);
                    if let Some(m) = try_load_manifold(
                        "cache_ngmr",
                        refno,
                        &inst.geo_param,
                        inst.geo_hash,
                        mat,
                        true,
                    ) {
                        if debug_this {
                            if let Some(aabb) = m.get_mesh().cal_aabb() {
                                eprintln!(
                                    "[bool][cache][dbg]   ngmr manifold_aabb: {}",
                                    fmt_aabb(&aabb)
                                );
                            }
                        }
                        if debug_dump {
                            let mut path = PathBuf::from("test_output/cache_bool_debug");
                            let _ = fs::create_dir_all(&path);
                            path.push(format!(
                                "{}_ngmr_{}_{}.glb",
                                refno.to_string(),
                                carrier_refno.to_string(),
                                inst.refno.to_string()
                            ));
                            let mesh = manifold_to_normal_mesh(m.get_mesh());
                            let _ = export_single_mesh_to_glb(&mesh, &path);
                        }
                        neg_manifolds.push(m);
                    }
                }
            }
        }

        if neg_manifolds.is_empty() {
            continue;
        }

        let pos_union = ManifoldRust::batch_boolean(
            &pos_manifolds,
            aios_core::csg::manifold::ManifoldOpType::Union,
        );

        // 始终打印 AABB 调试信息，用于排查布尔运算坐标空间问题
        {
            let pos_mesh = pos_union.get_mesh();
            debug_model_debug!(
                "[BOOL_AABB] refno={} pos_union: verts={} tris={} aabb={}",
                refno,
                pos_mesh.vertices.len() / 3,
                pos_mesh.indices.len() / 3,
                pos_mesh.cal_aabb().map(|a| fmt_aabb(&a)).unwrap_or_else(|| "None".to_string())
            );
            for (i, neg) in neg_manifolds.iter().enumerate() {
                let neg_mesh = neg.get_mesh();
                debug_model_debug!(
                    "[BOOL_AABB] refno={} neg[{}]: verts={} tris={} aabb={}",
                    refno, i,
                    neg_mesh.vertices.len() / 3,
                    neg_mesh.indices.len() / 3,
                    neg_mesh.cal_aabb().map(|a| fmt_aabb(&a)).unwrap_or_else(|| "None".to_string())
                );
            }
        }

        if debug_this {
            if let Some(aabb) = pos_union.get_mesh().cal_aabb() {
                eprintln!("[bool][cache][dbg] pos_union_aabb: {}", fmt_aabb(&aabb));
            }
            eprintln!(
                "[bool][cache][dbg] neg_count={} (includes cata_neg/neg/ngmr)",
                neg_manifolds.len()
            );
            if debug_dump {
                let mut path = PathBuf::from("test_output/cache_bool_debug");
                let _ = fs::create_dir_all(&path);
                path.push(format!("{}_pos_union.glb", refno.to_string()));
                let mesh = manifold_to_normal_mesh(pos_union.get_mesh());
                let _ = export_single_mesh_to_glb(&mesh, &path);
            }
        }

        // NGMR 穿墙投影：检测不与正实体 AABB 相交的负实体，沿不相交轴平移到正实体中心。
        // 典型场景：STWALL 墙体厚度 100mm（pos local Z=0..100），FITT 管件的 NGMR 负圆柱
        // 因 carrier 世界坐标偏移导致 Z 远超 0..100 范围，布尔差集无法切出孔洞。
        if let Some(pos_aabb) = pos_union.get_mesh().cal_aabb() {
            for (i, neg) in neg_manifolds.iter_mut().enumerate() {
                if let Some(reprojected) = reproject_neg_to_pos_aabb(neg, &pos_aabb, refno, i) {
                    *neg = reprojected;
                }
            }
        }

        let mut final_manifold = diff_with_guards(pos_union, &neg_manifolds, refno, "lo");

        // 经验：退化为空时尝试高精度重算一次
        if final_manifold.get_mesh().indices.is_empty() {
            let mut pos_hi = Vec::new();
            for inst in &inst_geos.insts {
                if !is_pos_geo_for_boolean(&inst.geo_type) {
                    continue;
                }
                let mat = local_mat_for_inst(inst);
                maybe_heartbeat(refno, "cache_pos_hi", processed);
                if let Some(m) = try_load_manifold(
                    "cache_pos_hi",
                    refno,
                    &inst.geo_param,
                    inst.geo_hash,
                    mat,
                    true,
                ) {
                    pos_hi.push(m);
                }
            }
            if !pos_hi.is_empty() {
                let pos_union_hi = ManifoldRust::batch_boolean(
                    &pos_hi,
                    aios_core::csg::manifold::ManifoldOpType::Union,
                );
                final_manifold = diff_with_guards(pos_union_hi, &neg_manifolds, refno, "hi");
            }
        }

        // 打印最终布尔结果的 AABB，用于对比是否真正发生了差集
        {
            let final_mesh = final_manifold.get_mesh();
            debug_model_debug!(
                "[BOOL_AABB] refno={} final: verts={} tris={} aabb={}",
                refno,
                final_mesh.vertices.len() / 3,
                final_mesh.indices.len() / 3,
                final_mesh.cal_aabb().map(|a| fmt_aabb(&a)).unwrap_or_else(|| "None".to_string())
            );
        }

        if debug_dump {
            let mut path = PathBuf::from("test_output/cache_bool_debug");
            let _ = fs::create_dir_all(&path);
            path.push(format!("{}_final.glb", refno.to_string()));
            let mesh = manifold_to_normal_mesh(final_manifold.get_mesh());
            let _ = export_single_mesh_to_glb(&mesh, &path);
        }

        if final_manifold.get_mesh().indices.is_empty() {
            eprintln!("[boolean_worker_cache] 结果为空，跳过输出: refno={}", refno);
            if let Some(&dbnum) = target_locations.get(&refno) {
                let mesh_id = {
                    let refu64: aios_core::RefU64 = refno.into();
                    refu64.to_string()
                };
                let _ = cache_manager
                    .upsert_inst_relate_bool(dbnum, refno, mesh_id, "Failed")
                    .await;
            }
            continue;
        }

        let mesh_id = {
            let refu64: aios_core::RefU64 = refno.into();
            refu64.to_string()
        };
        let target_path = boolean_glb_path(&mesh_id);
        ensure_parent_dir(&target_path)?;

        if debug_this {
            if let Some(aabb) = final_manifold.get_mesh().cal_aabb() {
                eprintln!("[bool][cache][dbg] final_aabb: {}", fmt_aabb(&aabb));
            }
        }

        // cache-only：布尔结果先保持 Manifold 拓扑计算的正确性，再转换回“重复顶点”的 PlantMesh，
        // 以匹配渲染/导出侧对硬表面（flat shading）与非共享顶点的默认语义。
        let normal_mesh = manifold_to_normal_mesh(final_manifold.get_mesh());
        if let Err(e) = export_single_mesh_to_glb(&normal_mesh, &target_path) {
            eprintln!("[boolean_worker_cache] 导出失败: refno={} err={}", refno, e);
            if let Some(&dbnum) = target_locations.get(&refno) {
                let _ = cache_manager
                    .upsert_inst_relate_bool(dbnum, refno, mesh_id.clone(), "Failed")
                    .await;
            }
            continue;
        }

        if let Some(&dbnum) = target_locations.get(&refno) {
            println!(
                "[boolean_worker_cache] 写入 bool Success: refno={} dbnum={} mesh_id={}",
                refno, dbnum, mesh_id
            );
            cache_manager
                .upsert_inst_relate_bool(dbnum, refno, mesh_id, "Success")
                .await?;
        } else {
            eprintln!(
                "[boolean_worker_cache] ⚠️ target_locations 中找不到 refno={}，无法写入 bool 结果！",
                refno
            );
        }

        processed += 1;
    }

    println!("[boolean_worker_cache] 布尔运算完成: {} 个", processed);
    Ok(processed)
}
