//! 检查 foyer instance_cache 中某个 refno 的 inst_geos 细节（geo_hash / geo_param / geo_transform）。
//!
//! 用途：
//! - 排查“导出尺寸不对/缩放被重复应用”的问题（例如 RTOR 160mm 被放大到 25600mm）。
//!
//! 运行示例（PowerShell）：
//!   $env:REFNO="24381/129922"
//!   $env:CACHE_DIR="output/instance_cache"
//!   cargo run --example inspect_cache_geom_refno

use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use std::collections::HashSet;
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
    let mut best: Option<(
        String,
        i64,
        aios_database::fast_model::instance_cache::CachedInstanceBatch,
    )> = None;

    // 汇总所有 batch 中的关系（便于确认“关系缺失” vs “仅最新 batch 丢失/覆盖”）
    let mut all_neg_carriers: HashSet<RefnoEnum> = HashSet::new();
    let mut all_ngmr_pairs: HashSet<(RefnoEnum, RefnoEnum)> = HashSet::new();

    for batch_id in batch_ids {
        let Some(batch) = cache.get(dbnum, &batch_id).await else {
            continue;
        };

        if let Some(v) = batch.neg_relate_map.get(&refno) {
            for &c in v {
                all_neg_carriers.insert(c);
            }
        }
        if let Some(v) = batch.ngmr_neg_relate_map.get(&refno) {
            for &p in v {
                all_ngmr_pairs.insert(p);
            }
        }

        // 命中条件：
        // - inst_geos 命中（可直接查看几何）
        // - 或仅 inst_info 命中（用于排查 owner/世界变换，但该 refno 本身可能无几何）
        let hit = batch.inst_info_map.keys().any(|k| k.refno() == want_u64)
            || batch
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
        println!("⚠️ 未在 cache 中找到该 refno 的 inst_info/inst_geos（可能未生成或已被清理）");
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

    println!(
        "\n== latest hit batch_id={} created_at={} ==",
        batch_id, batch.created_at
    );
    println!(
        "== all batches relation summary ==\n  - neg carriers: {}\n  - ngmr pairs: {}",
        all_neg_carriers.len(),
        all_ngmr_pairs.len()
    );
    if !all_ngmr_pairs.is_empty() {
        let mut pairs: Vec<(RefnoEnum, RefnoEnum)> = all_ngmr_pairs.iter().copied().collect();
        pairs.sort_by_key(|(c, g)| (c.refno().0, g.refno().0));
        println!("  - ngmr pairs (carrier, geom_refno) sample:");
        for (i, (c, g)) in pairs.iter().take(20).enumerate() {
            println!("    [{}] carrier={} geom_refno={}", i, c, g);
        }
    }
    // 关系映射（用于排查“负实体已生成但未被应用到目标”的情况）
    match batch.neg_relate_map.get(&refno) {
        Some(v) => {
            println!("neg_relate_map[target={}] carriers={}", refno, v.len());
            for (i, c) in v.iter().enumerate() {
                println!("  - neg_carrier[{}]={}", i, c);
            }
        }
        None => println!("neg_relate_map[target={}] carriers=<none>", refno),
    }
    match batch.ngmr_neg_relate_map.get(&refno) {
        Some(v) => {
            println!("ngmr_neg_relate_map[target={}] pairs={}", refno, v.len());
            for (i, (carrier, geom_refno)) in v.iter().enumerate() {
                println!(
                    "  - ngmr_pair[{}] carrier={} geom_refno={}",
                    i, carrier, geom_refno
                );
            }
        }
        None => println!("ngmr_neg_relate_map[target={}] pairs=<none>", refno),
    }

    // 反向查询：当前 refno 作为“负载体”会切哪些目标（便于定位 target 计算是否跑偏到 owner/attached）
    let mut as_neg_carrier: Vec<RefnoEnum> = Vec::new();
    for (target, carriers) in &batch.neg_relate_map {
        if carriers.iter().any(|c| *c == refno) {
            as_neg_carrier.push(*target);
        }
    }
    if !as_neg_carrier.is_empty() {
        println!(
            "neg_relate_map[carrier={}] targets={}",
            refno,
            as_neg_carrier.len()
        );
        for t in as_neg_carrier {
            println!("  - target={}", t);
        }
    }

    let mut as_ngmr_carrier: Vec<(RefnoEnum, RefnoEnum)> = Vec::new();
    for (target, pairs) in &batch.ngmr_neg_relate_map {
        for (carrier, geom_refno) in pairs {
            if *carrier == refno {
                as_ngmr_carrier.push((*target, *geom_refno));
            }
        }
    }
    if !as_ngmr_carrier.is_empty() {
        println!(
            "ngmr_neg_relate_map[carrier={}] targets={}",
            refno,
            as_ngmr_carrier.len()
        );
        for (t, g) in as_ngmr_carrier {
            println!("  - target={} geom_refno={}", t, g);
        }
    }

    if let Some(info) = info {
        println!(
            "inst_info: refno={} sesno={} visible={} owner={:?}/{:?}",
            info.refno, info.sesno, info.visible, info.owner_refno, info.owner_type
        );
        println!("inst_info.world_transform(raw): {:?}", info.world_transform);
        println!(
            "inst_info.world_transform(effective): {:?}",
            info.get_ele_world_transform()
        );
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
                "  - inst[{}] geom_refno={} geo_hash={} unit_flag={} geo_type={:?}",
                i,
                inst.refno,
                inst.geo_hash,
                inst.geo_param.is_reuse_unit(),
                inst.geo_type
            );
            println!("    geo_param: {:?}", inst.geo_param);
            println!("    geo_transform: {:?}", inst.geo_transform);
            println!(
                "    scale: [{:.6}, {:.6}, {:.6}]",
                inst.geo_transform.scale.x, inst.geo_transform.scale.y, inst.geo_transform.scale.z
            );
            println!(
                "    translation: [{:.3}, {:.3}, {:.3}]",
                inst.geo_transform.translation.x,
                inst.geo_transform.translation.y,
                inst.geo_transform.translation.z
            );
        }
    }

    Ok(())
}
