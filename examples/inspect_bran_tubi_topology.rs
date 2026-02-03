//! 基于 foyer instance_cache 的“拓扑数据”（inst_tubi_map + ptset_map + world_transform）
//! 还原 BRAN/HANG 的直管段端点，辅助判断“导出管道是否齐全/是否与基准截图一致”。
//!
//! 用法（PowerShell）：
//!   $env:ROOT_REFNO="24381/103385"
//!   $env:CACHE_DIR="output/instance_cache"
//!   cargo run --example inspect_bran_tubi_topology

use anyhow::{Context, Result};
use aios_core::RefnoEnum;
use glam::Vec3;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
struct Segment {
    refno: RefnoEnum,
    arrive: i32,
    leave: i32,
    a_world: Vec3,
    l_world: Vec3,
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    (a - b).length()
}

fn unit_dir(a: Vec3, b: Vec3) -> Vec3 {
    let d = b - a;
    let len = d.length();
    if len <= 1e-6 {
        Vec3::ZERO
    } else {
        d / len
    }
}

fn parse_arrive_leave_from_tubi_info_id(id: &str) -> Option<(i32, i32)> {
    // id 形如: "{cata_hash}_{arrive}_{leave}"
    let mut it = id.rsplitn(3, '_');
    let leave = it.next()?.parse::<i32>().ok()?;
    let arrive = it.next()?.parse::<i32>().ok()?;
    Some((arrive, leave))
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

    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载（请先生成 output/scene_tree/db_meta_info.json）")?;
    let Some(dbnum) =
        aios_database::data_interface::db_meta_manager::db_meta().get_dbnum_by_refno(root)
    else {
        anyhow::bail!("无法从 db_meta 推导 dbnum: {}", root);
    };

    // 以“导出默认语义”收集所有节点（包括自己 + 子孙），再从 inst_tubi_map 里取出对应的 tubing 拓扑。
    let all_refnos =
        aios_database::fast_model::export_model::model_exporter::collect_export_refnos(
            &[root],
            true,
            None,
            false,
        )
        .await
        .context("collect_export_refnos 失败（可能缺少 tree 索引）")?;

    println!("ROOT_REFNO={} dbnum={}", root, dbnum);
    println!("descendants(include self)={}", all_refnos.len());

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;
    let batch_ids = cache.list_batches(dbnum);
    anyhow::ensure!(!batch_ids.is_empty(), "cache 中没有 batch: dbnum={}", dbnum);

    // 对每个 refno，按“最新覆盖旧”的方式命中 inst_tubi_map。
    let mut segs: Vec<Segment> = Vec::new();
    for &r in &all_refnos {
        let mut hit = None;
        for bid in batch_ids.iter().rev() {
            let Some(batch) = cache.get(dbnum, bid).await else {
                continue;
            };
            // 注意：inst_tubi_map 的 key 可能是 SesRef([refno,sesno])，直接 get(&r) 会 miss。
            // 这里按 RefU64 归一化匹配，语义与导出侧一致（“最新覆盖旧”）。
            let want = r.refno();
            if let Some((_, info)) = batch
                .inst_tubi_map
                .iter()
                .find(|(k, _)| k.refno() == want)
            {
                hit = Some((bid.clone(), info.clone()));
                break;
            };
        }
        let Some((bid, info)) = hit else {
            continue;
        };

        let Some(tubi_id) = info.tubi_info_id.clone() else {
            // 没有 tubi_info_id 的一般不是“可还原端点”的 tubing 节点
            continue;
        };
        let Some((arrive, leave)) = parse_arrive_leave_from_tubi_info_id(&tubi_id) else {
            continue;
        };

        let Some(a) = info.ptset_map.values().find(|p| p.number == arrive) else {
            continue;
        };
        let Some(l) = info.ptset_map.values().find(|p| p.number == leave) else {
            continue;
        };

        // world_transform 作用到 ptset（局部坐标）
        let m = info.get_ele_world_transform().to_matrix();
        // a.pt/l.pt 的类型可能是 aios_core 的 RsVec3（内部可解引用为 glam::Vec3）。
        let a_world = m.transform_point3(*a.pt);
        let l_world = m.transform_point3(*l.pt);

        println!(
            "tubi_hit refno={} batch_id={} tubi_info_id={} arrive={} leave={} len={:.3}",
            r,
            bid,
            tubi_id,
            arrive,
            leave,
            dist(a_world, l_world)
        );

        segs.push(Segment {
            refno: r,
            arrive,
            leave,
            a_world,
            l_world,
        });
    }

    println!("\nsegments={}", segs.len());
    if segs.is_empty() {
        println!("⚠️ 未命中任何可解析端点的 tubi 段（inst_tubi_map/tubi_info_id 缺失）。");
        return Ok(());
    }

    // 简单检查：端点聚类（4 段时应该形成 3 个“内部结点” + 2 个“端点”）。
    // 由于只有少量段，直接 O(n^2) 找近邻即可。
    const EPS: f32 = 1e-2;
    let mut endpoints: Vec<(usize, bool, Vec3)> = Vec::new(); // (seg_idx, is_arrive, pt)
    for (i, s) in segs.iter().enumerate() {
        endpoints.push((i, true, s.a_world));
        endpoints.push((i, false, s.l_world));
    }

    // 聚类：将距离 < EPS 的端点视为同一点
    let mut clusters: Vec<Vec<(usize, bool)>> = Vec::new();
    let mut reps: Vec<Vec3> = Vec::new();
    'outer: for (si, is_a, p) in endpoints {
        for (ci, rep) in reps.iter().enumerate() {
            if dist(*rep, p) < EPS {
                clusters[ci].push((si, is_a));
                continue 'outer;
            }
        }
        reps.push(p);
        clusters.push(vec![(si, is_a)]);
    }

    println!("unique_junction_points={} (eps={})", clusters.len(), EPS);
    for (i, c) in clusters.iter().enumerate() {
        println!("  - junction[{}] degree={}", i, c.len());
    }

    // 角度检查：对 degree==2 的结点，计算两段方向夹角（期望 ~90° 或 ~180°）。
    for (ci, c) in clusters.iter().enumerate() {
        if c.len() != 2 {
            continue;
        }
        let (s0, a0) = c[0];
        let (s1, a1) = c[1];
        let seg0 = &segs[s0];
        let seg1 = &segs[s1];
        let p0 = if a0 { seg0.a_world } else { seg0.l_world };
        let p1 = if a1 { seg1.a_world } else { seg1.l_world };

        let d0 = if a0 {
            unit_dir(seg0.a_world, seg0.l_world)
        } else {
            unit_dir(seg0.l_world, seg0.a_world)
        };
        let d1 = if a1 {
            unit_dir(seg1.a_world, seg1.l_world)
        } else {
            unit_dir(seg1.l_world, seg1.a_world)
        };

        let dot = (d0.dot(d1)).clamp(-1.0, 1.0);
        let ang = dot.acos() * 180.0 / std::f32::consts::PI;
        println!(
            "  - junction[{}] connect: ({},{}) <-> ({},{}) at ({:.3},{:.3},{:.3}) angle={:.1}deg",
            ci,
            seg0.refno,
            if a0 { "arrive" } else { "leave" },
            seg1.refno,
            if a1 { "arrive" } else { "leave" },
            p0.x,
            p0.y,
            p0.z,
            ang
        );
    }

    Ok(())
}
