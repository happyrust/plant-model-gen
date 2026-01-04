//! Manifold 布尔运算模块
//!
//! 本模块提供基于 Manifold 库的几何体布尔运算功能。
//! 所有布尔运算操作均使用 Manifold 库实现，不再依赖 OpenCASCADE。

use crate::fast_model::{debug_model, debug_model_debug};
use aios_core::SurrealQueryExt;
use aios_core::csg::manifold::ManifoldRust;
use aios_core::get_db_option;
use aios_core::rs_surreal::boolean_query_optimized::query_manifold_boolean_operations_batch_optimized;
use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::{
    CataNegGroup, GmGeoData, ManiGeoTransQuery, NegInfo, query_cata_neg_boolean_groups,
    query_geom_mesh_data, query_negative_entities_batch,
};
use aios_core::{RefnoEnum, SUL_DB, utils::RecordIdExt};
use glam::DMat4;
use std::path::{Path, PathBuf};
use std::{fs, io};

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

fn mesh_base_dir() -> PathBuf {
    get_db_option()
        .meshes_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/meshes"))
}

/// 从文件加载网格数据
///
/// # 参数
///
/// * `id` - 网格文件的ID
///
/// # 返回值
///
/// 返回 `anyhow::Result<PlantMesh>` 表示加载是否成功以及加载的网格数据
#[inline]
fn load_mesh(id: &str) -> anyhow::Result<PlantMesh> {
    let base_dir = mesh_base_dir();
    let mesh_path = build_lod_mesh_path(&base_dir, id);
    
    use crate::fast_model::export_model::import_glb::import_glb_to_mesh;
    let mesh = import_glb_to_mesh(&mesh_path)?;
    debug_model_debug!(
        "[布尔] 加载 mesh (from .glb): {} (vertices={}, triangles={})",
        mesh_path.display(),
        mesh.vertices.len(),
        mesh.indices.len() / 3
    );
    Ok(mesh)
}

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
    let mesh = load_mesh(id)?;
    let manifold = ManifoldRust::convert_to_manifold(mesh, mat, more_precision);
    Ok(manifold)
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
    crate::fast_model::utils::save_inst_relate_bool(refno, None, "Failed", "bool_mesh").await;
    Ok(())
}

/// 更新布尔运算结果到数据库
///
/// 仅写入 inst_relate_bool（状态 + mesh_id），AABB 已写入 inst_relate_aabb
async fn update_booled_result(
    refno: RefnoEnum,
    mesh_id: &str,
    mesh: &PlantMesh,
) -> anyhow::Result<()> {
    use aios_core::gen_bytes_hash;
    use dashmap::DashMap;
    
    // 计算布尔后的 aabb
    let aabb = mesh.cal_aabb();
    
    if let Some(aabb) = aabb {
        // 使用 hash 格式存储 AABB（与 mesh_generate.rs 中的模式一致）
        let aabb_hash = gen_bytes_hash(&aabb);
        
        // 保存 AABB 记录到 SurrealDB
        let aabb_map = DashMap::new();
        aabb_map.insert(aabb_hash.to_string(), aabb);
        crate::fast_model::utils::save_aabb_to_surreal(&aabb_map).await;
        let inst_aabb_map = DashMap::new();
        inst_aabb_map.insert(refno, aabb_hash.to_string());
        crate::fast_model::utils::save_inst_relate_aabb(&inst_aabb_map, "bool_mesh").await;
        
        crate::fast_model::utils::save_inst_relate_bool(refno, Some(mesh_id), "Success", "bool_mesh")
            .await;
    } else {
        crate::fast_model::utils::save_inst_relate_bool(refno, Some(mesh_id), "Success", "bool_mesh")
            .await;
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

    for g in params {
        // 收集当前实例涉及的所有几何，批量查询 mesh 数据
        let geom_refnos: Vec<RefnoEnum> = g.boolean_group.iter().flatten().cloned().collect();
        let gms: Vec<GmGeoData> = query_geom_mesh_data(g.refno, &geom_refnos).await?;

        let mut update_sql = String::new();
        for bg in g.boolean_group {
            let Some(pos) = gms.iter().find(|x| x.geom_refno == bg[0]) else {
                crate::fast_model::utils::save_inst_relate_bool(g.refno, None, "Failed", "cata_bool")
                    .await;
                continue;
            };

            debug_model_debug!("加载 catalog 正实体 mesh: {}", pos.id.to_mesh_id());
            let Ok(mut pos_manifold) = load_manifold(
                &pos.id.to_mesh_id(),
                pos.trans.0.to_matrix().as_dmat4(),
                false,
            ) else {
                println!("布尔运算失败: 无法加载正实体 manifold, refno: {}", &g.refno);
                crate::fast_model::utils::save_inst_relate_bool(g.refno, None, "Failed", "cata_bool")
                    .await;
                continue;
            };

            let mut neg_manifolds = Vec::new();
            for &neg in bg.iter().skip(1) {
                let Some(neg_geo) = gms.iter().find(|x| x.geom_refno == neg) else {
                    continue;
                };
                let m = neg_geo.trans.0.to_matrix().as_dmat4();
                if let Ok(manifold) = load_manifold(&neg_geo.id.to_mesh_id(), m, true) {
                    neg_manifolds.push(manifold);
                }
            }

            // 即使没有负实体，也标记已处理，避免重复计算
            let new_id = g.refno.hash_with_another_refno(bg[0]);
            let final_manifold = pos_manifold.batch_boolean_subtract(&neg_manifolds);
            let mesh = PlantMesh::from(&final_manifold);
            // let target_path = boolean_mesh_path(&new_id.to_string());
            let target_path = boolean_glb_path(&new_id.to_string());
            ensure_parent_dir(&target_path)?;

            // if mesh.ser_to_file(&target_path).is_ok() {
            if export_single_mesh_to_glb(&mesh, &target_path).is_ok() {
                // let obj_path = boolean_obj_path(&new_id.to_string());
                // if let Err(e) = mesh.export_obj(false, obj_path.to_string_lossy().as_ref()) {
                //     eprintln!("导出 OBJ 失败: refno={} err={}", g.refno, e);
                // }

                update_sql.push_str(&format!(
                    "create inst_geo:⟨{}⟩ set meshed = true, aabb = {};",
                    new_id,
                    &pos.aabb_id.to_raw()
                ));
                let relate_sql = format!(
                    "relate {}->geo_relate->inst_geo:⟨{}⟩ set geom_refno=pe:⟨{}⟩, geo_type='Pos', trans=trans:⟨0⟩, visible = true;",
                    &g.inst_info_id.to_raw(),
                    new_id,
                    format!("{}_b", bg[0]),
                );
                update_sql.push_str(relate_sql.as_str());
                
                // 将原正实体的 geo_type 从 Pos 更新为 CatePos（隐含的原始正实体）
                // 并设置 visible=false，使其在查询时被排除
                let hide_original_sql = format!(
                    "UPDATE {}->geo_relate SET geo_type = 'CatePos', visible = false WHERE geom_refno = pe:⟨{}⟩ AND geo_type = 'Pos';",
                    &g.inst_info_id.to_raw(),
                    bg[0],
                );
                update_sql.push_str(&hide_original_sql);
            } else {
                crate::fast_model::utils::save_inst_relate_bool(g.refno, None, "Failed", "cata_bool")
                    .await;
            }
        }

        if !update_sql.is_empty() {
            SUL_DB.query(update_sql).await?;
        }
    }

    debug_model!("元件库的负实体计算{:?}完成", refnos);
    Ok(())
}

async fn apply_boolean_for_query(
    query: ManiGeoTransQuery,
    replace_exist: bool,
) -> anyhow::Result<()> {
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

    let mut pos_manifolds = Vec::new();
    for (pos_id, pos_t) in query.pos_geos.iter() {
        let pos_mesh_id = pos_id.to_mesh_id();
        // 正实体使用局部变换
        let pos_local_mat = pos_t.0.to_matrix().as_dmat4();
        debug_model_debug!(
            "加载正实体 mesh: {} (应用局部变换)",
            pos_mesh_id
        );
        if let Ok(manifold) = load_manifold(&pos_mesh_id, pos_local_mat, false) {
            pos_manifolds.push(manifold);
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

    // 调试：导出正实体 OBJ
    let pos_mesh = PlantMesh::from(&pos_manifold);
    let pos_obj_path = format!("test_output/debug_{}_pos.obj", query.refno);
    if let Err(e) = pos_mesh.export_obj(false, &pos_obj_path) {
        eprintln!("导出正实体 OBJ 失败: {}", e);
    } else {
        println!("✅ 导出正实体 OBJ: {}", pos_obj_path);
    }

    // 计算正实体世界坐标系的逆矩阵，用于将负实体转换到正实体的相对坐标系
    let inverse_pos_world = pos_world_mat.inverse();
    let mut neg_manifolds = Vec::new();
    for (_, _carrier_wt, neg_infos) in query.neg_ts.iter() {
        for NegInfo {
            id, geo_local_trans, aabb, carrier_world_trans, ..
        } in neg_infos.iter().cloned()
        {
            if aabb.is_none() {
                continue;
            }

            // 使用 NegInfo 中的 carrier_world_trans（每个负实体自己的载体世界变换）
            // 而不是 neg_ts 中的 carrier_wt（可能是虚拟的单位矩阵）
            let carrier_world_mat = carrier_world_trans
                .as_ref()
                .map(|t| t.0.to_matrix().as_dmat4())
                .unwrap_or(DMat4::IDENTITY);

            // 计算负实体相对于正实体坐标系的变换矩阵
            // 相对变换 = inverse(正实体世界坐标) × 负实体世界坐标
            // 负实体世界坐标 = carrier_world_mat × geo_local_trans
            let neg_world_mat = carrier_world_mat * geo_local_trans.0.to_matrix().as_dmat4();
            let relative_mat = inverse_pos_world * neg_world_mat;
            
            // 调试：打印变换矩阵信息
            println!("[变换调试] neg_id={}", id.to_mesh_id());
            println!("  pos_world_trans: {:?}", pos_world_mat.col(3));
            println!("  carrier_world: {:?}", carrier_world_mat.col(3));
            println!("  geo_local: {:?}", geo_local_trans.0.to_matrix().as_dmat4().col(3));
            println!("  relative: {:?}", relative_mat.col(3));

            debug_model_debug!("加载负实体 mesh: {} (相对于正实体坐标系)", id.to_mesh_id());

            if let Ok(manifold) = load_manifold(&id.to_mesh_id(), relative_mat, true) {
                neg_manifolds.push(manifold);
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
    let neg_mesh = PlantMesh::from(&neg_union);
    let neg_obj_path = format!("test_output/debug_{}_neg.obj", query.refno);
    if let Err(e) = neg_mesh.export_obj(false, &neg_obj_path) {
        eprintln!("导出负实体 OBJ 失败: {}", e);
    } else {
        println!("✅ 导出负实体 OBJ: {}", neg_obj_path);
    }

    let final_manifold = pos_manifold.batch_boolean_subtract(&neg_manifolds);
    let mesh = PlantMesh::from(&final_manifold);
    
    // 调试：导出布尔运算结果 OBJ
    let result_obj_path = format!("test_output/debug_{}_result.obj", query.refno);
    if let Err(e) = mesh.export_obj(false, &result_obj_path) {
        eprintln!("导出布尔结果 OBJ 失败: {}", e);
    } else {
        println!("✅ 导出布尔结果 OBJ: {}", result_obj_path);
    }
    
    let mesh_id = if query.sesno == 0 {
        query.refno.to_string()
    } else {
        format!("{}_{}", query.refno, query.sesno)
    };
    let target_path = boolean_glb_path(&mesh_id);
    ensure_parent_dir(&target_path)?;

    use crate::fast_model::export_model::export_glb::export_single_mesh_to_glb;
    // if mesh.ser_to_file(&target_path).is_ok() {
    if export_single_mesh_to_glb(&mesh, &target_path).is_ok() {
        update_booled_result(query.refno, &mesh_id, &mesh).await?;
        debug_model!("布尔运算完成: refno={} mesh={}", query.refno, mesh_id);
        return Ok(());
    }

    println!("布尔运算失败: 无法保存结果 mesh, refno: {}", query.refno);
    mark_bool_failed(query.refno).await
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
        query_manifold_boolean_operations_batch_optimized(&targets).await?;
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
