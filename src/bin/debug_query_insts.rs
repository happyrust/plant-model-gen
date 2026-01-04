use aios_core::{RefnoEnum, init_surreal};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_surreal().await?;

    let root_refno = RefnoEnum::from("24383_73962");
    let output_dir = std::path::Path::new("output_test");
    let mesh_dir = std::path::Path::new("meshes");

    println!("🚀 准备导出模型: {}", root_refno);

    // 尝试获取正确的 dbnum
    let dbnum = if let Ok(Some(pe)) = aios_core::get_pe(root_refno).await {
        println!("✅ 识别到 PE 记录: {}, dbnum={}", pe.name, pe.dbnum);
        Some(pe.dbnum as u32)
    } else {
        println!("⚠️ 未找到 PE 记录，回退至默认 dbno=1");
        Some(1)
    };

    println!("🚀 开始导出模型并生成 Parquet (dbno={:?})", dbnum);

    match aios_database::web_server::instance_export::export_model_bundle_with_dbno(
        &[root_refno],
        "test_task",
        output_dir,
        mesh_dir,
        dbnum,
    )
    .await
    {
        Ok(_) => {
            let final_dbno = dbnum.unwrap_or(1);
            println!(
                "✅ 导出完成，请检查 output_test 目录及 assets/database_models/{} 目录",
                final_dbno
            );
        }
        Err(e) => eprintln!("❌ 导出失败: {}", e),
    }

    Ok(())
}
