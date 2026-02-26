use super::fixtures::*;
use aios_core::RefnoEnum;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// 捕获模型回归测试 baseline fixture
///
/// 前置条件：SurrealDB 正在运行且已导入数据，output/scene_tree/ 下存在 TreeIndex 文件
///
/// 运行：
/// ```
/// cargo test --features gen_model --lib test::test_model_regression::capture_baseline::capture_17496_106028 -- --nocapture --ignored
/// ```
#[tokio::test]
#[ignore] // 需要 SurrealDB 运行，手动触发
async fn capture_17496_106028() {
    capture_baseline("17496_106028").await.unwrap();
}

async fn capture_baseline(refno_str: &str) -> anyhow::Result<()> {
    let refno: RefnoEnum = refno_str.parse().map_err(|_| {
        anyhow::anyhow!("无法解析 refno: {}", refno_str)
    })?;

    println!("📦 捕获模型回归测试 baseline: {}", refno_str);

    let out_dir = PathBuf::from(format!("test_data/model_regression/{}", refno_str));
    std::fs::create_dir_all(&out_dir)?;
    println!("   输出目录: {}", out_dir.display());

    // 1. 加载 db_meta
    use crate::data_interface::db_meta_manager::db_meta;
    db_meta().ensure_loaded()?;
    println!("✅ db_meta 加载完成");

    // 2. 收集子孙 refno（通过 TreeIndex）
    use crate::fast_model::export_model::model_exporter::collect_export_refnos;
    let all_refnos = collect_export_refnos(&[refno], true, None, true).await?;
    println!("✅ 子孙节点: {} 个（含自身）", all_refnos.len());

    let descendant_fixture = DescendantRefnos {
        root: refno_str.to_string(),
        count: all_refnos.len(),
        descendants: all_refnos.iter().map(|r| r.to_string()).collect(),
    };
    let json = serde_json::to_string_pretty(&descendant_fixture)?;
    std::fs::write(out_dir.join("descendant_refnos.json"), &json)?;
    println!("   写入 descendant_refnos.json");

    // 3. 连接 SurrealDB 查询 GeomInstQuery
    use crate::fast_model::export_model::model_exporter::query_geometry_instances_ext;
    use aios_core::init_test_surreal;
    init_test_surreal().await.map_err(|e| anyhow::anyhow!("初始化 SurrealDB 失败: {:?}", e))?;
    println!("✅ SurrealDB 连接成功");
    let geom_insts = query_geometry_instances_ext(&all_refnos, true, false, true).await?;
    println!("✅ 几何实例: {} 个", geom_insts.len());

    // 序列化
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

    let obj_candidates = [
        format!("output/debug_model/{}/{}.obj", refno_str, refno_str),
        format!("output/debug_model/{}.obj", refno_str),
        format!("output/{}.obj", refno_str),
    ];

    let all_candidates: Vec<String> = obj_candidates
        .iter()
        .cloned()
        .chain(extra_candidates.into_iter())
        .collect();

    let mut obj_found = false;
    for candidate in &all_candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            println!("   找到 OBJ 文件: {}", path.display());
            let content = std::fs::read_to_string(&path)?;
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
            let file_size_bytes = std::fs::metadata(&path)?.len();

            let stats = ObjStats {
                vertex_count,
                face_count,
                group_count,
                file_size_bytes,
            };
            println!(
                "   - vertices: {}, faces: {}, groups: {}, size: {} bytes",
                stats.vertex_count, stats.face_count, stats.group_count, stats.file_size_bytes
            );
            let json = serde_json::to_string_pretty(&stats)?;
            std::fs::write(out_dir.join("expected_obj_stats.json"), &json)?;
            std::fs::copy(&path, out_dir.join("expected.obj"))?;
            println!("   写入 expected_obj_stats.json + expected.obj");
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
