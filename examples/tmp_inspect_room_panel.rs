use aios_core::{RefnoEnum, init_surreal, query_insts};
use aios_database::options::get_db_option_ext_from_path;
use aios_database::spatial_index::SqliteSpatialIndex;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let _db_option_ext = get_db_option_ext_from_path("db_options/DbOption")?;
    init_surreal().await?;

    let panel = RefnoEnum::from("24381/35798");
    let expect = RefnoEnum::from("24381/145019");
    let rows = query_insts(&[panel, expect], true).await?;
    println!("rows={}", rows.len());
    for row in &rows {
        println!(
            "refno={} owner={} has_neg={} insts={} world_aabb={:?}",
            row.refno,
            row.owner,
            row.has_neg,
            row.insts.len(),
            row.world_aabb
        );
        for inst in row.insts.iter().take(5) {
            println!(
                "  inst geo_hash={} unit_flag={} is_tubi={} geo_transform={:?}",
                inst.geo_hash, inst.unit_flag, inst.is_tubi, inst.geo_transform
            );
        }
    }

    let idx = SqliteSpatialIndex::with_default_path()?;
    if let Some(panel_aabb) = rows
        .iter()
        .find(|row| row.refno == panel)
        .and_then(|row| row.world_aabb.as_ref())
    {
        let ids = idx.query_intersect(&panel_aabb.0)?;
        println!("panel world_aabb hits={:?}", ids);
    } else {
        println!("panel world_aabb missing");
    }

    Ok(())
}
