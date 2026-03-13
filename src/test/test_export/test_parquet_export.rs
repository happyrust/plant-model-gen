//! 测试 Parquet 导出功能

use clap::Command;
use std::path::PathBuf;
use std::sync::Arc;

/// 测试 Parquet CLI 不再暴露 cache 相关参数
#[test]
fn test_parquet_cli_args_remove_cache_flags() {
    let command = crate::cli_args::add_export_instance_args(Command::new("aios-database"));
    let arg_ids: Vec<String> = command
        .get_arguments()
        .map(|arg| arg.get_id().to_string())
        .collect();

    assert!(arg_ids.iter().any(|id| id == "export-parquet"));
    assert!(arg_ids.iter().any(|id| id == "export-dbnum-instances"));
    assert!(!arg_ids.iter().any(|id| id == "fill-missing-cache"));
    assert!(!arg_ids.iter().any(|id| id == "from-surrealdb"));
}

/// 测试导出 dbnum 实例数据为 Parquet
#[tokio::test]
#[cfg(feature = "parquet-export")]
#[ignore = "requires a SurrealDB build with rocksdb support and seeded export data"]
async fn test_export_dbnum_instances_parquet() {
    // 初始化测试数据库
    aios_core::init_surreal().await.unwrap();

    let dbnum = 1112;
    let output_dir = PathBuf::from("output/test/parquet/instances");
    let db_option_ext = crate::options::get_db_option_ext_from_path("db_options/DbOption").unwrap();
    let db_option = Arc::new(db_option_ext.inner.clone());

    // 调用导出函数
    let result = crate::fast_model::export_model::export_dbnum_instances_parquet::export_dbnum_instances_parquet(
        dbnum,
        &output_dir,
        db_option,
        true, // verbose
        None, // 使用默认毫米单位
        None, // root_refno: 导出整个 dbnum
    )
    .await;

    assert!(result.is_ok(), "Parquet 导出应该成功: {:?}", result.err());

    let stats = result.unwrap();
    println!("📊 导出统计:");
    println!("   - 实例数量: {}", stats.instance_count);
    println!("   - 几何引用数量: {}", stats.geo_instance_count);
    println!("   - TUBI 数量: {}", stats.tubing_count);
    println!("   - 变换矩阵数量: {}", stats.transform_count);
    println!("   - AABB 数量: {}", stats.aabb_count);
    println!("   - 总字节数: {}", stats.total_bytes);

    if stats.instance_count > 0 {
        // 仅当有实例数据时验证文件
        assert!(
            output_dir.join("instances.parquet").exists(),
            "instances.parquet 应该存在"
        );
        assert!(
            output_dir.join("manifest.json").exists(),
            "manifest.json 应该存在"
        );
    } else {
        println!("   ⚠️ 没有实例数据（测试环境），跳过文件检查");
    }
}

/// 测试导出 PDMS Tree 为 Parquet
#[tokio::test]
#[cfg(feature = "parquet-export")]
#[ignore = "requires a SurrealDB build with rocksdb support and seeded export data"]
async fn test_export_pdms_tree_parquet() {
    // 初始化测试数据库
    aios_core::init_surreal().await.unwrap();

    let dbnum = 1112;
    let output_dir = PathBuf::from("output/test/parquet/pdms_tree");

    // 调用导出函数
    let result =
        crate::fast_model::export_model::export_pdms_tree_parquet::export_pdms_tree_parquet(
            dbnum,
            &output_dir,
            true, // verbose
        )
        .await;

    assert!(
        result.is_ok(),
        "PDMS Tree Parquet 导出应该成功: {:?}",
        result.err()
    );

    let stats = result.unwrap();
    println!("📊 PDMS Tree 导出统计:");
    println!("   - 节点数量: {}", stats.node_count);
    println!("   - 文件大小: {} 字节", stats.total_bytes);
    println!("   - 文件名: {}", stats.file_name);

    // 验证生成的 Parquet 文件
    let parquet_file = output_dir.join(&stats.file_name);
    assert!(parquet_file.exists(), "PDMS Tree Parquet 文件应该存在");

    // 验证统计数据
    assert!(stats.node_count > 0, "应该有导出的节点");
}

/// 测试导出 Scene Tree 为 Parquet
///
/// 注意：scene_node 表需要先通过 scene_tree::init 初始化才有数据，
/// 因此此测试仅验证导出函数不会出错，不要求必须有数据。
#[tokio::test]
#[cfg(feature = "parquet-export")]
#[ignore = "requires a SurrealDB build with rocksdb support and seeded export data"]
async fn test_export_scene_tree_parquet() {
    // 初始化测试数据库
    aios_core::init_surreal().await.unwrap();

    let dbnum = 1112;
    let output_dir = PathBuf::from("output/test/parquet/scene_tree");

    // 调用导出函数
    let result =
        crate::scene_tree::parquet_export::export_scene_tree_parquet(dbnum, &output_dir).await;

    assert!(
        result.is_ok(),
        "Scene Tree Parquet 导出应该成功: {:?}",
        result.err()
    );

    let node_count = result.unwrap();
    println!("📊 Scene Tree 导出统计:");
    println!("   - 节点数量: {}", node_count);

    if node_count > 0 {
        // 仅当有数据时验证文件
        let parquet_file = output_dir.join(format!("scene_tree_{}.parquet", dbnum));
        assert!(parquet_file.exists(), "Scene Tree Parquet 文件应该存在");
    } else {
        println!("   ⚠️ scene_node 表为空（需要先初始化 scene_tree），跳过文件检查");
    }
}
