//! 检查 BRAN 的 tubi 连接关系
//!
//! 用法：
//!   ROOT_REFNO="24381/103385" cargo run --example check_tubi_connectivity

use anyhow::{Context, Result};
use aios_core::RefnoEnum;
use glam::Vec3;
use std::env;
use std::path::PathBuf;

const TOL: f32 = 1.0; // 1mm 容差

fn dist(a: Vec3, b: Vec3) -> f32 {
    (a - b).length()
}

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

    let Some(dbnum) = aios_database::data_interface::db_meta_manager::db_meta()
        .get_dbnum_by_refno(root) else {
        anyhow::bail!("无法获取 dbnum");
    };

    println!("🔎 BRAN={} dbnum={}", root, dbnum);

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await?;

    // 收集所有 tubi 的端点
    struct TubiEndpoints {
        refno: RefnoEnum,
        p0: Vec3,  // start
        p1: Vec3,  // end
    }

    let mut tubis: Vec<TubiEndpoints> = Vec::new();
    let batch_ids = cache.list_batches(dbnum);
    let root_u64 = root.refno().0;

    for batch_id in batch_ids.iter().rev() {
        let Some(batch) = cache.get(dbnum, batch_id).await else { continue };

        for (k, info) in batch.inst_tubi_map.iter() {
            if info.owner_refno.refno().0 != root_u64 { continue }

            let wt = info.get_ele_world_transform();
            let m = wt.to_matrix();
            let p0 = m.transform_point3(Vec3::new(0.0, 0.0, 0.0));
            let p1 = m.transform_point3(Vec3::new(0.0, 0.0, 1.0));

            tubis.push(TubiEndpoints { refno: *k, p0, p1 });
        }
        if !tubis.is_empty() { break }
    }

    println!("\n📊 共 {} 条 tubi:\n", tubis.len());

    for (i, t) in tubis.iter().enumerate() {
        println!("tubi #{} ({})", i + 1, t.refno);
        println!("  p0=({:.1},{:.1},{:.1})", t.p0.x, t.p0.y, t.p0.z);
        println!("  p1=({:.1},{:.1},{:.1})", t.p1.x, t.p1.y, t.p1.z);
    }

    // 检查连接关系
    println!("\n🔗 连接关系分析:\n");

    for (i, t1) in tubis.iter().enumerate() {
        for (j, t2) in tubis.iter().enumerate() {
            if i >= j { continue }

            // 检查 t1.p1 是否连接 t2.p0 或 t2.p1
            if dist(t1.p1, t2.p0) < TOL {
                println!("✅ tubi#{}.p1 -> tubi#{}.p0 (dist={:.3})", i+1, j+1, dist(t1.p1, t2.p0));
            }
            if dist(t1.p1, t2.p1) < TOL {
                println!("✅ tubi#{}.p1 -> tubi#{}.p1 (dist={:.3})", i+1, j+1, dist(t1.p1, t2.p1));
            }
            if dist(t1.p0, t2.p0) < TOL {
                println!("✅ tubi#{}.p0 -> tubi#{}.p0 (dist={:.3})", i+1, j+1, dist(t1.p0, t2.p0));
            }
            if dist(t1.p0, t2.p1) < TOL {
                println!("✅ tubi#{}.p0 -> tubi#{}.p1 (dist={:.3})", i+1, j+1, dist(t1.p0, t2.p1));
            }
        }
    }

    Ok(())
}
