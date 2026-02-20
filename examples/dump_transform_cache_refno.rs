//! Dump one refno's world_transform from:
//! - instance_cache inst_info_map (raw / effective)
//! - foyer transform_cache (if hit)
//!
//! Usage (PowerShell):
//!   $env:REFNO="17496/106064"
//!   $env:CACHE_DIR="output/AvevaMarineSample/instance_cache"
//!   cargo run --example dump_transform_cache_refno --features sqlite-index

use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let refno_str = env::var("REFNO").unwrap_or_else(|_| "17496/106064".to_string());
    let refno = RefnoEnum::from(refno_str.as_str());
    anyhow::ensure!(refno.is_valid(), "无效 REFNO: {}", refno_str);

    let cache_dir = env::var("CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));

    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载（请先生成 output/scene_tree/db_meta_info.json）")?;

    let Some(dbnum) =
        aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(refno)
    else {
        anyhow::bail!("无法从 db_meta 推导 dbnum: {}", refno);
    };

    println!("🔎 dump transform: refno={}, dbnum={}", refno, dbnum);
    println!("   - cache_dir: {}", cache_dir.display());

    // 1) instance_cache（按“最新覆盖旧”找 inst_info）
    let inst_cache =
        aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
            .await
            .context("打开 InstanceCacheManager 失败")?;
    let batch_ids = inst_cache.list_batches(dbnum);
    let mut best: Option<(
        String,
        i64,
        aios_database::fast_model::instance_cache::CachedInstanceBatch,
    )> = None;
    for bid in batch_ids {
        let Some(batch) = inst_cache.get(dbnum, &bid).await else {
            continue;
        };
        if !batch.inst_info_map.contains_key(&refno) {
            continue;
        }
        match &best {
            Some((_, ts, _)) if *ts >= batch.created_at => {}
            _ => best = Some((bid, batch.created_at, batch)),
        }
    }
    match best {
        Some((bid, ts, batch)) => {
            let info = batch.inst_info_map.get(&refno).unwrap();
            println!(
                "\n== instance_cache hit batch_id={} created_at={} ==",
                bid, ts
            );
            println!("inst_info.world_transform(raw): {:?}", info.world_transform);
            println!(
                "inst_info.world_transform(effective): {:?}",
                info.get_ele_world_transform()
            );
        }
        None => {
            println!("\n== instance_cache miss ==");
        }
    }

    // 2) transform_cache（foyer）
    let trans_dir = cache_dir.join("transform_cache");
    let trans_cache =
        aios_database::fast_model::transform_cache::TransformCacheManager::new(&trans_dir)
            .await
            .with_context(|| format!("打开 TransformCacheManager 失败: {}", trans_dir.display()))?;
    let hit = trans_cache.get_world_transform(dbnum, refno).await;
    match hit {
        Some(t) => {
            println!("\n== transform_cache hit ==");
            println!("transform_cache.world_transform: {:?}", t);
        }
        None => {
            println!("\n== transform_cache miss ==");
        }
    }

    Ok(())
}
