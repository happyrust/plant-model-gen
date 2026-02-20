//! 校验 instances_{dbnum}.json 中 tubings 的 order 是否符合“连通顺序沿管线递增”的期望。
//!
//! 思路：从导出的 JSON 读取 tubi 列表（按 order 排序），再从 foyer cache 的 inst_tubi_map
//! 取出对应 EleGeosInfo，比较相邻段的端点距离，给出统计。
//!
//! 用法（PowerShell）：
//!   $env:DBNUM="1112"
//!   $env:CACHE_DIR="output/AvevaMarineSample/instance_cache"
//!   $env:INSTANCES_JSON="output/AvevaMarineSample/instances/instances_1112.json"
//!   cargo run --example verify_instances_json_tubi_order
//!
//! 可选：
//!   $env:OWNER_REFNO="17496/171606"   # 只检查指定 owner_refno 的 group（默认取第一个 group）
//!   $env:TOL_MM="1.0"                # 容差（mm）

use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use glam::Vec3;
use serde::Deserialize;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct InstancesJson {
    groups: Vec<GroupJson>,
}

#[derive(Debug, Deserialize)]
struct GroupJson {
    owner_refno: String,
    tubings: Vec<TubiJson>,
}

#[derive(Debug, Deserialize, Clone)]
struct TubiJson {
    refno: String,
    order: u32,
}

#[derive(Debug, Clone)]
struct TubiPts {
    refno: RefnoEnum,
    start: Vec3,
    end: Vec3,
    arrive_axis: Vec3,
    leave_axis: Vec3,
}

fn parse_f32(s: &str, default: f32) -> f32 {
    s.parse::<f32>().unwrap_or(default)
}

fn vec3_from_arr(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

#[tokio::main]
async fn main() -> Result<()> {
    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .ok();

    let dbnum: u32 = env::var("DBNUM")
        .ok()
        .and_then(|x| x.parse::<u32>().ok())
        .unwrap_or(1112);
    let tol = parse_f32(
        &env::var("TOL_MM").unwrap_or_else(|_| "1.0".to_string()),
        1.0,
    );

    let cache_dir = PathBuf::from(
        env::var("CACHE_DIR").unwrap_or_else(|_| "output/AvevaMarineSample/instance_cache".into()),
    );
    let instances_json = PathBuf::from(env::var("INSTANCES_JSON").unwrap_or_else(|_| {
        format!(
            "output/AvevaMarineSample/instances/instances_{}.json",
            dbnum
        )
    }));

    let owner_filter = env::var("OWNER_REFNO")
        .ok()
        .and_then(|s| s.parse::<RefnoEnum>().ok())
        .map(|r| r.to_string());

    let data: InstancesJson = serde_json::from_str(
        &std::fs::read_to_string(&instances_json)
            .with_context(|| format!("读取 instances json 失败: {}", instances_json.display()))?,
    )
    .context("解析 instances json 失败")?;
    anyhow::ensure!(!data.groups.is_empty(), "instances json 中没有 groups");

    let group = if let Some(of) = owner_filter.as_ref() {
        data.groups
            .iter()
            .find(|g| &g.owner_refno == of)
            .with_context(|| format!("未找到 owner_refno={} 的 group", of))?
    } else {
        &data.groups[0]
    };

    let mut tubis = group.tubings.clone();
    tubis.sort_by_key(|t| t.order);

    println!(
        "dbnum={} owner_refno={} tubings={}",
        dbnum,
        group.owner_refno,
        tubis.len()
    );
    println!("cache_dir={}", cache_dir.display());
    println!("instances_json={}", instances_json.display());
    println!("tol_mm={}", tol);

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;
    let batch_ids = cache.list_batches(dbnum);
    anyhow::ensure!(!batch_ids.is_empty(), "cache 中没有 batch: dbnum={}", dbnum);
    println!("cache_batches={}", batch_ids.len());

    // 还原导出侧的“多 batch 合并”语义：从新到旧遍历 batch，首次命中就固定该 tubi 的 EleGeosInfo。
    let need: Vec<RefnoEnum> = tubis
        .iter()
        .filter_map(|t| t.refno.parse::<RefnoEnum>().ok())
        .collect();
    let mut resolved: std::collections::HashMap<RefnoEnum, aios_core::geometry::EleGeosInfo> =
        std::collections::HashMap::new();
    let mut hit_from: std::collections::HashMap<RefnoEnum, String> =
        std::collections::HashMap::new();

    for bid in batch_ids.iter().rev() {
        if resolved.len() == need.len() {
            break;
        }
        let Some(batch) = cache.get(dbnum, bid).await else {
            continue;
        };
        for r in &need {
            if resolved.contains_key(r) {
                continue;
            }
            if let Some(info) = batch.inst_tubi_map.get(r) {
                resolved.insert(*r, (*info).clone());
                hit_from.insert(*r, bid.clone());
            }
        }
    }

    let mut pts_list: Vec<TubiPts> = Vec::with_capacity(tubis.len());
    let mut missing = 0usize;

    for t in &tubis {
        let r: RefnoEnum = t
            .refno
            .parse()
            .map_err(|e| anyhow::anyhow!("解析 refno 失败: {} err={:?}", t.refno, e))?;
        let Some(info) = resolved.get(&r) else {
            missing += 1;
            continue;
        };

        // 基准：start/end（通常与 world_transform 的 unit cylinder 端点一致）
        let wt = info.get_ele_world_transform();
        let m = wt.to_matrix();
        let start = info
            .tubi_start_pt
            .unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 0.0)));
        let end = info
            .tubi_end_pt
            .unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 1.0)));

        let arrive_axis = info.arrive_axis_pt.map(vec3_from_arr).unwrap_or(end);
        let leave_axis = info.leave_axis_pt.map(vec3_from_arr).unwrap_or(start);

        pts_list.push(TubiPts {
            refno: r,
            start,
            end,
            arrive_axis,
            leave_axis,
        });
    }

    println!(
        "tubi_resolved={} missing_in_cache={}",
        pts_list.len(),
        missing
    );
    if let Some(t0) = pts_list.first() {
        if let Some(bid) = hit_from.get(&t0.refno) {
            println!("sample_hit_batch: {} -> {}", t0.refno, bid);
        }
    }
    if pts_list.len() <= 1 {
        return Ok(());
    }

    fn dist(a: Vec3, b: Vec3) -> f32 {
        (a - b).length()
    }

    let mut ok_axis_arrive_to_leave = 0usize;
    let mut ok_axis_leave_to_arrive = 0usize;
    let mut ok_start_end = 0usize;
    let mut ok_any = 0usize;

    let mut worst: Option<(usize, String, String, f32)> = None;

    for i in 0..(pts_list.len() - 1) {
        let a = &pts_list[i];
        let b = &pts_list[i + 1];

        let d_axis_a2l = dist(a.arrive_axis, b.leave_axis);
        let d_axis_l2a = dist(a.leave_axis, b.arrive_axis);
        let d_se = dist(a.end, b.start);

        if d_axis_a2l <= tol {
            ok_axis_arrive_to_leave += 1;
        }
        if d_axis_l2a <= tol {
            ok_axis_leave_to_arrive += 1;
        }
        if d_se <= tol {
            ok_start_end += 1;
        }

        let any_min = [
            dist(a.start, b.start),
            dist(a.start, b.end),
            dist(a.end, b.start),
            dist(a.end, b.end),
            dist(a.arrive_axis, b.arrive_axis),
            dist(a.arrive_axis, b.leave_axis),
            dist(a.leave_axis, b.arrive_axis),
            dist(a.leave_axis, b.leave_axis),
        ]
        .into_iter()
        .fold(f32::INFINITY, f32::min);

        if any_min <= tol {
            ok_any += 1;
        }

        worst = match worst {
            None => Some((i, a.refno.to_string(), b.refno.to_string(), any_min)),
            Some(cur) => {
                if any_min > cur.3 {
                    Some((i, a.refno.to_string(), b.refno.to_string(), any_min))
                } else {
                    Some(cur)
                }
            }
        };
    }

    let total = pts_list.len() - 1;
    println!("adjacent_total={}", total);
    println!(
        "ok_axis(arrive->leave)={}/{}  ok_axis(leave->arrive)={}/{}  ok_start(end->start)={}/{}  ok_any={}/{}",
        ok_axis_arrive_to_leave,
        total,
        ok_axis_leave_to_arrive,
        total,
        ok_start_end,
        total,
        ok_any,
        total
    );
    if let Some((i, a, b, d)) = worst {
        println!("worst_any: idx={} {} -> {}  dist={}", i, a, b, d);
    }

    // === 额外：用 axis 点重新按连通性排序，再看相邻是否满足 arrive->leave ===
    fn cell(p: Vec3, cell_size: f32) -> (i32, i32, i32) {
        let inv = 1.0 / cell_size.max(1e-6);
        (
            (p.x * inv).floor() as i32,
            (p.y * inv).floor() as i32,
            (p.z * inv).floor() as i32,
        )
    }

    fn build_bins(
        pts: &[TubiPts],
        tol: f32,
        get_p: fn(&TubiPts) -> Vec3,
    ) -> std::collections::HashMap<(i32, i32, i32), Vec<usize>> {
        let mut bins: std::collections::HashMap<(i32, i32, i32), Vec<usize>> =
            std::collections::HashMap::new();
        for (i, s) in pts.iter().enumerate() {
            bins.entry(cell(get_p(s), tol)).or_default().push(i);
        }
        bins
    }

    fn near_indices(
        bins: &std::collections::HashMap<(i32, i32, i32), Vec<usize>>,
        p: Vec3,
        tol: f32,
    ) -> Vec<usize> {
        let (cx, cy, cz) = cell(p, tol);
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = bins.get(&(cx + dx, cy + dy, cz + dz)) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        out
    }

    fn order_by_arrive_to_leave(pts: &[TubiPts], tol: f32) -> Vec<usize> {
        let leave_bins = build_bins(pts, tol, |s| s.leave_axis);
        let arrive_bins = build_bins(pts, tol, |s| s.arrive_axis);

        // 入度：是否存在其它段的 arrive 贴近我的 leave
        let mut has_prev = vec![false; pts.len()];
        for (i, s) in pts.iter().enumerate() {
            for j in near_indices(&arrive_bins, s.leave_axis, tol) {
                if i == j {
                    continue;
                }
                if (pts[j].arrive_axis - s.leave_axis).length() <= tol {
                    has_prev[i] = true;
                    break;
                }
            }
        }

        let mut visited = vec![false; pts.len()];
        let mut out = Vec::with_capacity(pts.len());
        let key = |idx: usize| pts[idx].refno.to_string();

        while out.len() < pts.len() {
            // 选起点：优先选“无前驱”的段；否则取最小 refno（处理环）
            let mut starts: Vec<usize> = (0..pts.len())
                .filter(|&i| !visited[i] && !has_prev[i])
                .collect();
            if starts.is_empty() {
                if let Some(i) = (0..pts.len())
                    .filter(|&i| !visited[i])
                    .min_by_key(|&i| key(i))
                {
                    starts.push(i);
                }
            }
            starts.sort_by_key(|&i| key(i));
            let mut cur = starts[0];

            loop {
                visited[cur] = true;
                out.push(cur);

                // next: 找 leave ~= cur.arrive 的段
                let mut cands: Vec<usize> = near_indices(&leave_bins, pts[cur].arrive_axis, tol)
                    .into_iter()
                    .filter(|&j| !visited[j])
                    .filter(|&j| (pts[j].leave_axis - pts[cur].arrive_axis).length() <= tol)
                    .collect();
                if cands.is_empty() {
                    break;
                }
                cands.sort_by_key(|&j| key(j));
                cur = cands[0];
            }
        }

        out
    }

    let ord = order_by_arrive_to_leave(&pts_list, tol);
    let mut ok = 0usize;
    for w in ord.windows(2) {
        let a = &pts_list[w[0]];
        let b = &pts_list[w[1]];
        if (a.arrive_axis - b.leave_axis).length() <= tol {
            ok += 1;
        }
    }
    println!(
        "reorder_axis(arrive->next.leave): ok_adjacent={}/{}",
        ok,
        ord.len().saturating_sub(1)
    );

    Ok(())
}
