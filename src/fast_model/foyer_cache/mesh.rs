//! foyer cache 专用的 Mesh 生成 Worker（不访问 SurrealDB）
//!
//! 该模块从 `fast_model::mesh_generate` 中抽离 cache-only 的 mesh 生成入口，
//! 以便：
//! - 在 orchestrator 中通过 `FoyerCacheContext` 统一编排；
//! - 将 cache-only 路径的语义（不回退 DB、替换策略等）集中维护。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use aios_core::geometry::csg::generate_csg_mesh;
use aios_core::mesh_precision::MeshPrecisionSettings;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;

use crate::fast_model::export_model::export_glb::export_single_mesh_to_glb;
use crate::fast_model::foyer_cache::FoyerCacheContext;
use crate::fast_model::{debug_model_debug, debug_model_warn};
use crate::options::MeshFormat;

/// 基于 foyer 缓存的 Mesh 生成 Worker（不访问 SurrealDB）
///
/// # 语义约定
/// - 不回退 SurrealDB；所有几何参数来自 `instance_cache`。
/// - cache-only 路径下也支持 `--regen-model` 的“替换”语义：通过环境变量 `FORCE_REPLACE_MESH` 控制。
pub async fn run_mesh_worker(
    ctx: &FoyerCacheContext,
    mesh_dir: &Path,
    precision: &MeshPrecisionSettings,
    mesh_formats: &[MeshFormat],
) -> anyhow::Result<usize> {
    run_mesh_worker_from_cache_manager(ctx.cache(), mesh_dir, precision, mesh_formats).await
}

/// 兼容入口：直接传入 `InstanceCacheManager`（旧 orchestrator/调用点可继续使用）。
pub async fn run_mesh_worker_from_cache_manager(
    cache_manager: &crate::fast_model::instance_cache::InstanceCacheManager,
    mesh_dir: &Path,
    precision: &MeshPrecisionSettings,
    mesh_formats: &[MeshFormat],
) -> anyhow::Result<usize> {
    if !mesh_dir.exists() {
        std::fs::create_dir_all(mesh_dir)?;
    }

    let dbnums = cache_manager.list_dbnums();
    if dbnums.is_empty() {
        println!("[mesh_worker_cache] 缓存为空，跳过 Mesh 生成");
        return Ok(0);
    }

    let mut unique_geo: HashMap<u64, (PdmsGeoParam, bool)> = HashMap::new();

    for dbnum in dbnums {
        let batch_ids = cache_manager.list_batches(dbnum);
        for batch_id in batch_ids {
            if let Some(batch) = cache_manager.get(dbnum, &batch_id).await {
                for geos_data in batch.inst_geos_map.values() {
                    for inst in &geos_data.insts {
                        unique_geo.entry(inst.geo_hash).or_insert_with(|| {
                            (inst.geo_param.clone(), inst.geo_param.is_reuse_unit())
                        });
                    }
                }
            }
        }
    }

    if unique_geo.is_empty() {
        println!("[mesh_worker_cache] 缓存中未找到几何参数，跳过 Mesh 生成");
        return Ok(0);
    }

    let lod_dir = mesh_dir.join(format!("lod_{:?}", precision.default_lod));
    if !lod_dir.exists() {
        std::fs::create_dir_all(&lod_dir)?;
    }

    // cache-only 路径下也需要支持 `--regen-model` 的“替换”语义。
    // 统一沿用 orchestrator/cli 的 FORCE_REPLACE_MESH 环境变量。
    let force_replace = std::env::var("FORCE_REPLACE_MESH")
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false);

    let mut processed = 0usize;
    let mut seen: HashSet<u64> = HashSet::new();
    for (geo_hash, (geo_param, unit_flag)) in unique_geo {
        if !seen.insert(geo_hash) {
            continue;
        }

        // 标准单位几何体（1/2/3）使用内置函数直接生成 GLB，
        // 不依赖实例的 geo_param（避免不同实例尺寸覆盖）。
        if crate::fast_model::reuse_unit::is_builtin_unit_geo_hash(geo_hash) {
            let mesh_id = geo_hash.to_string();
            let mesh_filename = format!("{}_{:?}", mesh_id, precision.default_lod);
            let glb_path = lod_dir.join(&mesh_filename).with_extension("glb");
            if glb_path.exists() && !force_replace {
                continue;
            }
            use aios_core::geometry::csg::{unit_box_mesh, unit_cylinder_mesh, unit_sphere_mesh};
            use aios_core::mesh_precision::LodMeshSettings;
            let unit_mesh = match geo_hash {
                1 => unit_box_mesh(),
                2 => unit_cylinder_mesh(&LodMeshSettings::default(), false),
                3 => unit_sphere_mesh(),
                _ => unreachable!(),
            };
            if let Err(e) = export_single_mesh_to_glb(&unit_mesh, &glb_path) {
                debug_model_warn!(
                    "[mesh_worker_cache] 生成内置 unit mesh GLB 失败: {} - {}",
                    mesh_id, e
                );
            } else {
                debug_model_debug!("[mesh_worker_cache] 生成内置 unit mesh: {}", mesh_id);
                processed += 1;
            }
            continue;
        }

        let mesh_id = geo_hash.to_string();
        let mesh_filename = format!("{}_{:?}", mesh_id, precision.default_lod);
        let glb_path = lod_dir.join(&mesh_filename).with_extension("glb");
        if glb_path.exists() && !force_replace {
            continue;
        }
        if force_replace {
            let _ = std::fs::remove_file(&glb_path);
            let _ = std::fs::remove_file(lod_dir.join(&mesh_filename).with_extension("obj"));
        }

        let geo_type_name = geo_param.type_name();
        let profile = precision.profile_for_geo(geo_type_name);
        let non_scalable_geo = precision.is_non_scalable_geo(geo_type_name);
        let lod_settings = profile.csg_settings;

        let geo_param_for_mesh = if unit_flag {
            geo_param.to_unit_param()
        } else {
            geo_param
        };

        match generate_csg_mesh(&geo_param_for_mesh, &lod_settings, non_scalable_geo, false, None) {
            Some(csg_mesh) => {
                let mesh_base_path = lod_dir.join(&mesh_filename);
                let glb_path = mesh_base_path.with_extension("glb");
                if let Err(e) = export_single_mesh_to_glb(&csg_mesh.mesh, &glb_path) {
                    debug_model_warn!(
                        "[mesh_worker_cache] 生成 GLB 失败: {} - {}",
                        mesh_id,
                        e
                    );
                    continue;
                }

                #[cfg(feature = "convex-decomposition")]
                {
                    let precompute = std::env::var("AIOS_PRECOMPUTE_CONVEX")
                        .ok()
                        .map(|v| {
                            let v = v.trim();
                            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
                        })
                        .unwrap_or(false);

                    if precompute {
                        if let Err(e) = crate::fast_model::convex_decomp::build_and_save_convex_from_glb(
                            mesh_dir,
                            &mesh_id,
                        )
                        .await
                        {
                            debug_model_warn!(
                                "[convex] 预计算失败(cache-only): geo_hash={}, error={}",
                                mesh_id,
                                e
                            );
                        }
                    }
                }

                if mesh_formats.contains(&MeshFormat::Obj) {
                    let obj_path = mesh_base_path.with_extension("obj");
                    if let Err(e) = csg_mesh.mesh.export_obj(false, obj_path.to_str().unwrap()) {
                        debug_model_warn!(
                            "[mesh_worker_cache] 生成 OBJ 失败: {} - {}",
                            mesh_id,
                            e
                        );
                    }
                }
                if unit_flag {
                    debug_model_debug!("[mesh_worker_cache] unit mesh: {}", mesh_id);
                }
                processed += 1;
            }
            None => {
                debug_model_warn!(
                    "[mesh_worker_cache] CSG mesh 返回 None: {} ({})",
                    mesh_id,
                    geo_type_name
                );
            }
        }
    }

    println!("[mesh_worker_cache] Mesh 生成完成: {} 个", processed);
    Ok(processed)
}

/// 兼容入口：直接传入 cache_dir（内部自建 `InstanceCacheManager`）。
pub async fn run_mesh_worker_from_cache(
    cache_dir: &Path,
    mesh_dir: &Path,
    precision: &MeshPrecisionSettings,
    mesh_formats: &[MeshFormat],
) -> anyhow::Result<usize> {
    let ctx = FoyerCacheContext::from_cache_dir(cache_dir).await?;
    run_mesh_worker(&ctx, mesh_dir, precision, mesh_formats).await
}

