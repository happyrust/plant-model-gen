//! 调试 103388 导出问题
//!
//! 用法：cargo run --example debug_export_103388

use anyhow::Result;
use aios_core::RefnoEnum;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化
    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()?;

    let refnos = vec![
        RefnoEnum::from("24381/103385"),
        RefnoEnum::from("24381/103386"),
        RefnoEnum::from("24381/103387"),
        RefnoEnum::from("24381/103388"),
    ];

    let cache_dir = PathBuf::from("output/instance_cache");

    println!("🔍 调用 query_geometry_instances_ext_from_cache...\n");

    let geom_insts = aios_database::fast_model::export_model::model_exporter::query_geometry_instances_ext_from_cache(
        &refnos,
        &cache_dir,
        true,  // enable_holes
        false, // include_negative
        true,  // verbose
    ).await?;

    println!("\n📊 返回的 GeomInstQuery 列表:");
    for (i, inst) in geom_insts.iter().enumerate() {
        println!("  [{}] refno={}, owner={}, insts.len()={}, has_neg={}",
            i, inst.refno, inst.owner, inst.insts.len(), inst.has_neg);
        for (j, sub) in inst.insts.iter().enumerate() {
            println!("      inst[{}]: geo_hash={}, is_tubi={}",
                j, sub.geo_hash, sub.is_tubi);
        }
    }

    Ok(())
}
