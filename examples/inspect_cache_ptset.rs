//! 调试：检查 foyer instance_cache 中某些 refno 的 ptset_map 以及 ARRI/LEAV 点编号。
//!
//! 用法（PowerShell）：
//!   $env:DBNUM="7997"
//!   $env:REFNOS="24381/145019,24381/145032"
//!   $env:CACHE_DIR="output/AvevaMarineSample/instance_cache"
//!   cargo run --example inspect_cache_ptset
use anyhow::{Context, Result};
use aios_core::RefnoEnum;
use std::env;
use std::path::PathBuf;

fn parse_u32_env(name: &str) -> Option<u32> {
    env::var(name).ok().and_then(|s| s.trim().parse::<u32>().ok())
}

fn parse_refnos_env() -> Vec<RefnoEnum> {
    let raw = env::var("REFNOS").unwrap_or_default();
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(RefnoEnum::from)
        .filter(|r| r.is_valid())
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let dbnum = parse_u32_env("DBNUM").context("请设置 DBNUM，例如 7997")?;
    let refnos = parse_refnos_env();
    anyhow::ensure!(!refnos.is_empty(), "请设置 REFNOS，例如 24381/145019,24381/145020");

    // 读取 ARRI/LEAV 需要 SurrealDB（只读）
    let _ = aios_core::init_surreal().await;

    let cache_dir = env::var("CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/AvevaMarineSample/instance_cache"));

    println!("dbnum={}", dbnum);
    println!("cache_dir={}", cache_dir.display());

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;

    let batch_ids = cache.list_batches(dbnum);
    anyhow::ensure!(!batch_ids.is_empty(), "cache 中没有 batch: dbnum={}", dbnum);
    let latest = batch_ids
        .last()
        .cloned()
        .context("batch_ids 为空（不应发生）")?;
    println!("latest_batch_id={}", latest);

    let batch = cache
        .get(dbnum, &latest)
        .await
        .context("读取 batch 失败")?
        .context("batch 不存在")?;

    for r in refnos {
        let want = r.refno();
        let hit = batch
            .inst_info_map
            .iter()
            .find(|(k, _)| k.refno() == want)
            .map(|(k, v)| (*k, v));

        println!("\n=== refno={} ===", r);
        let att = aios_core::get_named_attmap(r).await.unwrap_or_default();
        let arri = att.get_i32("ARRI").unwrap_or(0);
        let leav = att.get_i32("LEAV").unwrap_or(0);
        println!("ARRI={} LEAV={}", arri, leav);

        let Some((k, info)) = hit else {
            println!("cache: inst_info_map 未命中（key 可能不存在）");
            continue;
        };

        println!("cache_key_refno={}", k);
        println!("ptset_map_len={}", info.ptset_map.len());

        // 打印前若干点，观察 number 与 key 的关系
        let mut shown = 0usize;
        for (pt_key, pt) in info.ptset_map.iter() {
            if shown >= 12 {
                break;
            }
            println!(
                "  pt_key={} number={} pt=({:.3},{:.3},{:.3})",
                pt_key,
                pt.number,
                pt.pt.x,
                pt.pt.y,
                pt.pt.z
            );
            shown += 1;
        }
    }

    Ok(())
}
