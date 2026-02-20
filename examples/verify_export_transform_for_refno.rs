//! 校验：cache 中的 (inst_info.world_transform × inst_geos.transform) 应用到 GLB 后的 AABB
//! 是否与最终导出的 OBJ 分组 AABB 一致。
//!
//! 用法（PowerShell）：
//!   $env:REFNO="24381/103388"
//!   $env:OBJ="output/Copy-of-1RCS184YP-YK_109VP.obj"
//!   cargo run --example verify_export_transform_for_refno

use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

fn bbox_of_points(pts: &[(f32, f32, f32)]) -> Option<([f32; 3], [f32; 3])> {
    if pts.is_empty() {
        return None;
    }
    let mut mn = [pts[0].0, pts[0].1, pts[0].2];
    let mut mx = mn;
    for &(x, y, z) in pts.iter().skip(1) {
        mn[0] = mn[0].min(x);
        mn[1] = mn[1].min(y);
        mn[2] = mn[2].min(z);
        mx[0] = mx[0].max(x);
        mx[1] = mx[1].max(y);
        mx[2] = mx[2].max(z);
    }
    Some((mn, mx))
}

fn bbox_of_mesh(mesh: &aios_core::shape::pdms_shape::PlantMesh) -> Option<([f32; 3], [f32; 3])> {
    if mesh.vertices.is_empty() {
        return None;
    }
    let mut mn = [mesh.vertices[0].x, mesh.vertices[0].y, mesh.vertices[0].z];
    let mut mx = mn;
    for v in mesh.vertices.iter().skip(1) {
        mn[0] = mn[0].min(v.x);
        mn[1] = mn[1].min(v.y);
        mn[2] = mn[2].min(v.z);
        mx[0] = mx[0].max(v.x);
        mx[1] = mx[1].max(v.y);
        mx[2] = mx[2].max(v.z);
    }
    Some((mn, mx))
}

fn parse_obj_group_bbox(
    obj_path: &PathBuf,
    group: &str,
) -> Result<Option<([f32; 3], [f32; 3], usize)>> {
    let s = std::fs::read_to_string(obj_path)
        .with_context(|| format!("读取 OBJ 失败: {}", obj_path.display()))?;
    let mut cur: Option<&str> = None;
    let mut pts: Vec<(f32, f32, f32)> = Vec::new();
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("g ") {
            cur = Some(rest.trim());
            continue;
        }
        if cur != Some(group) {
            continue;
        }
        if let Some(rest) = line.strip_prefix("v ") {
            let mut it = rest.split_whitespace();
            let x: f32 = it.next().unwrap_or("0").parse().unwrap_or(0.0);
            let y: f32 = it.next().unwrap_or("0").parse().unwrap_or(0.0);
            let z: f32 = it.next().unwrap_or("0").parse().unwrap_or(0.0);
            pts.push((x, y, z));
        }
    }
    let Some((mn, mx)) = bbox_of_points(&pts) else {
        return Ok(None);
    };
    Ok(Some((mn, mx, pts.len())))
}

fn fmt_bbox(mn: [f32; 3], mx: [f32; 3]) -> String {
    format!(
        "min=({:.3},{:.3},{:.3}) max=({:.3},{:.3},{:.3}) size=({:.3},{:.3},{:.3})",
        mn[0],
        mn[1],
        mn[2],
        mx[0],
        mx[1],
        mx[2],
        mx[0] - mn[0],
        mx[1] - mn[1],
        mx[2] - mn[2]
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let refno_str = env::var("REFNO").unwrap_or_else(|_| "24381/103388".to_string());
    let refno = RefnoEnum::from(refno_str.as_str());
    anyhow::ensure!(refno.is_valid(), "无效 REFNO: {}", refno_str);

    let obj_path = env::var("OBJ")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/Copy-of-1RCS184YP-YK_109VP.obj"));

    // cache
    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载（请先生成 output/scene_tree/db_meta_info.json）")?;
    let Some(dbnum) =
        aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(refno)
    else {
        anyhow::bail!("无法从 db_meta 推导 dbnum: {}", refno);
    };

    let cache_dir = PathBuf::from("output/instance_cache");
    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;
    let batch_ids = cache.list_batches(dbnum);
    anyhow::ensure!(!batch_ids.is_empty(), "cache 中没有 batch: dbnum={}", dbnum);

    // 找“最新命中”的 batch（与 inspect_cache_geom_refno 同语义）
    let want_u64 = refno.refno();
    let mut best: Option<(
        String,
        i64,
        aios_database::fast_model::instance_cache::CachedInstanceBatch,
    )> = None;
    for bid in batch_ids {
        let Some(batch) = cache.get(dbnum, &bid).await else {
            continue;
        };
        let hit = batch
            .inst_geos_map
            .values()
            .any(|g| g.refno.refno() == want_u64 && !g.insts.is_empty());
        if !hit {
            continue;
        }
        match &best {
            Some((_, ts, _)) if *ts >= batch.created_at => {}
            _ => best = Some((bid, batch.created_at, batch)),
        }
    }
    let Some((batch_id, _ts, batch)) = best else {
        anyhow::bail!("cache 中未找到该 refno 的 inst_geos: {}", refno);
    };

    let info = batch
        .inst_info_map
        .iter()
        .find(|(k, _)| k.refno() == want_u64)
        .map(|(_, v)| v)
        .context("cache 中未找到 inst_info（world_transform）")?;

    let geos = batch
        .inst_geos_map
        .values()
        .find(|g| g.refno.refno() == want_u64 && !g.insts.is_empty())
        .context("cache 中未找到 inst_geos")?;

    println!("refno={} dbnum={} batch_id={}", refno, dbnum, batch_id);

    // 仅验证第一个 inst（本 case 两个 inst transform 相同）
    let inst0 = geos.insts.first().context("inst_geos.insts 为空")?;

    let mesh_dir = PathBuf::from("assets/meshes/lod_L1");
    let glb = mesh_dir.join(format!("{}_L1.glb", inst0.geo_hash));
    let base_mesh = aios_database::fast_model::export_model::import_glb::import_glb_to_mesh(&glb)
        .with_context(|| format!("加载 GLB 失败: {}", glb.display()))?;

    let world = aios_core::rs_surreal::geometry_query::PlantTransform::from(info.world_transform);
    let combined = world.to_matrix().as_dmat4() * inst0.transform.to_matrix().as_dmat4();
    let transformed = base_mesh.transform_by(&combined);

    let Some((mn_t, mx_t)) = bbox_of_mesh(&transformed) else {
        anyhow::bail!("变换后 mesh 为空");
    };
    println!("cache->(world*local) AABB: {}", fmt_bbox(mn_t, mx_t));

    // OBJ group AABB（group 名为 refno 的 '_' 形式）
    let group = refno.to_string().replace('/', "_");
    let Some((mn_o, mx_o, vcnt)) = parse_obj_group_bbox(&obj_path, &group)? else {
        anyhow::bail!("OBJ 中未找到 group: {}", group);
    };
    println!("obj group verts={} AABB: {}", vcnt, fmt_bbox(mn_o, mx_o));

    Ok(())
}
