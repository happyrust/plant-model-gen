//! 按 geo_hash 在 foyer instance_cache 中反查：有哪些 inst 使用了该 geo_hash，以及对应 geo_param/unit_flag/geo_transform。

use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let geo_hash: u64 = env::var("GEO_HASH")
        .ok()
        .or_else(|| std::env::args().nth(1))
        .unwrap_or_else(|| "9153972095265005083".to_string())
        .parse()
        .context("GEO_HASH 需为 u64")?;

    let cache_dir = env::var("CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));

    let dbnum: Option<u32> = env::var("DBNUM").ok().and_then(|s| s.parse().ok());

    println!("🔎 inspect_cache_geo_hash");
    println!("  - geo_hash: {}", geo_hash);
    println!("  - cache_dir: {}", cache_dir.display());
    if let Some(dbnum) = dbnum {
        println!("  - dbnum filter: {}", dbnum);
    }

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;

    let dbnums = if let Some(d) = dbnum {
        vec![d]
    } else {
        cache.list_dbnums()
    };

    let mut hits = 0usize;
    for db in dbnums {
        for batch_id in cache.list_batches(db) {
            let Some(batch) = cache.get(db, &batch_id).await else {
                continue;
            };
            for geos in batch.inst_geos_map.values() {
                for inst in &geos.insts {
                    if inst.geo_hash != geo_hash {
                        continue;
                    }
                    hits += 1;
                    println!("\n== hit #{hits} ==");
                    println!(
                        "  dbnum={} batch_id={} created_at={}",
                        db, batch_id, batch.created_at
                    );
                    println!("  refno={} type_name={}", geos.refno, geos.type_name);
                    println!(
                        "  unit_flag={} geo_type={:?}",
                        inst.geo_param.is_reuse_unit(),
                        inst.geo_type
                    );
                    println!("  geo_param: {:?}", inst.geo_param);
                    println!(
                        "  scale: [{:.6}, {:.6}, {:.6}]",
                        inst.geo_transform.scale.x,
                        inst.geo_transform.scale.y,
                        inst.geo_transform.scale.z
                    );
                    println!(
                        "  translation: [{:.3}, {:.3}, {:.3}]",
                        inst.geo_transform.translation.x,
                        inst.geo_transform.translation.y,
                        inst.geo_transform.translation.z
                    );
                }
            }
        }
    }

    if hits == 0 {
        println!("⚠️ 未在 cache 中找到 geo_hash={}", geo_hash);
    } else {
        println!("\n✅ total hits: {}", hits);
    }

    Ok(())
}
