//! 检查 foyer instance_cache 中某个 refno 的缓存情况（是否有 inst_info / inst_geos）。
//!
//! 用途：
//! - 排查 “SQLite 有 AABB，但 cache 找不到几何实例” 的原因
//! - 确认某个 refno 是否真的被写入到 CachedInstanceBatch.inst_geos_map
//!
//! 运行示例：
//!   set REFNO=17496/199296
//!   set CACHE_DIR=output/instance_cache
//!   cargo run --example inspect_cache_refno --features sqlite-index

use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let refno_str = env::var("REFNO").unwrap_or_else(|_| "17496/199296".to_string());
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

    println!("🔎 检查 cache: refno={}, dbnum={}", refno, dbnum);
    println!("   - cache_dir: {}", cache_dir.display());

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;

    let batch_ids = cache.list_batches(dbnum);
    println!("   - batches: {}", batch_ids.len());

    let want_u64 = refno.refno();
    let mut found_info = 0usize;
    let mut found_geos = 0usize;

    for batch_id in batch_ids {
        let Some(batch) = cache.get(dbnum, &batch_id).await else {
            continue;
        };

        // inst_info_map: key 可能是 Refno 或 SesRef([refno,sesno])，这里按 RefU64 归一化。
        let info_hit = batch
            .inst_info_map
            .iter()
            .any(|(k, _)| k.refno() == want_u64);
        if info_hit {
            found_info += 1;
        }

        let geos_hit = batch
            .inst_geos_map
            .values()
            .any(|g| g.refno.refno() == want_u64 && !g.insts.is_empty());
        if geos_hit {
            found_geos += 1;
        }

        if info_hit || geos_hit {
            println!(
                "   - hit batch_id={} info_hit={} geos_hit={} (info_cnt={}, geos_cnt={})",
                batch_id,
                info_hit,
                geos_hit,
                batch.inst_info_map.len(),
                batch.inst_geos_map.len()
            );
        }

        // 若已找到 geos，通常无需继续扫（加速调试）。
        if found_geos > 0 {
            break;
        }
    }

    println!(
        "✅ 总结: inst_info_hit_batches={}, inst_geos_hit_batches={}",
        found_info, found_geos
    );
    if found_info > 0 && found_geos == 0 {
        println!(
            "⚠️ 该 refno 在 cache 里只有 inst_info（AABB/transform），没有 inst_geos（geo_hash/insts）。"
        );
        println!("   这会导致 cache-only 的 query_insts / room_calc 无法加载 TriMesh。");
    }

    Ok(())
}
