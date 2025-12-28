use aios_core::{init_surreal, RefnoEnum};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_surreal().await?;

    let root_refno = RefnoEnum::from("24383_73962");
    let output_dir = std::path::Path::new("output_test");
    let mesh_dir = std::path::Path::new("meshes"); // 假设路径正确
    
    println!("🚀 开始导出模型并生成 Parquet: {}", root_refno);
    
    match crate::web_server::instance_export::export_model_bundle_with_dbno(
        &[root_refno],
        "test_task",
        output_dir,
        mesh_dir,
        Some(1) // 指定 dbno 触发 Parquet 写入
    ).await {
        Ok(_) => println!("✅ 导出完成，请检查 output_test 目录及 assets/parquet/1 目录"),
        Err(e) => eprintln!("❌ 导出失败: {}", e),
    }

    Ok(())
}
