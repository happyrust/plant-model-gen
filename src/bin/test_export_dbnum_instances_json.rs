//! 测试 export_dbnum_instances_json 函数
//!
//! 运行方式:
//! cargo run --bin test_export_dbnum_instances_json --features="web_server" -- 1112

use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 获取 dbnum 参数
    let args: Vec<String> = std::env::args().collect();
    let dbnum: u32 = if args.len() > 1 {
        args[1].parse().unwrap_or(1112)
    } else {
        1112
    };

    println!("🚀 测试 export_dbnum_instances_json dbnum={}", dbnum);

    // 初始化数据库连接
    aios_core::init_surreal().await?;

    // 获取 DbOption（通过 get_db_option_ext_from_path）
    let db_option_ext = aios_database::options::get_db_option_ext_from_path("DbOption")?;
    let db_option = Arc::new(db_option_ext.inner.clone());

    // 输出目录
    let output_dir = PathBuf::from("output/instances");

    // 调用导出函数
    let stats = aios_database::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json(
        dbnum,
        &output_dir,
        db_option,
        true, // verbose
        None, // 使用默认毫米单位
    )
    .await?;

    println!("✅ 导出完成！");
    println!("📊 统计: {:?}", stats);

    Ok(())
}
