//! 审计 cache 中“基本体”的缩放/单位化策略是否一致，帮助发现 SNOU 类似的“重复缩放”问题。
//!
//! 典型症状：
//! - inst_geo.unit_flag = false（表示 mesh 顶点已是“真实尺寸”）
//! - 但 inst.transform.scale != (1,1,1)（又在实例层做了一次缩放）
//! 这会导致布尔/导出时把同一个尺寸乘两次。
//!
//! 用法（cmd.exe）：
//!   set OWNER_REFNO=24381/131079
//!   set CACHE_DIR=output/instance_cache
//!   cargo run --example audit_basic_primitives
//!
//! 可选：
//! - 不设置 OWNER_REFNO：扫描该 refno 所在 dbnum 下的全部基本体（可能很大）
//! - 设置 MAX_ANOMALIES：限制输出条数（默认 50）

use aios_core::RefnoEnum;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

fn prim_kind(p: &PdmsGeoParam) -> Option<&'static str> {
    match p {
        PdmsGeoParam::PrimBox(_) => Some("BOX"),
        PdmsGeoParam::PrimSphere(_) => Some("SPHE"),
        PdmsGeoParam::PrimLCylinder(_) => Some("LCYL"),
        PdmsGeoParam::PrimSCylinder(_) => Some("SCYL"),
        PdmsGeoParam::PrimLSnout(_) => Some("SNOU"),
        PdmsGeoParam::PrimDish(_) => Some("DISH"),
        PdmsGeoParam::PrimCTorus(_) => Some("CTOR"),
        PdmsGeoParam::PrimRTorus(_) => Some("RTOR"),
        PdmsGeoParam::PrimPyramid(_) => Some("PYRA"),
        PdmsGeoParam::PrimLPyramid(_) => Some("LPYR"),
        _ => None,
    }
}

fn prim_signature(p: &PdmsGeoParam) -> Option<String> {
    // 用于检测“同一 geo_hash 下是否出现不同绝对尺寸”的碰撞风险。
    // 只取关键尺寸字段，避免打印过长。
    match p {
        PdmsGeoParam::PrimBox(b) => Some(format!(
            "BOX size=({:.3},{:.3},{:.3})",
            b.size.x, b.size.y, b.size.z
        )),
        PdmsGeoParam::PrimSphere(s) => Some(format!("SPHE r={:.3}", s.radius)),
        PdmsGeoParam::PrimLCylinder(c) => Some(format!(
            "LCYL dia={:.3} h={:.3}",
            c.pdia,
            (c.ptdi - c.pbdi).abs()
        )),
        PdmsGeoParam::PrimSCylinder(c) => Some(format!("SCYL dia={:.3} h={:.3}", c.pdia, c.phei)),
        PdmsGeoParam::PrimLSnout(s) => Some(format!(
            "SNOU dbot={:.3} dtop={:.3} h={:.3} off={:.3}",
            s.pbdm,
            s.ptdm,
            (s.ptdi - s.pbdi).abs(),
            s.poff
        )),
        PdmsGeoParam::PrimDish(d) => Some(format!(
            "DISH dia={:.3} h={:.3} prad={:.3}",
            d.pdia, d.pheig, d.prad
        )),
        PdmsGeoParam::PrimCTorus(t) => Some(format!(
            "CTOR rins={:.3} rout={:.3} ang={:.3}",
            t.rins, t.rout, t.angle
        )),
        PdmsGeoParam::PrimRTorus(t) => Some(format!(
            "RTOR rins={:.3} rout={:.3} h={:.3} ang={:.3}",
            t.rins, t.rout, t.height, t.angle
        )),
        PdmsGeoParam::PrimPyramid(py) => Some(format!(
            // Pyramid 的关键尺寸字段命名与 LPyramid 一致：pbbt/pcbt(底)、pbtp/pctp(顶)、ptdi/pbdi(高度)。
            "PYRA pbtp={:.3} pctp={:.3} pbbt={:.3} pcbt={:.3} h={:.3} off=({:.3},{:.3})",
            py.pbtp,
            py.pctp,
            py.pbbt,
            py.pcbt,
            (py.ptdi - py.pbdi).abs(),
            py.pbof,
            py.pcof
        )),
        PdmsGeoParam::PrimLPyramid(py) => Some(format!(
            "LPYR pbtp={:.3} pctp={:.3} pbbt={:.3} pcbt={:.3} h={:.3} off=({:.3},{:.3})",
            py.pbtp,
            py.pctp,
            py.pbbt,
            py.pcbt,
            (py.ptdi - py.pbdi).abs(),
            py.pbof,
            py.pcof
        )),
        _ => None,
    }
}

fn approx_one(s: f32) -> bool {
    (s - 1.0).abs() <= 1e-6
}

#[tokio::main]
async fn main() -> Result<()> {
    let owner_refno = env::var("OWNER_REFNO")
        .ok()
        .map(|s| RefnoEnum::from(s.as_str()));
    if let Some(o) = owner_refno {
        anyhow::ensure!(o.is_valid(), "无效 OWNER_REFNO: {}", o);
    }

    let scan_all_db = env::var("SCAN_ALL_DB")
        .ok()
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let max_anomalies: usize = env::var("MAX_ANOMALIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);

    let cache_dir = env::var("CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));

    aios_database::data_interface::db_meta_manager::db_meta()
        .ensure_loaded()
        .context("db_meta_info.json 未加载（请先生成 output/scene_tree/db_meta_info.json）")?;

    let dbnum = if let Some(o) = owner_refno {
        aios_database::data_interface::db_meta_manager::db_meta()
            .get_dbnum_by_refno(o)
            .context("无法从 db_meta 推导 dbnum（OWNER_REFNO 不在映射中）")?
    } else {
        anyhow::bail!("请设置 OWNER_REFNO（例如 24381/131079），以便推导 dbnum 并限制扫描范围。");
    };

    println!("🔎 audit basic primitives: dbnum={}", dbnum);
    println!("   - cache_dir: {}", cache_dir.display());
    if let Some(o) = owner_refno {
        println!("   - owner_refno: {}", o);
    }
    println!("   - scan_all_db: {}", scan_all_db);

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir)
        .await
        .context("打开 InstanceCacheManager 失败")?;

    // Pass 1：收集 (created_at, batch_id) 并排序；避免误读旧 batch。
    let batch_ids = cache.list_batches(dbnum);
    println!("   - batches: {}", batch_ids.len());
    let mut metas: Vec<(i64, String)> = Vec::with_capacity(batch_ids.len());
    for batch_id in batch_ids {
        let Some(batch) = cache.get(dbnum, &batch_id).await else {
            continue;
        };
        metas.push((batch.created_at, batch_id));
    }
    metas.sort_by(|a, b| b.0.cmp(&a.0)); // newest first

    // Pass 2：newest-first 扫描，每个 refno 只处理一次（取“最新命中”）。
    let owner_u64 = if scan_all_db {
        None
    } else {
        owner_refno.map(|o| o.refno())
    };
    // 以 RefnoEnum 去重：同一几何 refno 只取“最新命中”的 inst_geos。
    let mut seen_refnos: HashSet<RefnoEnum> = HashSet::new();

    let mut total_prim_insts = 0usize;
    let mut total_anomalies = 0usize;
    let mut printed = 0usize;
    let mut geo_hash_signatures: HashMap<u64, HashSet<String>> = HashMap::new();

    for (created_at, batch_id) in metas {
        let Some(batch) = cache.get(dbnum, &batch_id).await else {
            continue;
        };

        for geos in batch.inst_geos_map.values() {
            let r = geos.refno;
            if seen_refnos.contains(&r) {
                continue;
            }

            // owner 过滤：仅审计该 owner 下的子几何
            if let Some(owner_u64) = owner_u64 {
                let info = batch
                    .inst_info_map
                    .iter()
                    .find(|(k, _)| k.refno() == r.refno())
                    .map(|(_, v)| v);
                let Some(info) = info else {
                    continue;
                };
                if info.owner_refno.refno() != owner_u64 {
                    continue;
                }
            }

            seen_refnos.insert(r);

            for inst in &geos.insts {
                let Some(kind) = prim_kind(&inst.geo_param) else {
                    continue;
                };
                total_prim_insts += 1;

                if let Some(sig) = prim_signature(&inst.geo_param) {
                    geo_hash_signatures
                        .entry(inst.geo_hash)
                        .or_insert_with(HashSet::new)
                        .insert(sig);
                }

                // 约定：
                // - geo_hash=1/2/3 是“内置 unit mesh”（box/cylinder/sphere），尺寸必然由 scale 决定；
                //   这些不应按 unit_flag 判错（历史上 unit_flag 可能为 false）。
                let builtin_unit = matches!(inst.geo_hash, 1 | 2 | 3);

                // “非 unit mesh 却有 scale” 是最常见的重复缩放信号（排除内置 unit mesh）。
                if !builtin_unit
                    && !inst.unit_flag
                    && (!approx_one(inst.transform.scale.x)
                        || !approx_one(inst.transform.scale.y)
                        || !approx_one(inst.transform.scale.z))
                {
                    total_anomalies += 1;
                    if printed < max_anomalies {
                        printed += 1;
                        println!(
                            "\n⚠️ anomaly (latest batch): batch_id={} created_at={}",
                            batch_id, created_at
                        );
                        println!(
                            "   - refno={} type_name={} geo_hash={} kind={} unit_flag={} geo_type={:?}",
                            geos.refno,
                            geos.type_name,
                            inst.geo_hash,
                            kind,
                            inst.unit_flag,
                            inst.geo_type
                        );
                        println!("   - geo_param: {:?}", inst.geo_param);
                        println!(
                            "   - scale: [{:.6}, {:.6}, {:.6}]",
                            inst.transform.scale.x, inst.transform.scale.y, inst.transform.scale.z
                        );
                        println!(
                            "   - translation: [{:.3}, {:.3}, {:.3}]",
                            inst.transform.translation.x,
                            inst.transform.translation.y,
                            inst.transform.translation.z
                        );
                    }
                }
            }
        }
    }

    println!("\n== summary ==");
    println!("basic_prim_insts: {}", total_prim_insts);
    println!(
        "anomalies(unit_flag=false but scale!=1): {}",
        total_anomalies
    );
    if total_anomalies > printed {
        println!("(仅输出前 {} 条；可用 MAX_ANOMALIES 调整)", printed);
    }

    // 打印“同 geo_hash 多套绝对参数”的潜在碰撞：这类情况强烈暗示必须 unit 化（靠 scale 还原），
    // 或者把绝对尺寸纳入 geo_hash（放弃跨尺寸复用）。
    let mut collision_cnt = 0usize;
    for (geo_hash, sigs) in geo_hash_signatures.iter() {
        if sigs.len() > 1 {
            collision_cnt += 1;
            println!(
                "\n⚠️ geo_hash 参数碰撞: geo_hash={} signatures={}",
                geo_hash,
                sigs.len()
            );
            for s in sigs.iter().take(8) {
                println!("   - {}", s);
            }
            if sigs.len() > 8 {
                println!("   ... (省略 {})", sigs.len() - 8);
            }
        }
    }
    println!(
        "\ngeo_hash_collisions(distinct signatures > 1): {}",
        collision_cnt
    );

    Ok(())
}
