//! 检查 foyer instance_cache 中某个 refno 的 inst_geos 细节（geo_hash / geo_param / transform）。
//!
//! 用途：
//! - 排查“导出尺寸不对/缩放被重复应用”的问题（例如 RTOR 160mm 被放大到 25600mm）。
//!
//! 运行示例（PowerShell）：
//!   $env:REFNO="24381/129922"
//!   $env:CACHE_DIR="output/instance_cache"
//!   cargo run --example inspect_cache_geom_refno

use anyhow::{Context, Result};
use aios_core::RefnoEnum;
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let refno_str = env::var("REFNO").unwrap_or_else(|_| "24381/129922".to_string());
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

    println!("🔎 检查 inst_geos: refno={}, dbnum={}", refno, dbnum);
    println!("   - cache_dir: {}", cache_dir.display());

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;

    let batch_ids = cache.list_batches(dbnum);
    println!("   - batches: {}", batch_ids.len());

    let want_u64 = refno.refno();

    // 选择“最新”的命中 batch（按 created_at），避免误读旧缓存。
    let mut best: Option<(String, i64, aios_database::fast_model::instance_cache::CachedInstanceBatch)> =
        None;

    for batch_id in batch_ids {
        let Some(batch) = cache.get(dbnum, &batch_id).await else {
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
            _ => best = Some((batch_id, batch.created_at, batch)),
        }
    }

    let Some((batch_id, _ts, batch)) = best else {
        println!("⚠️ 未在 cache 中找到该 refno 的 inst_geos（可能未生成或已被清理）");
        return Ok(());
    };

    // inst_info_map: key 可能是 Refno 或 SesRef([refno,sesno])，这里按 RefU64 归一化。
    let info = batch
        .inst_info_map
        .iter()
        .find(|(k, _)| k.refno() == want_u64)
        .map(|(_, v)| v);

    let geos = batch
        .inst_geos_map
        .values()
        .find(|g| g.refno.refno() == want_u64 && !g.insts.is_empty());

    println!("\n== latest hit batch_id={} created_at={} ==", batch_id, batch.created_at);
    if let Some(info) = info {
        println!(
            "inst_info: refno={} sesno={} visible={} owner={:?}/{:?}",
            info.refno, info.sesno, info.visible, info.owner_refno, info.owner_type
        );
        println!("inst_info.world_transform: {:?}", info.world_transform);
    }

    if let Some(geos) = geos {
        println!(
            "inst_geos: type_name={} inst_key={} inst_cnt={}",
            geos.type_name,
            geos.inst_key,
            geos.insts.len()
        );
        for (i, inst) in geos.insts.iter().enumerate() {
            println!(
                "  - inst[{}] geo_hash={} unit_flag={} geo_type={:?}",
                i, inst.geo_hash, inst.unit_flag, inst.geo_type
            );
            println!("    geo_param: {:?}", inst.geo_param);
            println!("    transform: {:?}", inst.transform);
            println!(
                "    scale: [{:.6}, {:.6}, {:.6}]",
                inst.transform.scale.x, inst.transform.scale.y, inst.transform.scale.z
            );
        }
    }

    Ok(())
}
