//! Dump one refno's cached inst_geos + neg/ngmr relations (cache-only path).
//!
//! Usage (cmd.exe):
//!   set REFNO=17496/142306
//!   set CACHE_DIR=output/instance_cache
//!   cargo run --example dump_cache_refno_bool_inputs --features sqlite-index
//!
//! Notes:
//! - Requires output/scene_tree/db_meta_info.json to map refno -> dbnum.
//! - Prints enough info to verify boolean worker inputs:
//!   - inst_geos.insts geo_type / geo_hash / reuse_unit_flag / geo_transform scale+translation
//!   - neg_relate_map keys for this refno
//!   - ngmr_neg_relate_map keys for this refno

use anyhow::{Context, Result};
use aios_core::geometry::GeoBasicType;
use aios_core::RefnoEnum;
use std::collections::HashSet;
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

    let Some(dbnum) = aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(refno) else {
        anyhow::bail!("无法从 db_meta 推导 dbnum: {}", refno);
    };

    println!("🔎 dump cache bool inputs: refno={}, dbnum={}", refno, dbnum);
    println!("   - cache_dir: {}", cache_dir.display());

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;

    let batch_ids = cache.list_batches(dbnum);
    println!("   - batches: {}", batch_ids.len());

    let want_u64 = refno.refno();
    for batch_id in batch_ids {
        let Some(batch) = cache.get(dbnum, &batch_id).await else {
            continue;
        };

        // inst_key is derived from EleGeosInfo, so we match by refno first.
        let info = batch
            .inst_info_map
            .iter()
            .find(|(k, _)| k.refno() == want_u64)
            .map(|(_, v)| v.clone());
        if info.is_none() {
            continue;
        }
        let info = info.unwrap();

        let inst_key = info.get_inst_key();
        let geos = batch.inst_geos_map.get(&inst_key).cloned();
        if geos.is_none() {
            continue;
        }
        let geos = geos.unwrap();

        println!("✅ hit batch_id={}", batch_id);
        println!("   - inst_key={}", inst_key);
        println!("   - geos.type_name={}", geos.type_name);
        println!("   - owner_type={} owner_refno={}", info.owner_type, info.owner_refno);
        println!("   - has_cata_neg={} is_solid={}", info.has_cata_neg, info.is_solid);
        println!(
            "   - world_trans: t=({:.3},{:.3},{:.3}) s=({:.3},{:.3},{:.3})",
            info.world_transform.translation.x,
            info.world_transform.translation.y,
            info.world_transform.translation.z,
            info.world_transform.scale.x,
            info.world_transform.scale.y,
            info.world_transform.scale.z
        );

        println!("   - inst_geos.insts: {}", geos.insts.len());
        for (i, inst) in geos.insts.iter().enumerate() {
            // Keep output compact but sufficient for scale/placement debugging.
            let unit_flag = inst.geo_param.is_reuse_unit();
            println!(
                "     [{:02}] geo_type={:?} geom_refno={} geo_hash={} unit_flag={} vis={} cata_neg_cnt={}",
                i,
                inst.geo_type,
                inst.refno,
                inst.geo_hash,
                unit_flag,
                inst.visible,
                inst.cata_neg_refnos.len()
            );
            println!(
                "          local: t=({:.3},{:.3},{:.3}) s=({:.3},{:.3},{:.3})",
                inst.geo_transform.translation.x,
                inst.geo_transform.translation.y,
                inst.geo_transform.translation.z,
                inst.geo_transform.scale.x,
                inst.geo_transform.scale.y,
                inst.geo_transform.scale.z
            );
            // rotation 也会影响“底对齐”补偿方向（例如局部 z 轴翻转会导致 +0.5 变成向下移动）。
            println!(
                "          rot: ({:.6},{:.6},{:.6},{:.6})",
                inst.geo_transform.rotation.x,
                inst.geo_transform.rotation.y,
                inst.geo_transform.rotation.z,
                inst.geo_transform.rotation.w
            );
        }

        // Show what cache has for bool results.
        if let Some(b) = batch.inst_relate_bool_map.get(&refno) {
            println!(
                "   - inst_relate_bool: status={} mesh_id={} created_at={}",
                b.status, b.mesh_id, b.created_at
            );
        } else {
            println!("   - inst_relate_bool: <none>");
        }

        // Relations:
        let neg_carriers = batch
            .neg_relate_map
            .get(&refno)
            .cloned()
            .unwrap_or_default();
        let uniq_neg: HashSet<_> = neg_carriers.into_iter().collect();
        println!("   - neg_relate_map[refno] carriers: {}", uniq_neg.len());
        for c in uniq_neg {
            println!("     - carrier={}", c);
        }

        let ngmr_pairs = batch
            .ngmr_neg_relate_map
            .get(&refno)
            .cloned()
            .unwrap_or_default();
        let uniq_ngmr: HashSet<_> = ngmr_pairs.into_iter().collect();
        println!("   - ngmr_neg_relate_map[refno] pairs: {}", uniq_ngmr.len());
        for (carrier, geom) in uniq_ngmr {
            println!("     - (carrier={}, geom_refno={})", carrier, geom);
        }

        // Quick summary: count neg-ish shapes in this inst itself.
        let mut n_cata_neg = 0usize;
        let mut n_cata_cross = 0usize;
        let mut n_pos_like = 0usize;
        for inst in &geos.insts {
            match inst.geo_type {
                GeoBasicType::CataNeg => n_cata_neg += 1,
                GeoBasicType::CataCrossNeg => n_cata_cross += 1,
                GeoBasicType::Pos | GeoBasicType::Compound | GeoBasicType::CatePos | GeoBasicType::DesiPos => {
                    n_pos_like += 1
                }
                _ => {}
            }
        }
        println!(
            "   - summary: pos_like={} cata_neg={} cata_cross_neg={}",
            n_pos_like, n_cata_neg, n_cata_cross
        );

        return Ok(());
    }

    anyhow::bail!("未在 cache 中找到 refno={} 的 inst_info+inst_geos（请先 regen-model 生成缓存）", refno);
}

