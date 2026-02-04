//! foyer cache 专用的 Manifold 布尔运算实现（不访问 SurrealDB）
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

    let mut inst_info_map: HashMap<RefnoEnum, EleGeosInfo> = HashMap::new();
    let mut inst_geos_map: HashMap<String, EleInstGeosData> = HashMap::new();
    let mut neg_relate_map: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::new();
    let mut ngmr_relate_map: HashMap<RefnoEnum, Vec<(RefnoEnum, RefnoEnum)>> = HashMap::new();
    // 记录每个 target(refno) 属于哪个 (dbnum, batch_id)，用于回写 inst_relate_bool 到 instance_cache。
    let mut target_locations: HashMap<RefnoEnum, (u32, String)> = HashMap::new();
    // 元件库（CATE）布尔：以 inst_info.has_cata_neg 为准补齐 targets（CataNeg 不经 neg_relate_map 指向）。
    let mut cata_targets: HashSet<RefnoEnum> = HashSet::new();

    for dbnum in dbnums {
        let batch_ids = cache_manager.list_batches(dbnum);

        // 重要：instance_cache 是“多 batch 追加”的结构；不同 batch 里可能包含同一 refno/inst_key 的旧数据。
        // 因此这里按 created_at 从新到旧扫描：对每个 key 只取“最新命中”的那一份，旧的直接跳过。
        let mut batches = Vec::new();
        for batch_id in batch_ids {
            if let Some(batch) = cache_manager.get(dbnum, &batch_id).await {
                batches.push((batch.created_at, batch_id, batch));
            }
        }
        batches.sort_by_key(|(ts, _, _)| *ts);

        for (_ts, batch_id, mut batch) in batches.into_iter().rev() {
            // 先扫描 inst_info_map，补齐 CATE 布尔 targets 的 batch 归属。
            for (k, info) in batch.inst_info_map.iter() {
                if info.has_cata_neg {
                    cata_targets.insert(*k);
                    target_locations
                        .entry(*k)
                        .or_insert((dbnum, batch_id.clone()));
                }
            }

            // 先记录 target -> batch 归属（仅需要 keys；避免后续 batch move 导致借用冲突）
            for k in batch.neg_relate_map.keys().copied().collect::<Vec<_>>() {
                target_locations
                    .entry(k)
                    .or_insert((dbnum, batch_id.clone()));
            }
            for k in batch.ngmr_neg_relate_map.keys().copied().collect::<Vec<_>>() {
                target_locations
                    .entry(k)
                    .or_insert((dbnum, batch_id.clone()));
            }

            for (k, v) in batch.inst_info_map.drain() {
                inst_info_map.entry(k).or_insert(v);
            }
            for (k, v) in batch.inst_geos_map.drain() {
                inst_geos_map.entry(k).or_insert(v);
            }
            for (k, v) in batch.neg_relate_map.drain() {
                // 关系数据会被“按 carrier 分批写入”到不同 batch，需要跨 batch 合并 + 去重。
                let entry = neg_relate_map.entry(k).or_insert_with(Vec::new);
                entry.extend(v);
                let mut seen: HashSet<RefnoEnum> = HashSet::new();
                entry.retain(|x| seen.insert(*x));
            }
            for (k, v) in batch.ngmr_neg_relate_map.drain() {
                let entry = ngmr_relate_map.entry(k).or_insert_with(Vec::new);
                entry.extend(v);
                let mut seen: HashSet<(RefnoEnum, RefnoEnum)> = HashSet::new();
                entry.retain(|x| seen.insert(*x));
            }
        }
    }

    let mut processed = 0usize;
    // 目标集合：
    // - 元件库（CATE）负实体：以 inst_info.has_cata_neg 为准；
    // - 设计型负实体：以 neg_relate/ngmr_relate 的 key 为准；
    // 同时过滤掉“看起来像 refno 但实际上是 geom_refno”的 key（即 inst_info_map 中不存在者）。
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

    // 如果指定了过滤集合，只处理该集合内的 refno（用于 debug_model 模式）
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

    for refno in targets {
        let Some(info) = inst_info_map.get(&refno) else {
            continue;
        };
        let inst_key = info.get_inst_key();
        let Some(inst_geos) = inst_geos_map.get(&inst_key) else {
            continue;
        };

        let pos_world_mat = world_mat_for_info(info);
        let inverse_pos_world = pos_world_mat.inverse();

        // 正实体：使用局部变换（pos local space）加载，优先复用 _m.glb 缓存
        let mut pos_manifolds = Vec::new();
        for inst in &inst_geos.insts {
            if !is_pos_geo_for_boolean(&inst.geo_type) {
                continue;
            }
            let mat = local_mat_for_inst(inst);
            match load_or_generate_manifold_glb(&inst.geo_param, inst.geo_hash, mat, false) {
                Ok(m) => pos_manifolds.push(m),
                Err(e) => log_load_manifold_failed("cache_pos", refno, &inst.geo_hash.to_string(), &e),
            }
        }
        if pos_manifolds.is_empty() {
            continue;
        }

        // 负实体：通过关系表（neg_relate/ngmr_relate）定位切割几何，并转换到 pos local space
        let mut neg_manifolds: Vec<ManifoldRust> = Vec::new();

        // 元件库（CATE）“本体孔洞”负实体：只应包含 CataNeg。
        if info.has_cata_neg {
            for inst in &inst_geos.insts {
                if inst.geo_type != GeoBasicType::CataNeg {
                    continue;
                }
                let mat = local_mat_for_inst(inst);
                match load_or_generate_manifold_glb(&inst.geo_param, inst.geo_hash, mat, true) {
                    Ok(m) => neg_manifolds.push(m),
                    Err(e) => log_load_manifold_failed("cache_cata_neg", refno, &inst.geo_hash.to_string(), &e),
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

                    let local_mat = local_mat_for_inst(inst);
                    let neg_world_mat = carrier_world_mat * local_mat;
                    let relative_mat = inverse_pos_world * neg_world_mat;
                    match load_or_generate_manifold_glb(&inst.geo_param, inst.geo_hash, relative_mat, true) {
                        Ok(m) => neg_manifolds.push(m),
                        Err(e) => log_load_manifold_failed("cache_neg", refno, &inst.geo_hash.to_string(), &e),
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
                    let neg_world_mat = carrier_world_mat * local_mat_for_inst(inst);
                    let relative_mat = inverse_pos_world * neg_world_mat;
                    match load_or_generate_manifold_glb(&inst.geo_param, inst.geo_hash, relative_mat, true) {
                        Ok(m) => neg_manifolds.push(m),
                        Err(e) => log_load_manifold_failed("cache_ngmr", refno, &inst.geo_hash.to_string(), &e),
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
        let mut final_manifold = diff_with_guards(pos_union, &neg_manifolds, refno, "lo");

        // 经验：退化为空时尝试高精度重算一次
        if final_manifold.get_mesh().indices.is_empty() {
            let mut pos_hi = Vec::new();
            for inst in &inst_geos.insts {
                if !is_pos_geo_for_boolean(&inst.geo_type) {
                    continue;
                }
                let mat = local_mat_for_inst(inst);
                match load_or_generate_manifold_glb(&inst.geo_param, inst.geo_hash, mat, true) {
                    Ok(m) => pos_hi.push(m),
                    Err(e) => log_load_manifold_failed("cache_pos_hi", refno, &inst.geo_hash.to_string(), &e),
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

        if final_manifold.get_mesh().indices.is_empty() {
            eprintln!("[boolean_worker_cache] 结果为空，跳过输出: refno={}", refno);
            if let Some((dbnum, batch_id)) = target_locations.get(&refno) {
                let mesh_id = {
                    let refu64: aios_core::RefU64 = refno.into();
                    refu64.to_string()
                };
                let _ = cache_manager
                    .upsert_inst_relate_bool(*dbnum, batch_id, refno, mesh_id, "Failed")
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

        // cache-only：布尔结果先保持 Manifold 拓扑计算的正确性，再转换回“重复顶点”的 PlantMesh，
        // 以匹配渲染/导出侧对硬表面（flat shading）与非共享顶点的默认语义。
        let normal_mesh = manifold_to_normal_mesh(final_manifold.get_mesh());
        if let Err(e) = export_single_mesh_to_glb(&normal_mesh, &target_path) {
            eprintln!("[boolean_worker_cache] 导出失败: refno={} err={}", refno, e);
            if let Some((dbnum, batch_id)) = target_locations.get(&refno) {
                let _ = cache_manager
                    .upsert_inst_relate_bool(*dbnum, batch_id, refno, mesh_id.clone(), "Failed")
                    .await;
            }
            continue;
        }

        if let Some((dbnum, batch_id)) = target_locations.get(&refno) {
            cache_manager
                .upsert_inst_relate_bool(*dbnum, batch_id, refno, mesh_id, "Success")
                .await?;
        }

        processed += 1;
    }

    println!("[boolean_worker_cache] 布尔运算完成: {} 个", processed);
    Ok(processed)
}

