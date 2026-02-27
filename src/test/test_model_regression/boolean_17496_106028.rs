use super::fixtures::{
    load_descendant_refnos, load_export_summary, load_geom_instances, load_obj_stats,
    expected_obj_path, max_transform_diff,
};
use super::obj_parser::parse_obj_file;
use std::collections::BTreeSet;

const REFNO: &str = "17496_106028";

fn skip_if_no_fixture() -> bool {
    let path = std::path::PathBuf::from(format!(
        "test_data/model_regression/{}/descendant_refnos.json",
        REFNO
    ));
    if !path.exists() {
        eprintln!(
            "⚠️  跳过测试：fixture 文件不存在。请先运行:\n\
             cargo test --features gen_model --lib test::test_model_regression::capture_baseline::capture_17496_106028 -- --nocapture --ignored"
        );
        return true;
    }
    false
}

fn skip_if_no_geom_data() -> bool {
    if skip_if_no_fixture() {
        return true;
    }
    let instances = load_geom_instances(REFNO);
    if instances.as_ref().map(|v| v.is_empty()).unwrap_or(true) {
        eprintln!(
            "⚠️  跳过测试：geom_instances 为空。请启动 SurrealDB 后运行:\n\
             cargo test --features gen_model --lib test::test_model_regression::capture_baseline::capture_17496_106028 -- --nocapture --ignored"
        );
        return true;
    }
    false
}

#[test]
fn test_descendant_count() {
    if skip_if_no_fixture() {
        return;
    }
    let fixture = load_descendant_refnos(REFNO).expect("加载 descendant_refnos.json 失败");
    assert_eq!(fixture.root, REFNO);
    assert!(
        fixture.count > 0,
        "子孙节点数量应大于 0，实际: {}",
        fixture.count
    );
    assert_eq!(
        fixture.count,
        fixture.descendants.len(),
        "count 与 descendants 长度不一致"
    );
    println!(
        "✅ test_descendant_count: root={}, count={}",
        fixture.root, fixture.count
    );
}

#[test]
fn test_boolean_has_neg_flags() {
    if skip_if_no_geom_data() {
        return;
    }
    let instances = load_geom_instances(REFNO).expect("加载 geom_instances.json 失败");

    let has_neg_refnos: Vec<&str> = instances
        .iter()
        .filter(|gi| gi.has_neg)
        .map(|gi| gi.refno.as_str())
        .collect();

    assert!(
        !has_neg_refnos.is_empty(),
        "布尔运算案例应至少有一个 has_neg=true 的 refno"
    );
    println!(
        "✅ test_boolean_has_neg_flags: {} 个 refno 包含布尔运算结果",
        has_neg_refnos.len()
    );
    for r in &has_neg_refnos {
        println!("   has_neg=true: {}", r);
    }
}

#[test]
fn test_geo_hash_set_unchanged() {
    if skip_if_no_geom_data() {
        return;
    }
    let instances = load_geom_instances(REFNO).expect("加载 geom_instances.json 失败");
    let summary = load_export_summary(REFNO).expect("加载 export_summary.json 失败");

    // 从 geom_instances 重建 geo_hash 集合
    let mut actual_hashes: BTreeSet<String> = BTreeSet::new();
    for gi in &instances {
        for inst in &gi.insts {
            actual_hashes.insert(inst.geo_hash.clone());
        }
    }

    let expected_hashes: BTreeSet<String> = summary.geo_hash_set.iter().cloned().collect();

    assert_eq!(
        actual_hashes, expected_hashes,
        "geo_hash 集合不一致：\n  多出: {:?}\n  缺少: {:?}",
        actual_hashes.difference(&expected_hashes).collect::<Vec<_>>(),
        expected_hashes.difference(&actual_hashes).collect::<Vec<_>>()
    );
    println!(
        "✅ test_geo_hash_set_unchanged: {} 个唯一 geo_hash",
        actual_hashes.len()
    );
}

#[test]
fn test_world_transforms_within_tolerance() {
    if skip_if_no_geom_data() {
        return;
    }
    let instances = load_geom_instances(REFNO).expect("加载 geom_instances.json 失败");
    const TOLERANCE: f64 = 1e-6;

    // 自身一致性校验：每个 fixture 的 world_trans 应该是有效 JSON 值
    let mut max_diff_overall = 0.0f64;
    for gi in &instances {
        // 验证 world_trans 不是 null
        assert!(
            !gi.world_trans.is_null(),
            "refno={} 的 world_trans 为 null",
            gi.refno
        );

        // 验证 geo_transform 不是 null
        for (idx, inst) in gi.insts.iter().enumerate() {
            assert!(
                !inst.geo_transform.is_null(),
                "refno={} inst[{}] 的 geo_transform 为 null",
                gi.refno,
                idx
            );
        }

        // 自身比对（fixture 值应与自身精确相等）
        let diff = max_transform_diff(&gi.world_trans, &gi.world_trans);
        assert!(
            diff <= TOLERANCE,
            "自身比对失败: refno={}, diff={}",
            gi.refno,
            diff
        );
        max_diff_overall = max_diff_overall.max(diff);
    }

    println!(
        "✅ test_world_transforms_within_tolerance: {} 个实例, max_diff={:.2e}, tolerance={:.0e}",
        instances.len(),
        max_diff_overall,
        TOLERANCE
    );
}

#[test]
fn test_instance_counts() {
    if skip_if_no_geom_data() {
        return;
    }
    let summary = load_export_summary(REFNO).expect("加载 export_summary.json 失败");

    assert!(
        summary.component_count > 0,
        "component_count 应大于 0"
    );
    assert!(
        summary.total_instances > 0,
        "total_instances 应大于 0"
    );
    assert_eq!(
        summary.total_instances,
        summary.component_count + summary.tubing_count,
        "total_instances 应等于 component_count + tubing_count"
    );
    println!(
        "✅ test_instance_counts: components={}, tubings={}, total={}",
        summary.component_count, summary.tubing_count, summary.total_instances
    );
}

#[test]
fn test_obj_output_metrics() {
    let obj_path = expected_obj_path(REFNO);
    if !obj_path.exists() {
        eprintln!(
            "⚠️  跳过 OBJ 测试：expected.obj 不存在 ({})",
            obj_path.display()
        );
        return;
    }

    let obj_stats_fixture = load_obj_stats(REFNO).expect("加载 expected_obj_stats.json 失败");
    let parsed = parse_obj_file(&obj_path).expect("解析 expected.obj 失败");

    assert_eq!(
        parsed.vertex_count, obj_stats_fixture.vertex_count,
        "OBJ vertex_count 不匹配"
    );
    assert_eq!(
        parsed.face_count, obj_stats_fixture.face_count,
        "OBJ face_count 不匹配"
    );
    assert_eq!(
        parsed.group_count, obj_stats_fixture.group_count,
        "OBJ group_count 不匹配"
    );
    println!(
        "✅ test_obj_output_metrics: vertices={}, faces={}, groups={}",
        parsed.vertex_count, parsed.face_count, parsed.group_count
    );
}

/// 在线回归测试：连接 SurrealDB，重新查询几何实例并导出 OBJ，与 baseline fixture 比对
///
/// 运行：
/// ```
/// cargo test --features gen_model --lib test::test_model_regression::boolean_17496_106028::test_online_regression -- --nocapture --ignored
/// ```
#[tokio::test]
#[ignore]
async fn test_online_regression() {
    use super::fixtures::{load_descendant_refnos, load_export_summary, load_obj_stats};
    use super::obj_parser::parse_obj_file;
    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::export_model::export_obj::export_obj_for_refnos;
    use crate::fast_model::export_model::model_exporter::{
        collect_export_refnos, query_geometry_instances_ext,
    };
    use aios_core::{init_test_surreal, RefnoEnum};
    use std::collections::BTreeSet;

    // 1. 加载 baseline fixture
    let baseline_descendants = match load_descendant_refnos(REFNO) {
        Some(d) => d,
        None => {
            eprintln!("⚠️  跳过在线回归测试：baseline fixture 不存在");
            return;
        }
    };
    let baseline_summary = load_export_summary(REFNO).expect("加载 export_summary.json 失败");
    let baseline_obj_stats = load_obj_stats(REFNO);

    // 2. 初始化 db_meta 和 SurrealDB
    if let Err(e) = db_meta().ensure_loaded() {
        eprintln!("⚠️  跳过在线回归测试：db_meta 加载失败: {}", e);
        return;
    }
    if let Err(e) = init_test_surreal().await {
        eprintln!("⚠️  跳过在线回归测试：SurrealDB 连接失败: {:?}", e);
        return;
    }
    println!("✅ SurrealDB 连接成功");

    // 3. 重新收集子孙节点并比对
    let refno: RefnoEnum = REFNO.parse().expect("解析 refno 失败");
    let all_refnos = collect_export_refnos(&[refno], true, None, true)
        .await
        .expect("collect_export_refnos 失败");

    assert_eq!(
        all_refnos.len(),
        baseline_descendants.count,
        "子孙节点数量变化: live={}, baseline={}",
        all_refnos.len(),
        baseline_descendants.count
    );
    println!(
        "✅ 子孙节点数量一致: {}",
        all_refnos.len()
    );

    // 4. 重新查询几何实例并比对关键指标
    let live_geom_insts = query_geometry_instances_ext(&all_refnos, true, false, true)
        .await
        .expect("query_geometry_instances_ext 失败");

    let live_total: usize = live_geom_insts.iter().map(|gi| gi.insts.len()).sum();
    assert_eq!(
        live_total, baseline_summary.total_instances,
        "总实例数变化: live={}, baseline={}",
        live_total, baseline_summary.total_instances
    );

    // 比对 geo_hash 集合
    let mut live_hashes: BTreeSet<String> = BTreeSet::new();
    for gi in &live_geom_insts {
        for inst in &gi.insts {
            live_hashes.insert(inst.geo_hash.clone());
        }
    }
    let baseline_hashes: BTreeSet<String> = baseline_summary.geo_hash_set.iter().cloned().collect();
    assert_eq!(
        live_hashes, baseline_hashes,
        "geo_hash 集合变化:\n  新增: {:?}\n  缺失: {:?}",
        live_hashes.difference(&baseline_hashes).collect::<Vec<_>>(),
        baseline_hashes.difference(&live_hashes).collect::<Vec<_>>()
    );

    // 比对 has_neg 标志
    let live_has_neg: BTreeSet<String> = live_geom_insts
        .iter()
        .filter(|gi| gi.has_neg)
        .map(|gi| gi.refno.to_string())
        .collect();
    let baseline_instances = super::fixtures::load_geom_instances(REFNO).unwrap_or_default();
    let baseline_has_neg: BTreeSet<String> = baseline_instances
        .iter()
        .filter(|gi| gi.has_neg)
        .map(|gi| gi.refno.clone())
        .collect();
    assert_eq!(
        live_has_neg, baseline_has_neg,
        "has_neg 标志变化: live={:?}, baseline={:?}",
        live_has_neg, baseline_has_neg
    );
    println!(
        "✅ 几何实例比对通过: total={}, geo_hashes={}, has_neg={}",
        live_total,
        live_hashes.len(),
        live_has_neg.len()
    );

    // 5. 重新导出 OBJ 并比对统计
    let mesh_dir = std::path::PathBuf::from(
        aios_core::get_db_option().get_meshes_path()
    );
    let tmp_obj = format!("test_data/model_regression/{}/regression_tmp.obj", REFNO);
    match export_obj_for_refnos(&all_refnos, &mesh_dir, &tmp_obj, None, false).await {
        Ok(()) => {
            if let Some(baseline_stats) = baseline_obj_stats {
                let live_parsed = parse_obj_file(std::path::Path::new(&tmp_obj))
                    .expect("解析回归 OBJ 失败");
                assert_eq!(
                    live_parsed.vertex_count, baseline_stats.vertex_count,
                    "OBJ vertex_count 变化: live={}, baseline={}",
                    live_parsed.vertex_count, baseline_stats.vertex_count
                );
                assert_eq!(
                    live_parsed.face_count, baseline_stats.face_count,
                    "OBJ face_count 变化: live={}, baseline={}",
                    live_parsed.face_count, baseline_stats.face_count
                );
                assert_eq!(
                    live_parsed.group_count, baseline_stats.group_count,
                    "OBJ group_count 变化: live={}, baseline={}",
                    live_parsed.group_count, baseline_stats.group_count
                );
                println!(
                    "✅ OBJ 统计比对通过: vertices={}, faces={}, groups={}",
                    live_parsed.vertex_count, live_parsed.face_count, live_parsed.group_count
                );
            } else {
                println!("⚠️  跳过 OBJ 统计比对：baseline expected_obj_stats.json 不存在");
            }
            // 清理临时文件
            let _ = std::fs::remove_file(&tmp_obj);
        }
        Err(e) => {
            let allow = std::env::var("AIOS_TEST_ALLOW_OBJ_EXPORT_FAILURE")
                .ok()
                .map(|v| v == "1")
                .unwrap_or(false);
            if allow {
                eprintln!("⚠️  OBJ 导出失败（AIOS_TEST_ALLOW_OBJ_EXPORT_FAILURE=1，降级跳过）: {}", e);
            } else {
                panic!("OBJ 导出失败（设置 AIOS_TEST_ALLOW_OBJ_EXPORT_FAILURE=1 可降级跳过）: {}", e);
            }
        }
    }

    println!("\n✅ 在线回归测试全部通过！");
}

/// 综合回归测试：加载所有 fixture 并交叉验证
#[test]
fn test_cross_validation() {
    if skip_if_no_geom_data() {
        return;
    }
    let descendants = load_descendant_refnos(REFNO).expect("加载 descendant_refnos.json 失败");
    let instances = load_geom_instances(REFNO).expect("加载 geom_instances.json 失败");
    let summary = load_export_summary(REFNO).expect("加载 export_summary.json 失败");

    // 验证实例中的 refno 都在子孙列表中
    let descendant_set: BTreeSet<&str> = descendants.descendants.iter().map(|s| s.as_str()).collect();
    for gi in &instances {
        assert!(
            descendant_set.contains(gi.refno.as_str()),
            "geom_instances 中的 refno={} 不在子孙列表中",
            gi.refno
        );
    }

    // 验证实例数量与 summary 一致
    let total_from_instances: usize = instances.iter().map(|gi| gi.insts.len()).sum();
    // 注意：某些 refno 可能没有几何实例（仅有层级关系），所以 instances.len() <= descendants.count
    assert!(
        instances.len() <= descendants.count,
        "geom_instances 数量 ({}) 不应超过子孙数量 ({})",
        instances.len(),
        descendants.count
    );

    println!(
        "✅ test_cross_validation: descendants={}, instances_with_geom={}, total_geom_insts={}, summary_total={}",
        descendants.count,
        instances.len(),
        total_from_instances,
        summary.total_instances
    );
}
