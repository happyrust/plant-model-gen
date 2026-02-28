//! Manifold 布尔运算模块

//!

//! 本模块提供基于 Manifold 库的几何体布尔运算功能。

//! 所有布尔运算操作均使用 Manifold 库实现，不再依赖 OpenCASCADE。



use crate::fast_model::{debug_model, debug_model_debug, debug_model_warn};

use aios_core::SurrealQueryExt;

use aios_core::csg::manifold::ManifoldRust;

use aios_core::geometry::csg::{unit_box_mesh, unit_cylinder_mesh, unit_sphere_mesh};

use aios_core::get_db_option;

use aios_core::mesh_precision::LodMeshSettings;

use aios_core::rs_surreal::boolean_query_optimized::query_manifold_boolean_operations_batch_optimized;

use aios_core::{

    CataNegGroup, GmGeoData, ManiGeoTransQuery, NegInfo, query_cata_neg_boolean_groups,

    query_geom_mesh_data, query_negative_entities_batch,

};

use aios_core::{RefnoEnum, SUL_DB, utils::RecordIdExt};

use aios_core::geometry::{EleGeosInfo, EleInstGeosData, GeoBasicType};

use aios_core::geometry::csg::UNIT_MESH_SCALE;

use aios_core::parsed_data::geo_params_data::PdmsGeoParam;

use aios_core::shape::pdms_shape::BrepShapeTrait;

// InstanceCacheManager 已随 instance_cache 模块移除

use glam::DMat4;

use std::collections::{HashMap, HashSet};

use std::path::{Path, PathBuf};

use std::{fs, io};

/// 负实体膨胀量（mm）：消除布尔运算中共面薄片的 epsilon
/// 每边扩展此值，使负实体略微超出正实体表面，产生干净切割
const NEG_INFLATE_EPSILON_MM: f64 = 0.5;



async fn filter_out_bran_refnos(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<RefnoEnum>> {

    if refnos.is_empty() {

        return Ok(Vec::new());

    }



    let refno_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();

    let refno_keys = refno_keys.join(",");

    let sql = format!(

        "SELECT value id FROM [{refno_keys}] WHERE noun != 'BRAN'"

    );

    SUL_DB.query_take(&sql, 0).await

}



/// 根据 mesh_id 和当前 LOD 配置构建完整的 mesh 文件路径

///

/// # 参数

///

/// * `base_dir` - mesh 基础目录（通常是 DbOption.meshes_path 或其父目录）

/// * `mesh_id` - mesh 文件 ID

///

/// # 返回

///

/// 完整的 mesh 文件路径，格式为：

/// - `{base_dir}/lod_{LOD}/{mesh_id}_{LOD}.glb`（启用 LOD 时）

/// - `{base_dir}/{mesh_id}.glb`（无 LOD）

///

/// # 示例

///

/// ```ignore

/// let path = build_lod_mesh_path(Path::new("/assets/meshes"), "12232319344565648304");

/// // 返回: /assets/meshes/lod_L2/12232319344565648304_L2.mesh

/// ```

fn build_lod_mesh_path(base_dir: &Path, mesh_id: &str) -> PathBuf {

    use aios_core::mesh_precision::LodLevel;



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

/// 构建 _m.manifold 布尔运算专用 mesh 文件路径（在 manifold 目录查找）
fn build_manifold_mesh_path(base_dir: &Path, mesh_id: &str) -> Option<PathBuf> {
    // 溯源到不含 lod_ 的基础目录
    let mut clean_base = base_dir.to_path_buf();
    while let Some(last_component) = clean_base.file_name().and_then(|n| n.to_str()) {
        if last_component.starts_with("lod_") {
            clean_base.pop();
        } else {
            break;
        }
    }

    let filename = format!("{}_m.manifold", mesh_id);

    // 在 manifold 目录查找
    let manifold_path = clean_base.join("manifold").join(&filename);
    if manifold_path.exists() {
        return Some(manifold_path);
    }

    None
}



fn mesh_base_dir() -> PathBuf {

    get_db_option()

        .meshes_path

        .as_ref()

        .map(PathBuf::from)

        .unwrap_or_else(|| PathBuf::from("assets/meshes"))

}



// 移除 load_mesh，改用 load_manifold



/// 从文件加载流形数据

///

/// # 参数

///

/// * `dir` - 模型文件目录路径

/// * `id` - 网格文件的ID

/// * `mat` - 变换矩阵

/// * `more_precision` - 是否需要更高精度

///

/// # 返回值

///

/// 返回 `anyhow::Result<ManifoldRust>` 表示加载是否成功以及加载的流形数据

#[inline]

fn load_manifold(id: &str, mat: DMat4, more_precision: bool) -> anyhow::Result<ManifoldRust> {

    // 对标准单位几何体（1/2/3）强制使用内置几何生成，避免磁盘上被误写/污染的同名 GLB 影响布尔结果。

    if matches!(id, "1" | "2" | "3") {

        debug_model_debug!("load_manifold: 使用内置 unit mesh: id={}", id);

        let unit_mesh = match id {

            "1" => unit_box_mesh(),

            "2" => unit_cylinder_mesh(&LodMeshSettings::default(), false),

            "3" => unit_sphere_mesh(),

            _ => unreachable!(),

        };

        // 将 Vec3 数组转换为 glam::Vec3 数组

        let vertices: Vec<glam::Vec3> = unit_mesh.vertices.iter()

            .map(|v| glam::Vec3::new(v[0], v[1], v[2]))

            .collect();

        let manifold = ManifoldRust::from_vertices_indices(&vertices, &unit_mesh.indices, mat, more_precision);

        // 复用下面的"空/哨兵"校验逻辑

        let mesh = manifold.get_mesh();

        if mesh.indices.is_empty() {

            return Err(anyhow::anyhow!("单位 Manifold mesh 为空：id={}", id));

        }

        if let Some(aabb) = mesh.cal_aabb() {

            let ext_mag = aabb.extents().magnitude();

            if ext_mag.is_finite() && ext_mag < 1e-6 {

                return Err(anyhow::anyhow!(

                    "单位 Manifold mesh 可能为空（哨兵 cube）：id={} ext_mag={:.3e}",

                    id,

                    ext_mag

                ));

            }

        } else {

            return Err(anyhow::anyhow!("单位 Manifold mesh AABB 无效：id={}", id));

        }

        return Ok(manifold);

    }



    let base_dir = mesh_base_dir();

    // ── 从 .manifold 加载（生成阶段已完成 Manifold 验证，直接变换即可） ──
    if let Some(manifold_path) = build_manifold_mesh_path(&base_dir, id) {
        use aios_core::csg::manifold::ManifoldMeshRust;
        match ManifoldMeshRust::load_from_file(&manifold_path) {
            Ok(raw_mesh) => {
                if !raw_mesh.vertices.is_empty() && !raw_mesh.indices.is_empty() {
                    let transformed_verts: Vec<f32> = raw_mesh.vertices.chunks_exact(3)
                        .flat_map(|c| {
                            let pt = mat.transform_point3(glam::DVec3::new(
                                c[0] as f64, c[1] as f64, c[2] as f64,
                            ));
                            [pt.x as f32, pt.y as f32, pt.z as f32]
                        })
                        .collect();
                    let manifold = ManifoldRust::from_mesh(&ManifoldMeshRust {
                        vertices: transformed_verts,
                        indices: raw_mesh.indices,
                    });
                    match validate_manifold_result(manifold, id) {
                        Ok(m) => {
                            debug_model_debug!(
                                "load_manifold: 从 .manifold 加载成功: id={} path={}",
                                id, manifold_path.display()
                            );
                            return Ok(m);
                        }
                        Err(e) => {
                            eprintln!(
                                "[Manifold] .manifold 加载失败: id={} err={}",
                                id, e
                            );
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[Manifold] .manifold 文件读取失败: id={} err={}",
                    id, e
                );
            }
        }
    }

    // ── 兼容：从 GLB 加载（旧数据没有 .manifold） ──
    let mesh_path = build_lod_mesh_path(&base_dir, id);
    let manifold = ManifoldRust::import_glb_to_manifold(&mesh_path, mat, more_precision)?;
    validate_manifold_result(manifold, id)
}



/// 直接从几何参数生成 Manifold 模型

///

/// 此函数避免了从 glb 文件加载转换的问题：

/// - 不依赖 glb 文件存在

/// - 避免 glb → Manifold 转换时的精度丢失

///

/// # 参数

///

/// * `geo_param` - 几何参数

/// * `geo_hash` - 几何哈希值

/// * `mat` - 变换矩阵

/// * `more_precision` - 是否需要更高精度

///

/// # 返回值

///

/// 返回 `anyhow::Result<ManifoldRust>` 表示生成是否成功

#[inline]

pub(crate) fn load_manifold_from_geo_param(

    geo_param: &PdmsGeoParam,

    geo_hash: u64,

    mat: DMat4,

    more_precision: bool,

) -> anyhow::Result<ManifoldRust> {

    // 对标准单位几何体（1/2/3）使用内置几何生成

    if matches!(geo_hash, 1 | 2 | 3) {

        debug_model_debug!("load_manifold_from_geo_param: 使用内置 unit mesh: geo_hash={}", geo_hash);

        let unit_mesh = match geo_hash {

            1 => unit_box_mesh(),

            2 => unit_cylinder_mesh(&LodMeshSettings::default(), false),

            3 => unit_sphere_mesh(),

            _ => unreachable!(),

        };

        let manifold = ManifoldRust::from_vertices_indices(&unit_mesh.vertices, &unit_mesh.indices, mat, more_precision);

        return validate_manifold_result(manifold, &geo_hash.to_string());

    }



    // 尝试从 geo_param 直接生成 mesh

    let plant_mesh = match geo_param {

        PdmsGeoParam::Unknown | PdmsGeoParam::CompoundShape => {

            // 对于 Unknown 和 CompoundShape，回退到从 glb 加载

            debug_model_debug!(

                "load_manifold_from_geo_param: geo_param 不支持直接生成，回退到 glb 加载: geo_hash={}",

                geo_hash

            );

            return load_manifold(&geo_hash.to_string(), mat, more_precision);

        }

        _ => {

            // 尝试使用 gen_csg_shape 生成

            match geo_param.gen_csg_shape_compat() {

                Ok(csg_mesh) => (*csg_mesh.0).clone(),

                Err(e) => {

                    debug_model_debug!(

                        "load_manifold_from_geo_param: gen_csg_shape 失败 ({}), 尝试回退到 glb 加载: geo_hash={}",

                        e,

                        geo_hash

                    );

                    return load_manifold(&geo_hash.to_string(), mat, more_precision);

                }

            }

        }

    };



    if plant_mesh.vertices.is_empty() || plant_mesh.indices.is_empty() {

        return Err(anyhow::anyhow!(

            "从 geo_param 生成的 mesh 为空: geo_hash={} type={}",

            geo_hash,

            geo_param.type_name()

        ));

    }



    let manifold = ManifoldRust::from_vertices_indices(&plant_mesh.vertices, &plant_mesh.indices, mat, more_precision);

    validate_manifold_result(manifold, &geo_hash.to_string())

}



/// 校验 Manifold 结果是否有效

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



#[inline]

fn log_load_manifold_failed(scene: &str, refno: RefnoEnum, mesh_id: &str, err: &anyhow::Error) {

    eprintln!(

        "[bool][{}] load_manifold 失败: refno={} mesh_id={} err={}",

        scene, refno, mesh_id, err

    );

}



#[inline]

fn is_source_level_manifold_error(err: &anyhow::Error) -> bool {

    let msg = err.to_string();

    msg.contains("Manifold mesh 为空")

        || msg.contains("Manifold mesh AABB 无效")

        || msg.contains("哨兵 cube")

        || msg.contains("No such file or directory")

        || msg.contains("(os error 2)")

}



fn ensure_parent_dir(path: &Path) -> io::Result<()> {

    if let Some(parent) = path.parent() {

        fs::create_dir_all(parent)?;

    }

    Ok(())

}



/// 标记布尔运算失败

///

/// 写入 inst_relate_bool 状态为 Failed

async fn mark_bool_failed(refno: RefnoEnum) -> anyhow::Result<()> {

    crate::fast_model::utils::save_inst_relate_bool(refno, None, "Failed", "bool_mesh").await?;

    Ok(())

}



/// 更新布尔运算结果到数据库

///

/// 仅写入 inst_relate_bool（状态 + mesh_id），AABB 已写入 inst_relate_aabb

async fn update_booled_result(

    refno: RefnoEnum,

    mesh_id: &str,

    aabb: Option<parry3d::bounding_volume::Aabb>,

) -> anyhow::Result<()> {

    use dashmap::DashMap;

    

    if let Some(aabb) = aabb {

        // 使用 hash 格式存储 AABB（与 mesh_generate.rs 保持一致）

        let aabb_hash = aios_core::gen_aabb_hash(&aabb);

        

        // 保存 AABB 记录到 SurrealDB

        let aabb_map = DashMap::new();

        aabb_map.insert(aabb_hash.to_string(), aabb);

        crate::fast_model::utils::save_aabb_to_surreal(&aabb_map).await;

        

        let inst_aabb_map = DashMap::new();

        inst_aabb_map.insert(refno, aabb_hash.to_string());

        crate::fast_model::utils::save_inst_relate_aabb(&inst_aabb_map, "bool_mesh").await;

        

        crate::fast_model::utils::save_inst_relate_bool(refno, Some(mesh_id), "Success", "bool_mesh")

            .await?;

    } else {

        crate::fast_model::utils::save_inst_relate_bool(refno, Some(mesh_id), "Success", "bool_mesh")

            .await?;

    }

    

    Ok(())

}



// fn boolean_mesh_path(mesh_id: &str) -> PathBuf {

//     build_lod_mesh_path(&mesh_base_dir(), mesh_id)

// }



fn boolean_glb_path(mesh_id: &str) -> PathBuf {

    let mut path = build_lod_mesh_path(&mesh_base_dir(), mesh_id);

    path.set_extension("glb");

    path

}



/// Manifold 共享顶点 mesh -> 重复顶点 PlantMesh（flat shading）

fn manifold_to_normal_mesh(mesh: aios_core::csg::manifold::ManifoldMeshRust) -> aios_core::shape::pdms_shape::PlantMesh {

    use aios_core::shape::pdms_shape::PlantMesh;

    use glam::Vec3;



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

        if tri.len() != 3 { break; }

        let v0 = get_v(tri[0]);

        let v1 = get_v(tri[1]);

        let v2 = get_v(tri[2]);



        let face_n = (v1 - v0).cross(v2 - v0);

        let n = if face_n.length_squared() > 1e-10 { face_n.normalize() } else { Vec3::Y };



        let base = out.vertices.len() as u32;

        out.vertices.extend_from_slice(&[v0, v1, v2]);

        out.normals.extend_from_slice(&[n, n, n]);

        out.uvs.extend_from_slice(&[[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]);

        out.indices.extend_from_slice(&[base, base + 1, base + 2]);

    }

    out

}



fn boolean_obj_path(mesh_id: &str) -> PathBuf {

    let mut path = build_lod_mesh_path(&mesh_base_dir(), mesh_id);

    path.set_extension("obj");

    path

}



/// 处理元件库负实体布尔（catalog 级别）

pub async fn apply_cata_neg_boolean_manifold(

    refnos: &[RefnoEnum],

    replace_exist: bool,

) -> anyhow::Result<()> {

    use crate::fast_model::export_model::export_glb::export_single_mesh_to_glb;



    if refnos.is_empty() {

        return Ok(());

    }



    let filtered_refnos = filter_out_bran_refnos(refnos).await?;

    if filtered_refnos.is_empty() {

        return Ok(());

    }



    let params = query_cata_neg_boolean_groups(&filtered_refnos, replace_exist).await?;

    if params.is_empty() {

        return Ok(());

    }



    // 对“缺文件/空 mesh”这类源级失败按 geo_hash 去重：

    // - 避免同一个坏源在多组布尔中反复加载与刷屏

    // - 命中后直接跳过该源，显著减少无效开销

    let mut source_failed_geo_hashes: HashMap<u64, String> = HashMap::new();

    let mut source_failed_geo_hashes_warned: HashSet<u64> = HashSet::new();



    let mut try_load_geo_manifold = |scene: &str,

                                     refno: RefnoEnum,

                                     geo_param: &PdmsGeoParam,

                                     geo_hash: u64,

                                     mat: DMat4,

                                     more_precision: bool|

     -> Option<ManifoldRust> {

        if let Some(reason) = source_failed_geo_hashes.get(&geo_hash) {

            if source_failed_geo_hashes_warned.insert(geo_hash) {

                eprintln!(

                    "[bool][{}] 跳过已知失败几何: refno={} mesh_id={} reason={}",

                    scene, refno, geo_hash, reason

                );

            }

            return None;

        }



        match load_manifold_from_geo_param(geo_param, geo_hash, mat, more_precision) {

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



    for g in params {

        // 收集当前实例涉及的所有几何，批量查询 mesh 数据

        let geom_refnos: Vec<RefnoEnum> = g.boolean_group.iter().flatten().cloned().collect();

        let gms: Vec<GmGeoData> = query_geom_mesh_data(g.refno, &geom_refnos).await?;



        let mut update_sql = String::new();

        for bg in g.boolean_group {

            let Some(pos) = gms.iter().find(|x| x.geom_refno == bg[0]) else {

                crate::fast_model::utils::save_inst_relate_cata_bool(

                    g.refno,

                    None,

                    "Failed",

                    "cata_bool",

                )

                .await;

                continue;

            };



            let pos_mesh_id = pos.id.to_mesh_id();

            let pos_geo_hash: u64 = pos_mesh_id.parse().unwrap_or(0);

            debug_model_debug!("加载 catalog 正实体 mesh: geo_hash={}", pos_geo_hash);

            let mut pos_tf = pos.trans.0.clone();

            if matches!(pos_geo_hash, 1 | 2 | 3) {

                pos_tf.scale /= aios_core::geometry::csg::UNIT_MESH_SCALE;

            }

            let mut pos_manifold = match try_load_geo_manifold(

                "cata_pos",

                g.refno,

                &pos.param,

                pos_geo_hash,

                pos_tf.to_matrix().as_dmat4(),

                false,

            ) {

                Some(m) => m,

                None => {

                    println!("布尔运算失败: 无法加载正实体 manifold, refno: {}", &g.refno);

                    crate::fast_model::utils::save_inst_relate_cata_bool(

                        g.refno,

                        None,

                        "Failed",

                        "cata_bool",

                    )

                    .await;

                    continue;

                }

            };



            let mut neg_manifolds = Vec::new();

            let mut neg_load_fail_cnt = 0usize;

            for &neg in bg.iter().skip(1) {

                let Some(neg_geo) = gms.iter().find(|x| x.geom_refno == neg) else {

                    continue;

                };

                let neg_mesh_id = neg_geo.id.to_mesh_id();

                let neg_geo_hash: u64 = neg_mesh_id.parse().unwrap_or(0);

                let mut neg_tf = neg_geo.trans.0.clone();

                if matches!(neg_geo_hash, 1 | 2 | 3) {

                    neg_tf.scale /= aios_core::geometry::csg::UNIT_MESH_SCALE;

                }

                match try_load_geo_manifold(

                    "cata_neg",

                    g.refno,

                    &neg_geo.param,

                    neg_geo_hash,

                    neg_tf.to_matrix().as_dmat4(),

                    true,

                ) {

                    Some(manifold) => neg_manifolds.push(manifold.inflate_from_center(NEG_INFLATE_EPSILON_MM)),

                    None => neg_load_fail_cnt += 1,

                }

            }



            if bg.len() > 1 && neg_manifolds.is_empty() {

                eprintln!(

                    "[bool][cata] 布尔跳过：负实体全部不可用，refno={} neg_total={}",

                    g.refno,

                    bg.len().saturating_sub(1)

                );

                crate::fast_model::utils::save_inst_relate_cata_bool(

                    g.refno,

                    None,

                    "Failed",

                    "cata_bool",

                )

                .await;

                continue;

            }



            if neg_load_fail_cnt > 0 {

                eprintln!(

                    "[bool][cata] 部分负实体加载失败（已忽略失败项）: refno={} failed={} loaded={}",

                    g.refno,

                    neg_load_fail_cnt,

                    neg_manifolds.len()

                );

            }



            // 即使没有负实体，也标记已处理，避免重复计算

            // 统一使用 RefU64 格式（如 17496_106028），不管 sesno 是否为 0

            let refu64: aios_core::RefU64 = g.refno.into();

            let mesh_id = refu64.to_string();

            let mut final_manifold = pos_manifold.batch_boolean_subtract(&neg_manifolds);



            // 经验：某些模型在默认精度下布尔结果可能退化为 0 三角形，尝试提升精度重算一次。

            if !neg_manifolds.is_empty() && final_manifold.get_mesh().indices.is_empty() {

                eprintln!(

                    "[bool][cata] ⚠️ 布尔结果为空，尝试 more_precision=true 重算: refno={}",

                    g.refno

                );

                if let Ok(pos_hi) = load_manifold_from_geo_param(

                    &pos.param,

                    pos_geo_hash,

                    pos.trans.0.to_matrix().as_dmat4(),

                    true,

                ) {

                    final_manifold = pos_hi.batch_boolean_subtract(&neg_manifolds);

                }

            }

            let target_path = boolean_glb_path(&mesh_id);

            ensure_parent_dir(&target_path)?;



            if final_manifold.export_to_glb(&target_path).is_ok() {

                // ========== 步骤 1：创建布尔结果 mesh 记录 ==========

                // 创建 inst_geo 记录，标记为已网格化

                update_sql.push_str(&format!(

                    "create inst_geo:⟨{}⟩ set meshed = true, aabb = {};",

                    mesh_id,

                    &pos.aabb_id.to_raw()

                ));



                // ========== 步骤 2：创建 geo_relate 关联记录 ==========

                //

                // 关键点：SurrealQL 的 relation table 写入语法是 `INSERT RELATION INTO <table> { ... }`

                // 或 `INSERT RELATION INTO <table> [ {...}, ... ]`（见官方 INSERT 文档的 Insert relation tables）。

                // 这里不能写成 `INSERT RELATION <table:id> CONTENT {...}`，否则部分 SurrealDB 版本会报

                // “Unexpected token `CONTENT`, expected Eof”。

                //

                // 目标：relation id 需“实例唯一”，但同一实例内只保留一条 CatePos。
                // 历史问题：
                // - 仅用 geom_refno 作为 id，会发生跨实例覆盖；
                // - 为每个 boolean_group 生成一条 CatePos，会导致同一 inst_geo 被重复导出（OBJ 翻倍）。
                //
                // 这里固定为“每实例一个 CatePos id”，后续循环内覆盖更新，最终仅保留最新结果。
                let relation_id = g.refno.to_string();



                // 先删除该实例下全部旧 CatePos（兼容历史 id 规则，且避免同实例重复导出）
                update_sql.push_str(&format!(
                    "LET $old_geo_ids = SELECT VALUE id FROM {}->geo_relate WHERE geo_type = 'CatePos'; DELETE $old_geo_ids;",
                    &g.inst_info_id.to_raw(),
                ));



                // 建立 inst_info -> geo_relate -> inst_geo 的关系

                // geo_type='CatePos' 表示这是布尔运算后的结果（应该导出）

                let relate_sql = format!(

                    "INSERT RELATION INTO geo_relate {{ in: {}, id: '{rel_id}', out: inst_geo:⟨{mesh_id}⟩, geom_refno: pe:⟨{geom_refno}⟩, geo_type: 'CatePos', trans: trans:⟨0⟩, visible: true }};",

                    &g.inst_info_id.to_raw(),

                    rel_id = relation_id,

                    mesh_id = mesh_id,

                    geom_refno = bg[0],

                );

                update_sql.push_str(&relate_sql);

                

                // ========== 步骤 3：隐藏原始几何记录 ==========

                // 将原正实体的 geo_type 从 Pos 更新为 Compound（不导出）

                // 并设置 visible=false，使其在查询时被排除

                let hide_original_sql = format!(

                    "UPDATE {}->geo_relate SET geo_type = 'Compound', visible = false WHERE geom_refno = pe:⟨{}⟩ AND geo_type IN ['Pos','Compound'];",

                    &g.inst_info_id.to_raw(),

                    bg[0],

                );

                update_sql.push_str(&hide_original_sql);



                // ========== 步骤 4：写入 catalog 布尔状态 ==========

                // 用于 worker 去重与排查

                crate::fast_model::utils::save_inst_relate_cata_bool(

                    g.refno,

                    Some(&mesh_id),

                    "Success",

                    "cata_bool",

                )

                .await;

            } else {

                crate::fast_model::utils::save_inst_relate_cata_bool(

                    g.refno,

                    None,

                    "Failed",

                    "cata_bool",

                )

                .await;

            }

        }



        if !update_sql.is_empty() {

            // 仅在 debug_model 下打印 SQL，便于定位 SurrealQL 解析/兼容性问题。

            if aios_core::is_debug_model_enabled() {

                let preview = update_sql.chars().take(8000).collect::<String>();

                if update_sql.chars().count() > 8000 {

                    debug_model_debug!(

                        "[boolean_worker] 将执行 update_sql (len={}):\n{}...\n[truncated]",

                        update_sql.len(),

                        preview

                    );

                } else {

                    debug_model_debug!(

                        "[boolean_worker] 将执行 update_sql (len={}):\n{}",

                        update_sql.len(),

                        preview

                    );

                }

            }



            if let Err(e) = SUL_DB.query(update_sql.clone()).await {

                debug_model_warn!(

                    "[boolean_worker] 执行 update_sql 失败: {}\nSQL(len={}):\n{}",

                    e,

                    update_sql.len(),

                    update_sql

                );

                return Err(e.into());

            }

        }

    }



    debug_model!("元件库的负实体计算{:?}完成", refnos);

    Ok(())

}



async fn apply_boolean_for_query(

    query: ManiGeoTransQuery,

    replace_exist: bool,

) -> anyhow::Result<()> {

    fn aabb_contains(outer: &parry3d::bounding_volume::Aabb, inner: &parry3d::bounding_volume::Aabb) -> bool {

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



    // 非替换模式下，已有成功记录则跳过

    if !replace_exist {

        let check_sql = format!(

            "select value status from inst_relate_bool:{} limit 1",

            query.refno.to_string()

        );

        let existing_status: Vec<Option<String>> = SUL_DB.query_take(&check_sql, 0).await?;

        if matches!(

            existing_status

                .first()

                .and_then(|s| s.as_deref()),

            Some("Success")

        ) {

            debug_model!("跳过已存在的布尔结果: {}", query.refno);

            return Ok(());

        }

    }



    // 使用正实体的世界坐标系作为基准坐标系

    // 正实体在基准坐标系中，使用单位矩阵（相对于自身的坐标系）

    let pos_world_mat = query.inst_world_trans.0.to_matrix().as_dmat4();



    // 没有任何负实体关系：不需要产出 bool 结果，也不应写入 Failed（否则会污染 inst_relate_bool）

    if query.neg_ts.is_empty() {

        debug_model_debug!("跳过布尔：无负实体关系 refno={}", query.refno);

        return Ok(());

    }



    let mut pos_manifolds = Vec::new();

    for (pos_id, pos_t) in query.pos_geos.iter() {

        let pos_mesh_id = pos_id.to_mesh_id();

        // 正实体使用局部变换

        let pos_local_mat = pos_t.0.to_matrix().as_dmat4();

        println!(

            "[POS_TRANS_DBG] refno={} mesh={} pos_local_mat:\n  col0=({:.3},{:.3},{:.3})\n  col1=({:.3},{:.3},{:.3})\n  col2=({:.3},{:.3},{:.3})\n  col3=({:.3},{:.3},{:.3})\n  scale=({:.3},{:.3},{:.3})",

            query.refno, pos_mesh_id,

            pos_local_mat.col(0).x, pos_local_mat.col(0).y, pos_local_mat.col(0).z,

            pos_local_mat.col(1).x, pos_local_mat.col(1).y, pos_local_mat.col(1).z,

            pos_local_mat.col(2).x, pos_local_mat.col(2).y, pos_local_mat.col(2).z,

            pos_local_mat.col(3).x, pos_local_mat.col(3).y, pos_local_mat.col(3).z,

            pos_local_mat.col(0).truncate().length(),

            pos_local_mat.col(1).truncate().length(),

            pos_local_mat.col(2).truncate().length(),

        );

        debug_model_debug!(

            "加载正实体 mesh: {} (应用局部变换)",

            pos_mesh_id

        );

        match load_manifold(&pos_mesh_id, pos_local_mat, false) {

            Ok(manifold) => pos_manifolds.push(manifold),

            Err(e) => log_load_manifold_failed("inst_pos", query.refno, &pos_mesh_id, &e),

        }

    }



    if pos_manifolds.is_empty() {

        println!(

            "布尔运算失败: 未找到正实体 manifold，refno: {}, 正几何数量={}",

            query.refno,

            query.pos_geos.len()

        );

        mark_bool_failed(query.refno).await?;

        return Ok(());

    }



    let mut pos_manifold = ManifoldRust::batch_boolean(&pos_manifolds, aios_core::csg::manifold::ManifoldOpType::Union);

    if pos_manifold.get_mesh().indices.is_empty() {

        println!(

            "布尔运算失败: 正实体 manifold 没有三角形, refno: {}",

            query.refno

        );

        mark_bool_failed(query.refno).await?;

        return Ok(());

    }



    // 调试：打印正实体 AABB

    if let Some(pos_aabb) = pos_manifold.get_mesh().cal_aabb() {

        println!("[AABB_DBG] pos refno={} aabb: min=({:.1},{:.1},{:.1}) max=({:.1},{:.1},{:.1}) verts={} tris={}",

            query.refno,

            pos_aabb.mins.x, pos_aabb.mins.y, pos_aabb.mins.z,

            pos_aabb.maxs.x, pos_aabb.maxs.y, pos_aabb.maxs.z,

            pos_manifold.get_mesh().vertices.len() / 3,

            pos_manifold.get_mesh().indices.len() / 3,

        );

    }



    // 调试：导出正实体 OBJ

    let pos_obj_path = format!("test_output/debug_{}_pos.obj", query.refno);

    if let Err(e) = pos_manifold.export_to_obj(&pos_obj_path) {

        eprintln!("导出正实体 OBJ 失败: {}", e);

    } else {

        println!("✅ 导出正实体 OBJ: {}", pos_obj_path);

    }



    // 计算正实体世界坐标系的逆矩阵，用于将负实体转换到正实体的相对坐标系

    let inverse_pos_world = pos_world_mat.inverse();

    let mut neg_manifolds = Vec::new();

    let mut neg_load_fail_logged = 0usize;

    for (_, _carrier_wt, neg_infos) in query.neg_ts.iter() {

        for NegInfo {

            id, geo_local_trans, aabb, carrier_world_trans, ..

        } in neg_infos.iter().cloned()

        {

            if aabb.is_none() {

                continue;

            }



            let neg_mesh_id = id.to_mesh_id();

            let is_unit_mesh = matches!(neg_mesh_id.as_str(), "1" | "2" | "3");



            // 使用 NegInfo 中的 carrier_world_trans（每个负实体自己的载体世界变换）

            // 而不是 neg_ts 中的 carrier_wt（可能是虚拟的单位矩阵）

            let carrier_world_mat = carrier_world_trans

                .as_ref()

                .map(|t| t.0.to_matrix().as_dmat4())

                .unwrap_or(DMat4::IDENTITY);



            // 计算负实体相对于正实体坐标系的变换矩阵

            // 相对变换 = inverse(正实体世界坐标) × 负实体世界坐标

            // 负实体世界坐标 = carrier_world_mat × geo_local_trans

            // 注意：单位几何体（geo_hash=1/2/3）在当前数据中，scale 字段往往是“实际尺寸(mm)”而非“归一化比例”，

            // 而 unit_*_mesh 本身的尺寸为 UNIT_MESH_SCALE(=100)。因此需要把 scale 再除以 UNIT_MESH_SCALE 才能得到正确尺寸。

            let mut geo_tf = geo_local_trans.0;

            if is_unit_mesh {

                geo_tf.scale /= aios_core::geometry::csg::UNIT_MESH_SCALE;

            }

            let neg_world_mat = carrier_world_mat * geo_tf.to_matrix().as_dmat4();

            let relative_mat = inverse_pos_world * neg_world_mat;

            

            // 调试：打印变换矩阵信息

            println!("[变换调试] neg_id={}", neg_mesh_id);

            println!("  pos_world_trans: {:?}", pos_world_mat.col(3));

            println!("  carrier_world: {:?}", carrier_world_mat.col(3));

            println!("  geo_local: {:?}", geo_local_trans.0.to_matrix().as_dmat4().col(3));

            println!("  geo_scale: {:?}", geo_local_trans.0.scale);

            if is_unit_mesh {

                println!("  geo_scale_eff(unit/100): {:?}", geo_tf.scale);

            }

            if let Some(t) = carrier_world_trans.as_ref() {

                println!("  carrier_scale: {:?}", t.0.scale);

            }

            println!("  relative: {:?}", relative_mat.col(3));

            println!(

                "  relative_basis_len: x={:.6} y={:.6} z={:.6}",

                relative_mat.col(0).truncate().length(),

                relative_mat.col(1).truncate().length(),

                relative_mat.col(2).truncate().length(),

            );



            debug_model_debug!("加载负实体 mesh: {} (相对于正实体坐标系)", neg_mesh_id);



            match load_manifold(&neg_mesh_id, relative_mat, true) {

                Ok(manifold) => {

                    if let Some(neg_aabb) = manifold.get_mesh().cal_aabb() {

                        println!("[AABB_DBG] neg[{}] id={} aabb: min=({:.1},{:.1},{:.1}) max=({:.1},{:.1},{:.1}) verts={} tris={}",

                            neg_manifolds.len(), neg_mesh_id,

                            neg_aabb.mins.x, neg_aabb.mins.y, neg_aabb.mins.z,

                            neg_aabb.maxs.x, neg_aabb.maxs.y, neg_aabb.maxs.z,

                            manifold.get_mesh().vertices.len() / 3,

                            manifold.get_mesh().indices.len() / 3,

                        );

                    }

                    neg_manifolds.push(manifold.inflate_from_center(NEG_INFLATE_EPSILON_MM));

                }

                Err(e) => {

                    // 负实体可能数量很大，简单限流，避免刷屏

                    if neg_load_fail_logged < 10 {

                        log_load_manifold_failed("inst_neg", query.refno, &neg_mesh_id, &e);

                        neg_load_fail_logged += 1;

                    }

                }

            }

        }

    }



    if neg_manifolds.is_empty() {

        println!(

            "布尔运算失败: 未找到负实体 manifold，refno: {}, neg 载体数={}",

            query.refno,

            query.neg_ts.len()

        );

        mark_bool_failed(query.refno).await?;

        return Ok(());

    }



    // 调试：导出负实体 OBJ（合并所有负实体）

    let neg_union = ManifoldRust::batch_boolean(&neg_manifolds, aios_core::csg::manifold::ManifoldOpType::Union);

    let neg_obj_path = format!("test_output/debug_{}_neg.obj", query.refno);

    if let Err(e) = neg_union.export_to_obj(&neg_obj_path) {

        eprintln!("导出负实体 OBJ 失败: {}", e);

    } else {

        println!("✅ 导出负实体 OBJ: {}", neg_obj_path);

    }



    // 逐个减去负实体，并在出现“异常清空”时尽量避免把结果整个抹掉：

    // - 如果 neg 的 AABB 不与当前结果相交，差集不应改变；若却得到空结果，认为是数值/拓扑异常，跳过该 neg。

    // - 如果 neg 的 AABB 未包含当前结果 AABB，但差集得到空结果，也认为高度可疑，跳过该 neg。

    let mut final_manifold = pos_manifold.clone();

    for (i, neg) in neg_manifolds.iter().enumerate() {

        let before = final_manifold.clone();

        let before_aabb = before.get_mesh().cal_aabb();

        let neg_aabb = neg.get_mesh().cal_aabb();



        // 如果当前结果已经是空的，就没必要继续差集了

        if before.get_mesh().indices.is_empty() {

            break;

        }



        let mut after = before.clone();

        after.inner = after.inner.difference(&neg.inner);



        if after.get_mesh().indices.is_empty() {

            match (&before_aabb, &neg_aabb) {

                (Some(before_aabb), Some(neg_aabb)) => {

                    let intersects = aabb_intersects(before_aabb, neg_aabb);

                    let contains = aabb_contains(neg_aabb, before_aabb);



                    // 差集把结果清空只有在 neg 真实覆盖/包含正实体时才合理；否则认为异常并跳过该 neg。

                    if !intersects || !contains {

                        eprintln!(

                            "[bool][inst] ⚠️ 差集结果被异常清空，跳过该负实体: refno={} neg_idx={} intersects={} contains={}",

                            query.refno, i, intersects, contains

                        );

                        final_manifold = before;

                        continue;

                    }

                }

                _ => {

                    eprintln!(

                        "[bool][inst] ⚠️ 差集结果被清空且无法计算 AABB，跳过该负实体: refno={} neg_idx={}",

                        query.refno, i

                    );

                    final_manifold = before;

                    continue;

                }

            };

        }



        final_manifold = after;

    }



    // 额外兜底：如果逐个 subtract 仍退化为空，尝试先 union 再一次 difference，

    // 避免多次 difference 引入的数值/拓扑退化。

    if final_manifold.get_mesh().indices.is_empty() {

        let pos_aabb = pos_manifold.get_mesh().cal_aabb();

        let neg_union = ManifoldRust::batch_boolean(

            &neg_manifolds,

            aios_core::csg::manifold::ManifoldOpType::Union,

        );

        let union_aabb = neg_union.get_mesh().cal_aabb();



        let mut union_diff = pos_manifold.clone();

        union_diff.inner = union_diff.inner.difference(&neg_union.inner);



        if union_diff.get_mesh().indices.is_empty() {

            match (&pos_aabb, &union_aabb) {

                (Some(pos_aabb), Some(union_aabb)) => {

                    if aabb_contains(union_aabb, pos_aabb) {

                        final_manifold = union_diff;

                    } else {

                        eprintln!(

                            "[bool][inst] ⚠️ union-diff 清空但 AABB 未包含正体，疑似退化: refno={}",

                            query.refno

                        );

                    }

                }

                _ => {

                    eprintln!(

                        "[bool][inst] ⚠️ union-diff 清空且无法计算 AABB，保留逐个 subtract 的结果: refno={}",

                        query.refno

                    );

                }

            }

        } else {

            final_manifold = union_diff;

        }

    }



    // 经验：当差集退化为空时，通常是精度/焊接导致的数值问题；尝试提升正实体加载精度重算一次。

    if final_manifold.get_mesh().indices.is_empty() {

        eprintln!(

            "[bool][inst] ⚠️ 布尔结果为空，尝试 more_precision=true 重算: refno={}",

            query.refno

        );



        let mut pos_hi_manifolds = Vec::new();

        for (pos_id, pos_t) in query.pos_geos.iter() {

            let pos_mesh_id = pos_id.to_mesh_id();

            let pos_local_mat = pos_t.0.to_matrix().as_dmat4();

            if let Ok(m) = load_manifold(&pos_mesh_id, pos_local_mat, true) {

                pos_hi_manifolds.push(m);

            }

        }



        if !pos_hi_manifolds.is_empty() {

            let pos_hi = ManifoldRust::batch_boolean(

                &pos_hi_manifolds,

                aios_core::csg::manifold::ManifoldOpType::Union,

            );

            if !pos_hi.get_mesh().indices.is_empty() {

                // 复用“逐个 subtract + 退化保护 + union-diff 兜底”的逻辑，避免高精度重算时再次退化。

                let mut hi_final = pos_hi.clone();

                for (i, neg) in neg_manifolds.iter().enumerate() {

                    let before = hi_final.clone();

                    let before_aabb = before.get_mesh().cal_aabb();

                    let neg_aabb = neg.get_mesh().cal_aabb();



                    if before.get_mesh().indices.is_empty() {

                        break;

                    }



                    let mut after = before.clone();

                    after.inner = after.inner.difference(&neg.inner);



                    if after.get_mesh().indices.is_empty() {

                        match (&before_aabb, &neg_aabb) {

                            (Some(before_aabb), Some(neg_aabb)) => {

                                let intersects = aabb_intersects(before_aabb, neg_aabb);

                                let contains = aabb_contains(neg_aabb, before_aabb);



                                if !intersects || !contains {

                                    eprintln!(

                                        "[bool][inst] ⚠️(hi) 差集结果被异常清空，跳过该负实体: refno={} neg_idx={} intersects={} contains={}",

                                        query.refno, i, intersects, contains

                                    );

                                    hi_final = before;

                                    continue;

                                }

                            }

                            _ => {

                                eprintln!(

                                    "[bool][inst] ⚠️(hi) 差集结果被清空且无法计算 AABB，跳过该负实体: refno={} neg_idx={}",

                                    query.refno, i

                                );

                                hi_final = before;

                                continue;

                            }

                        };

                    }



                    hi_final = after;

                }



                if hi_final.get_mesh().indices.is_empty() {

                    let neg_union = ManifoldRust::batch_boolean(

                        &neg_manifolds,

                        aios_core::csg::manifold::ManifoldOpType::Union,

                    );

                    let mut union_diff = pos_hi.clone();

                    union_diff.inner = union_diff.inner.difference(&neg_union.inner);

                    if !union_diff.get_mesh().indices.is_empty() {

                        hi_final = union_diff;

                    }

                }



                final_manifold = hi_final;

            }

        }

    }



    if final_manifold.get_mesh().indices.is_empty() {

        println!(

            "布尔运算失败: 结果为空（三角形=0）, refno: {} (pos_geos={}, neg_geos={})",

            query.refno,

            query.pos_geos.len(),

            neg_manifolds.len()

        );

        mark_bool_failed(query.refno).await?;

        return Ok(());

    }



    // 调试：导出布尔运算结果 OBJ

    let result_obj_path = format!("test_output/debug_{}_result.obj", query.refno);

    if let Err(e) = final_manifold.export_to_obj(&result_obj_path) {

        eprintln!("导出布尔结果 OBJ 失败: {}", e);

    } else {

        println!("✅ 导出布尔结果 OBJ: {}", result_obj_path);

    }

    

    // 统一使用 RefU64 格式（如 17496_106028），不管 sesno 是否为 0

    let refu64: aios_core::RefU64 = query.refno.into();

    let mesh_id = refu64.to_string();

    let target_path = boolean_glb_path(&mesh_id);

    ensure_parent_dir(&target_path)?;



    // 将 Manifold 共享顶点 mesh 转为重复顶点 PlantMesh（flat shading），

    // 再用标准 export_single_mesh_to_glb 导出，与 mesh_worker 输出格式一致。

    let normal_mesh = manifold_to_normal_mesh(final_manifold.get_mesh());

    use crate::fast_model::export_model::export_glb::export_single_mesh_to_glb;

    match export_single_mesh_to_glb(&normal_mesh, &target_path) {

        Ok(_) => {

            let aabb = final_manifold.get_mesh().cal_aabb();

            update_booled_result(query.refno, &mesh_id, aabb).await?;

            debug_model!("布尔运算完成: refno={} mesh={}", query.refno, mesh_id);

            return Ok(());

        }

        Err(e) => {

            eprintln!("布尔运算失败: 无法保存结果 mesh, refno: {} err: {}", query.refno, e);

            mark_bool_failed(query.refno).await

        }

    }

}



/// 对多个实例进行布尔运算（使用 Manifold，新查询流程）

pub async fn apply_insts_boolean_manifold(

    refnos: &[RefnoEnum],

    replace_exist: bool,

) -> anyhow::Result<()> {

    if refnos.is_empty() {

        return Ok(());

    }



    let filtered_refnos = filter_out_bran_refnos(refnos).await?;

    if filtered_refnos.is_empty() {

        return Ok(());

    }



    if filtered_refnos.len() != refnos.len() {

        debug_model_debug!(

            "实例布尔：跳过 BRAN 类型实体 {} 个（输入={} 过滤后={}）",

            refnos.len().saturating_sub(filtered_refnos.len()),

            refnos.len(),

            filtered_refnos.len()

        );

    }



    let refnos = filtered_refnos;

    if refnos.is_empty() {

        return Ok(());

    }



    // 先用新的批量 API 筛选出存在负实体的实例

    let neg_mapping = query_negative_entities_batch(&refnos).await?;

    let targets: Vec<RefnoEnum> = neg_mapping

        .into_iter()

        .filter_map(|(pos, negs)| if negs.is_empty() { None } else { Some(pos) })

        .collect();



    if targets.is_empty() {

        debug_model!("没有需要布尔运算的实例，输入 {} 个", refnos.len());

        return Ok(());

    }



    let queries: Vec<ManiGeoTransQuery> =

        query_manifold_boolean_operations_batch_optimized(&targets, replace_exist).await?;

    println!(

        "布尔任务数量: {} (targets={})",

        queries.len(),

        targets.len()

    );

    if queries.is_empty() {

        debug_model!("未查询到布尔运算参数，跳过");

        return Ok(());

    }



    for query in queries {

        apply_boolean_for_query(query, replace_exist).await?;

    }



    Ok(())

}



// [foyer-removal] cache-only 布尔运算函数已禁用
/*
pub async fn run_boolean_worker_from_cache_manager(
    cache_manager: &InstanceCacheManager,
    filter_refnos: Option<&HashSet<RefnoEnum>>,
) -> anyhow::Result<usize> { unimplemented!() }

pub async fn run_boolean_worker_from_cache(cache_dir: &Path) -> anyhow::Result<usize> { unimplemented!() }
*/
