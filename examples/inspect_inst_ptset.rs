//! 检查 inst_info_map 中子元件的 ptset_map 数据
//!
//! 用法：
//!   REFNO="24381/103386" CACHE_DIR="output/instance_cache" cargo run --example inspect_inst_ptset

use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let refno_str = env::var("REFNO").unwrap_or_else(|_| "24381/103386".to_string());
    let refno = RefnoEnum::from(refno_str.as_str());
    anyhow::ensure!(refno.is_valid(), "无效 REFNO: {}", refno_str);

    let cache_dir = env::var("CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));

    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载")?;

    let Some(dbnum) =
        aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(refno)
    else {
        anyhow::bail!("无法从 db_meta 推导 dbnum: {}", refno);
    };

    println!("🔎 检查 ptset_map: refno={}, dbnum={}", refno, dbnum);

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;

    let batch_ids = cache.list_batches(dbnum);
    let want_u64 = refno.refno();

    for batch_id in batch_ids.iter().rev() {
        let Some(batch) = cache.get(dbnum, batch_id).await else {
            continue;
        };

        for (k, info) in batch.inst_info_map.iter() {
            if k.refno() != want_u64 {
                continue;
            }

            println!("\n== inst_info batch_id={} ==", batch_id);
            println!("refno={}", info.refno);
            println!("owner_refno={}", info.owner_refno);
            println!("owner_type={}", info.owner_type);
            println!("cata_hash={:?}", info.cata_hash);

            let wt = info.get_ele_world_transform();
            println!(
                "world_transform: t=({:.3},{:.3},{:.3})",
                wt.translation.x, wt.translation.y, wt.translation.z
            );
            println!(
                "world_transform: s=({:.3},{:.3},{:.3})",
                wt.scale.x, wt.scale.y, wt.scale.z
            );

            println!("\nptset_map.len()={}", info.ptset_map.len());
            for (num, axis) in &info.ptset_map {
                println!(
                    "  [{}] pt=({:.3},{:.3},{:.3}) dir={:?}",
                    num,
                    axis.pt.0.x,
                    axis.pt.0.y,
                    axis.pt.0.z,
                    axis.dir
                        .as_ref()
                        .map(|d| format!("({:.3},{:.3},{:.3})", d.0.x, d.0.y, d.0.z))
                );

                // 变换到世界坐标
                let transformed = axis.transformed(&wt);
                println!(
                    "      -> world_pt=({:.3},{:.3},{:.3})",
                    transformed.pt.0.x, transformed.pt.0.y, transformed.pt.0.z
                );
            }

            return Ok(());
        }
    }

    println!("⚠️ 未找到 refno={} 的 inst_info", refno);
    Ok(())
}
