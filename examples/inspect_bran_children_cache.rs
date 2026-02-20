//! 检查 BRAN 子元件在 instance_cache 中的状态
//!
//! 用法：
//!   $env:ROOT_REFNO="24381/103385"
//!   cargo run --example inspect_bran_children_cache

use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let root_str = env::var("ROOT_REFNO").unwrap_or_else(|_| "24381/103385".to_string());
    let root = RefnoEnum::from(root_str.as_str());

    let cache_dir = env::var("CACHE_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));

    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载")?;

    let Some(dbnum) =
        aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(root)
    else {
        anyhow::bail!("无法获取 dbnum");
    };

    println!("🔎 BRAN={} dbnum={}", root, dbnum);

    let cache =
        aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir).await?;

    let batch_ids = cache.list_batches(dbnum);
    println!("📦 共 {} 个 batch", batch_ids.len());

    let root_u64 = root.refno().0;

    // 检查所有 batch
    for batch_id in batch_ids.iter().rev() {
        println!("\n📋 Batch: {}", batch_id);
        let Some(batch) = cache.get(dbnum, batch_id).await else {
            println!("  ❌ 无法读取 batch");
            continue;
        };

        // 收集属于该 BRAN 的子元件
        println!("\n📊 inst_info_map 中属于 BRAN {} 的子元件:", root);
        let mut children_in_info: Vec<_> = batch
            .inst_info_map
            .iter()
            .filter(|(_, info)| info.owner_refno.refno().0 == root_u64)
            .collect();
        children_in_info.sort_by_key(|(k, _)| k.refno().0);

        for (refno, info) in &children_in_info {
            let inst_key = info.get_inst_key();
            let has_geos = batch.inst_geos_map.contains_key(&inst_key);
            println!(
                "  {} -> inst_key={}, has_geos={}, cata_hash={:?}",
                refno, inst_key, has_geos, info.cata_hash
            );
        }

        println!("\n📊 inst_geos_map 中的所有 key:");
        let mut geos_keys: Vec<_> = batch.inst_geos_map.keys().collect();
        geos_keys.sort();
        for key in geos_keys.iter().take(20) {
            println!("  {}", key);
        }
        if geos_keys.len() > 20 {
            println!("  ... 共 {} 个", geos_keys.len());
        }
    }

    Ok(())
}
