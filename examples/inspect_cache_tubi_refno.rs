//! 检查 foyer instance_cache 中某个 BRAN/HANG（ROOT_REFNO）对应的 tubing（inst_tubi_map）。
//!
//! 目的：排查“tubi 未按 ARRIVE->LEAVE（世界坐标端点）绘制”的根因：
//! - cache 中是否存在 tubi 记录？
//! - 是否缺失 tubi_info_id / ptset_map（导致无法按 arrive/leave 还原端点）？
//! - tubi 的 world_transform 与（unit cylinder z=[0..1]）端点是否一致？
//! - arrive_axis_pt / leave_axis_pt 是否与 transform 端点一致（从而判断是否坐标系/约定混用）？
//!
//! 用法（PowerShell）：
//!   $env:ROOT_REFNO="24381/103385"
//!   $env:CACHE_DIR="output/instance_cache"
//!   cargo run --example inspect_cache_tubi_refno
//!
//! 可选：
//!   $env:MAX=50   # 限制输出条数
//!   $env:BATCH="xxx"  # 指定 batch_id（不指定则用最新 batch）
//!
use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use glam::Vec3;
use std::env;
use std::path::PathBuf;

fn dist(a: Vec3, b: Vec3) -> f32 {
    (a - b).length()
}

fn parse_i32_env(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
}

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

    let max_out = parse_i32_env("MAX").unwrap_or(200);
    let prefer_batch = env::var("BATCH").ok().filter(|s| !s.trim().is_empty());

    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载（请先生成 output/scene_tree/db_meta_info.json）")?;
    let Some(dbnum) =
        aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(root)
    else {
        anyhow::bail!("无法从 db_meta 推导 dbnum: {}", root);
    };

    println!("ROOT_REFNO={} dbnum={}", root, dbnum);
    println!("cache_dir={}", cache_dir.display());

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;
    let mut batch_ids = cache.list_batches(dbnum);
    anyhow::ensure!(!batch_ids.is_empty(), "cache 中没有 batch: dbnum={}", dbnum);

    // 选 batch：默认取“最后一个”（list_batches 的顺序由 index 决定；导出侧习惯倒序命中最新覆盖旧）
    if let Some(b) = prefer_batch.as_deref() {
        anyhow::ensure!(
            batch_ids.iter().any(|x| x == b),
            "指定 BATCH={} 不在 cache.index 中：dbnum={}",
            b,
            dbnum
        );
        batch_ids = vec![b.to_string()];
    }

    let root_u64 = root.refno();
    let mut found = 0usize;
    let mut printed = 0usize;

    // 倒序：尽量先看“最新”batch
    for bid in batch_ids.iter().rev() {
        let Some(batch) = cache.get(dbnum, bid).await else {
            continue;
        };

        // 只看 owner_refno==root 的 tubi（与 cata_model.rs 写入约定一致）
        for (k, info) in batch.inst_tubi_map.iter() {
            if info.owner_refno.refno() != root_u64 {
                continue;
            }
            found += 1;
            if printed >= max_out {
                continue;
            }

            let wt = info.get_ele_world_transform(); // bevy::Transform
            let m = wt.to_matrix(); // Mat4

            // unit cylinder 约定：z=[0..1]
            let p0 = m.transform_point3(Vec3::new(0.0, 0.0, 0.0));
            let p1 = m.transform_point3(Vec3::new(0.0, 0.0, 1.0));

            println!("\n== tubi #{} batch_id={} ==", printed + 1, bid);
            println!("key_refno={} (key_refu64={})", k, k.refno());
            println!("owner_refno={} (expect ROOT_REFNO)", info.owner_refno);
            println!("type(owner_type)={}", info.owner_type);
            println!(
                "world_transform: t=({:.3},{:.3},{:.3}) s=({:.3},{:.3},{:.3})",
                wt.translation.x,
                wt.translation.y,
                wt.translation.z,
                wt.scale.x,
                wt.scale.y,
                wt.scale.z
            );
            println!(
                "unit_endpoints_by_transform: p0=({:.3},{:.3},{:.3}) p1=({:.3},{:.3},{:.3}) len={:.3}",
                p0.x,
                p0.y,
                p0.z,
                p1.x,
                p1.y,
                p1.z,
                dist(p0, p1)
            );

            println!(
                "tubi_info_id={:?} ptset_map_len={}",
                info.tubi_info_id,
                info.ptset_map.len()
            );

            if let (Some(sp), Some(ep)) = (info.tubi_start_pt, info.tubi_end_pt) {
                println!(
                    "tubi_start_end: start=({:.3},{:.3},{:.3}) end=({:.3},{:.3},{:.3}) len={:.3}",
                    sp.x,
                    sp.y,
                    sp.z,
                    ep.x,
                    ep.y,
                    ep.z,
                    dist(sp, ep)
                );
                println!(
                    "delta_vs_transform: |start-p0|={:.3} |end-p1|={:.3} |start-p1|={:.3} |end-p0|={:.3}",
                    dist(sp, p0),
                    dist(ep, p1),
                    dist(sp, p1),
                    dist(ep, p0)
                );
            } else {
                println!("tubi_start_end: None");
            }

            if let Some(a) = info.arrive_axis_pt {
                println!(
                    "arrive_axis_pt(raw) = [{:.3},{:.3},{:.3}]  dist_to_p0={:.3} dist_to_p1={:.3}",
                    a[0],
                    a[1],
                    a[2],
                    dist(Vec3::from(a), p0),
                    dist(Vec3::from(a), p1)
                );
            } else {
                println!("arrive_axis_pt: None");
            }
            if let Some(l) = info.leave_axis_pt {
                println!(
                    "leave_axis_pt(raw)  = [{:.3},{:.3},{:.3}]  dist_to_p0={:.3} dist_to_p1={:.3}",
                    l[0],
                    l[1],
                    l[2],
                    dist(Vec3::from(l), p0),
                    dist(Vec3::from(l), p1)
                );
            } else {
                println!("leave_axis_pt: None");
            }

            printed += 1;
        }
    }

    println!(
        "\nsummary: tubi_found(owner==ROOT)={} printed={}",
        found, printed
    );
    if found == 0 {
        println!("⚠️ 未在 cache.inst_tubi_map 中找到 owner_refno==ROOT 的 tubi 记录。");
        println!(
            "   这通常意味着：tubi 写入 key/owner 与导出筛选条件不一致，或 BRAN/HANG tubing 未生成/未写入 cache。"
        );
    }

    Ok(())
}
