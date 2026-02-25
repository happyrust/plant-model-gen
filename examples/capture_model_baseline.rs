//! 模型回归测试 fixture 捕获工具
//!
//! 用法：
//! ```
//! cargo run --example capture_model_baseline --features gen_model -- 17496_106028
//! ```
//!
//! 前置条件：
//! 1. 已运行过 `cargo run --bin aios-database -- --debug-model <refno> --regen-model --export-obj`
//! 2. output/scene_tree/ 下存在 db_meta_info.json 和对应的 .tree 文件
//! 3. output/foyer_cache/ 下存在 instance_cache 数据
//!
//! 输出：
//! - test_data/model_regression/<refno>/descendant_refnos.json
//! - test_data/model_regression/<refno>/geom_instances.json
//! - test_data/model_regression/<refno>/export_summary.json
//! - test_data/model_regression/<refno>/expected_obj_stats.json
//! - test_data/model_regression/<refno>/expected.obj（复制已有 OBJ）

use aios_core::RefnoEnum;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 子孙 refno 列表 fixture
#[derive(Serialize, Deserialize)]
struct DescendantRefnos {
    root: String,
    count: usize,
    descendants: Vec<String>,
}

/// 几何实例摘要（用于序列化到 JSON fixture）
#[derive(Serialize, Deserialize)]
struct GeomInstanceFixture {
    refno: String,
    owner: String,
    has_neg: bool,
    world_trans: serde_json::Value,
    insts: Vec<GeomInstEntry>,
}

#[derive(Serialize, Deserialize)]
struct GeomInstEntry {
    geo_hash: String,
    geo_transform: serde_json::Value,
    is_tubi: bool,
    unit_flag: bool,
}

/// 导出统计 fixture
#[derive(Serialize, Deserialize)]
struct ExportSummary {
    component_count: usize,
    tubing_count: usize,
    total_instances: usize,
    geo_hash_set: Vec<String>,
}

/// OBJ 文件统计 fixture
#[derive(Serialize, Deserialize)]
struct ObjStats {
    vertex_count: usize,
    face_count: usize,
    group_count: usize,
    file_size_bytes: u64,
}

fn parse_obj_stats(path: &Path) -> anyhow::Result<ObjStats> {
    let content = std::fs::read_to_string(path)?;
    let mut vertex_count = 0usize;
    let mut face_count = 0usize;
    let mut group_count = 0usize;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("v ") {
            vertex_count += 1;
        } else if trimmed.starts_with("f ") {
            face_count += 1;
        } else if trimmed.starts_with("g ") {
            group_count += 1;
        }
    }
    let file_size_bytes = std::fs::metadata(path)?.len();
    Ok(ObjStats {
        vertex_count,
        face_count,
        group_count,
        file_size_bytes,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: cargo run --example capture_model_baseline --features gen_model -- <refno>");
        eprintln!("示例: cargo run --example capture_model_baseline --features gen_model -- 17496_106028");
        std::process::exit(1);
    }

    let refno_str = &args[1];
    let refno: RefnoEnum = refno_str.parse().map_err(|_| {
        anyhow::anyhow!("无法解析 refno: {}", refno_str)
    })?;

    println!("📦 捕获模型回归测试 baseline: {}", refno_str);

    // 输出目录
    let out_dir = PathBuf::from(format!("test_data/model_regression/{}", refno_str));
    std::fs::create_dir_all(&out_dir)?;
    println!("   输出目录: {}", out_dir.display());

    // 1. 加载 db_meta
    use aios_database::data_interface::db_meta_manager::db_meta;
    db_meta().ensure_loaded()?;
    println!("✅ db_meta 加载完成");

    // 2. 收集子孙 refno（通过 TreeIndex，不需要 SurrealDB）
    use aios_database::fast_model::export_model::model_exporter::collect_export_refnos;
    let all_refnos = collect_export_refnos(&[refno], true, None, true).await?;
    println!("✅ 子孙节点: {} 个（含自身）", all_refnos.len());

    let descendant_fixture = DescendantRefnos {
        root: refno_str.clone(),
        count: all_refnos.len(),
        descendants: all_refnos.iter().map(|r| r.to_string()).collect(),
    };
    let json = serde_json::to_string_pretty(&descendant_fixture)?;
    std::fs::write(out_dir.join("descendant_refnos.json"), &json)?;
    println!("   写入 descendant_refnos.json");

    // 3. 从 foyer cache 读取 GeomInstQuery
    use aios_database::fast_model::foyer_cache::query::query_geometry_instances_ext_from_cache;
    let cache_dir = PathBuf::from("output/foyer_cache");
    let geom_insts =
        query_geometry_instances_ext_from_cache(&all_refnos, &cache_dir, true, false, true).await?;
    println!("✅ 几何实例: {} 个", geom_insts.len());

    // 序列化 GeomInstQuery 为 fixture
    let mut geom_fixtures: Vec<GeomInstanceFixture> = Vec::new();
    let mut all_geo_hashes: BTreeSet<String> = BTreeSet::new();
    let mut total_component_insts = 0usize;
    let mut total_tubi_insts = 0usize;

    for gi in &geom_insts {
        let world_trans_json = serde_json::to_value(&gi.world_trans)?;
        let mut insts = Vec::new();
        for inst in &gi.insts {
            all_geo_hashes.insert(inst.geo_hash.clone());
            if inst.is_tubi {
                total_tubi_insts += 1;
            } else {
                total_component_insts += 1;
            }
            insts.push(GeomInstEntry {
                geo_hash: inst.geo_hash.clone(),
                geo_transform: serde_json::to_value(&inst.geo_transform)?,
                is_tubi: inst.is_tubi,
                unit_flag: inst.unit_flag,
            });
        }
        geom_fixtures.push(GeomInstanceFixture {
            refno: gi.refno.to_string(),
            owner: gi.owner.to_string(),
            has_neg: gi.has_neg,
            world_trans: world_trans_json,
            insts,
        });
    }

    // 按 refno 排序，确保 fixture 稳定
    geom_fixtures.sort_by(|a, b| a.refno.cmp(&b.refno));

    let json = serde_json::to_string_pretty(&geom_fixtures)?;
    std::fs::write(out_dir.join("geom_instances.json"), &json)?;
    println!("   写入 geom_instances.json ({} 条)", geom_fixtures.len());

    // 4. 导出统计
    let export_summary = ExportSummary {
        component_count: geom_insts.iter().filter(|g| g.insts.iter().any(|i| !i.is_tubi)).count(),
        tubing_count: total_tubi_insts,
        total_instances: total_component_insts + total_tubi_insts,
        geo_hash_set: all_geo_hashes.into_iter().collect(),
    };
    let json = serde_json::to_string_pretty(&export_summary)?;
    std::fs::write(out_dir.join("export_summary.json"), &json)?;
    println!("   写入 export_summary.json");
    println!(
        "   - components: {}, tubings: {}, total: {}, unique_geo_hashes: {}",
        export_summary.component_count,
        export_summary.tubing_count,
        export_summary.total_instances,
        export_summary.geo_hash_set.len()
    );

    // 5. OBJ 文件统计 + 复制
    // 搜索多个可能的 OBJ 路径
    let obj_candidates = [
        format!("output/debug_model/{}/{}.obj", refno_str, refno_str),
        format!("output/debug_model/{}.obj", refno_str),
        format!("output/{}.obj", refno_str),
    ];

    // 也在 output/ 的子目录中搜索 (如 output/AvevaMarineSample/)
    let mut extra_candidates: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("output") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let candidate = path.join(format!("{}.obj", refno_str));
                extra_candidates.push(candidate.to_string_lossy().to_string());
            }
        }
    }

    let mut obj_found = false;
    let all_candidates: Vec<String> = obj_candidates
        .iter()
        .map(|s| s.clone())
        .chain(extra_candidates.into_iter())
        .collect();

    for candidate in &all_candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            println!("   找到 OBJ 文件: {}", path.display());
            let stats = parse_obj_stats(&path)?;
            println!(
                "   - vertices: {}, faces: {}, groups: {}, size: {} bytes",
                stats.vertex_count, stats.face_count, stats.group_count, stats.file_size_bytes
            );
            let json = serde_json::to_string_pretty(&stats)?;
            std::fs::write(out_dir.join("expected_obj_stats.json"), &json)?;
            println!("   写入 expected_obj_stats.json");

            std::fs::copy(&path, out_dir.join("expected.obj"))?;
            println!("   复制 expected.obj");
            obj_found = true;
            break;
        }
    }

    if !obj_found {
        println!("⚠️  未找到 OBJ 文件，跳过 OBJ 统计。请先运行:");
        println!("   cargo run --bin aios-database -- --debug-model {} --regen-model --export-obj", refno_str);
    }

    println!("\n✅ baseline 捕获完成！fixture 文件位于: {}", out_dir.display());
    Ok(())
}
