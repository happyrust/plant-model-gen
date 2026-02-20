//! 对照同一 refno 的不同 world_transform 来源（用于定位 cache-only 布尔错位根因）
//!
//! 输出来源：
//! - instance_cache: inst_info.world_transform（以及 get_ele_world_transform）
//! - SurrealDB pe_transform: query_pe_transform().world
//! - SurrealDB pe: query_pe_world_trans()
//! - 计算路径：aios_core::get_world_transform()
//! - owner：get_named_attmap().get_owner()
//!
//! 用法（PowerShell）：
//!   $env:REFNO="17496/106064"
//!   $env:CACHE_DIR="output/AvevaMarineSample/instance_cache"
//!   cargo run --example compare_refno_world_transforms --features sqlite-index

use aios_core::{RefnoEnum, get_named_attmap, init_surreal, query_pe_transform};
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
        .unwrap_or_else(|| PathBuf::from("output/AvevaMarineSample/instance_cache"));

    // 需要 db_meta 才能从 refno 找 dbnum -> batch
    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载（请先生成 output/AvevaMarineSample/scene_tree/db_meta_info.json）")?;

    let Some(dbnum) =
        aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(refno)
    else {
        anyhow::bail!("无法从 db_meta 推导 dbnum: {}", refno);
    };

    println!(
        "🔎 compare world transforms: refno={}, dbnum={}",
        refno, dbnum
    );
    println!("   - cache_dir: {}", cache_dir.display());

    // 1) instance_cache
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

    // 2) SurrealDB 对照（需要 init_surreal）
    init_surreal().await?;

    let att = get_named_attmap(refno).await?;
    println!("\n== owner(attmap) ==");
    println!("owner: {}", att.get_owner());

    println!("\n== pe_transform cache ==");
    match query_pe_transform(refno).await? {
        Some(cache) => {
            println!("local: {:?}", cache.local);
            println!("world: {:?}", cache.world);
        }
        None => println!("miss"),
    }

    println!("\n== pe.world_trans ==");
    match aios_core::rs_surreal::query_pe_world_trans(refno).await? {
        Some(t) => println!("{:?}", t),
        None => println!("miss"),
    }

    println!("\n== aios_core::get_world_transform() ==");
    match aios_core::get_world_transform(refno).await? {
        Some(t) => println!("{:?}", t),
        None => println!("None"),
    }

    Ok(())
}
