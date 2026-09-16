use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::{RefnoEnum, SurrealQueryExt, init_surreal, model_primary_db, query_insts};
use aios_database::fast_model::export_model::model_exporter::query_geometry_instances_ext;
use aios_database::fast_model::export_model::{GltfMeshCache, collect_export_data};
use aios_database::options::get_db_option_ext_from_path;
use anyhow::Result;
use serde_json::Value;

fn mesh_bbox(mesh: &PlantMesh) -> Option<([f32; 3], [f32; 3])> {
    let mut iter = mesh.vertices.iter();
    let first = iter.next()?;
    let mut min = *first;
    let mut max = *first;
    for v in iter {
        min = min.min(*v);
        max = max.max(*v);
    }
    Some((min.to_array(), max.to_array()))
}

fn parse_refno_arg() -> RefnoEnum {
    let arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "24381/47031".to_string());
    RefnoEnum::from(arg.as_str())
}

#[tokio::main]
async fn main() -> Result<()> {
    let _db_option_ext = get_db_option_ext_from_path("db_options/DbOption-mac")?;
    init_surreal().await?;

    let refno = parse_refno_arg();
    let rows = query_insts(&[refno], true).await?;
    let mesh_dir = std::path::Path::new("assets/meshes/lod_L1");
    let mesh_cache = GltfMeshCache::new();
    println!("rows={}", rows.len());

    let export_rows = query_geometry_instances_ext(&[refno], true, false, false).await?;
    let export_data =
        collect_export_data(export_rows, &[refno], mesh_dir, false, None, true).await?;
    for comp in &export_data.components {
        if comp.refno == refno {
            println!(
                "export_component refno={} geometries={} hashes={:?}",
                comp.refno,
                comp.geometries.len(),
                comp.geometries
                    .iter()
                    .map(|g| g.geo_hash.clone())
                    .collect::<Vec<_>>()
            );
        }
    }

    for row in &rows {
        let world_mat = row.world_trans.to_matrix().as_dmat4();
        println!(
            "refno={} owner={} has_neg={} insts={} world_trans={:?} world_aabb={:?}",
            row.refno,
            row.owner,
            row.has_neg,
            row.insts.len(),
            row.world_trans,
            row.world_aabb
        );
        for (idx, inst) in row.insts.iter().enumerate() {
            let geo_mat = inst.geo_transform.to_matrix().as_dmat4();
            let combined = world_mat * geo_mat;
            println!(
                "  inst[{idx}] geo_hash={} unit_flag={} is_tubi={} geo_transform={:?}",
                inst.geo_hash, inst.unit_flag, inst.is_tubi, inst.geo_transform
            );
            let mesh = mesh_cache.load_or_get(&inst.geo_hash, mesh_dir)?;
            let world_mesh = mesh.as_ref().transform_by(&combined);
            println!(
                "    mesh_vertices={} mesh_indices={}",
                mesh.vertices.len(),
                mesh.indices.len()
            );
            if let Some((local_min, local_max)) = mesh_bbox(mesh.as_ref()) {
                println!("    local_bbox min={local_min:?} max={local_max:?}");
            }
            if let Some((world_min, world_max)) = mesh_bbox(&world_mesh) {
                println!("    world_bbox min={world_min:?} max={world_max:?}");
            }
            let sql = format!(
                "SELECT param, geo_type, unit_flag, meshed, bad, aabb != NONE as has_aabb, array::len(pts ?? []) as pts_count FROM inst_geo:`{}`;",
                inst.geo_hash
            );
            let geo_rows: Vec<Value> = model_primary_db().query_take(&sql, 0).await?;
            if let Some(first) = geo_rows.first() {
                println!("    inst_geo={}", serde_json::to_string(first)?);
            }
        }
    }

    Ok(())
}
