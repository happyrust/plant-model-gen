//! 检查 BRAN 下所有子元件的 ptset_map 数据
//!
//! 用法：
//!   ROOT_REFNO="24381/103385" CACHE_DIR="output/instance_cache" cargo run --example inspect_bran_children_ptset

use aios_core::RefnoEnum;
use aios_database::fast_model::gen_model::tree_index_manager::TreeIndexManager;
use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let root_str = env::var("ROOT_REFNO").unwrap_or_else(|_| "24381/103385".to_string());
    let root = RefnoEnum::from(root_str.as_str());
    anyhow::ensure!(root.is_valid(), "无效 ROOT_REFNO: {}", root_str);

    let cache_dir = env::var("CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));

    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载")?;

    let Some(dbnum) =
        aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(root)
    else {
        anyhow::bail!("无法从 db_meta 推导 dbnum: {}", root);
    };

    println!("🔎 BRAN={} dbnum={}", root, dbnum);

    // 获取子元件列表
    let children = TreeIndexManager::collect_children_elements_from_tree(root)
        .await
        .unwrap_or_default();
    println!("子元件数量: {}", children.len());

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;

    let batch_ids = cache.list_batches(dbnum);

    for child in &children {
        let child_refno = child.refno;
        let child_type = &child.noun;
        let child_u64 = child_refno.refno();

        println!("\n== {} ({}) ==", child_refno, child_type);

        let mut found = false;
        // 在 cache 中查找
        for batch_id in batch_ids.iter().rev() {
            let Some(batch) = cache.get(dbnum, batch_id).await else {
                continue;
            };

            for (k, info) in batch.inst_info_map.iter() {
                if k.refno() != child_u64 {
                    continue;
                }

                found = true;
                let wt = info.get_ele_world_transform();
                println!("  cata_hash={:?}", info.cata_hash);
                println!(
                    "  world_t=({:.3},{:.3},{:.3})",
                    wt.translation.x, wt.translation.y, wt.translation.z
                );

                println!("  ptset_map.len()={}", info.ptset_map.len());
                for (num, axis) in &info.ptset_map {
                    let transformed = axis.transformed(&wt);
                    println!(
                        "    [{}] local=({:.3},{:.3},{:.3}) -> world=({:.3},{:.3},{:.3})",
                        num,
                        axis.pt.0.x,
                        axis.pt.0.y,
                        axis.pt.0.z,
                        transformed.pt.0.x,
                        transformed.pt.0.y,
                        transformed.pt.0.z
                    );
                }
                break;
            }
            if found {
                break;
            }
        }
        if !found {
            println!("  ⚠️ 未在 cache 中找到 inst_info");
        }
    }

    Ok(())
}
