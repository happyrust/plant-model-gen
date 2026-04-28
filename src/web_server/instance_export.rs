use crate::fast_model::export_instanced_bundle::export_instanced_bundle_for_refnos;
use crate::fast_model::export_model::parquet_writer::ParquetManager;
use aios_core::{RefnoEnum, get_db_option};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Export complete bundle (GLB + instances.json + manifest.json)
/// 使用现有的 export_instanced_bundle_for_refnos 方法生成临时 bundle
/// 同时写入 Parquet 文件用于持久化缓存
pub async fn export_model_bundle(
    refnos: &[RefnoEnum],
    task_id: &str,
    output_dir: &Path,
    mesh_dir: &Path,
) -> Result<PathBuf> {
    export_model_bundle_with_dbno(refnos, task_id, output_dir, mesh_dir, None).await
}

/// Export complete bundle with optional dbno for Parquet persistence
pub async fn export_model_bundle_with_dbno(
    refnos: &[RefnoEnum],
    task_id: &str,
    output_dir: &Path,
    mesh_dir: &Path,
    dbno: Option<u32>,
) -> Result<PathBuf> {
    // Create output directory
    fs::create_dir_all(output_dir).context(format!("创建输出目录失败: {:?}", output_dir))?;

    // Get database option
    let db_option_arc = get_db_option();

    // 导出 bundle 并获取 ExportData
    let export_data = export_instanced_bundle_for_refnos(
        refnos,
        mesh_dir,
        output_dir,
        db_option_arc.clone(),
        true, // verbose
    )
    .await?;

    // 如果提供了 dbno 且有数据，则根据配置写入 Parquet 用于持久化
    if let Some(db_num) = dbno {
        // 检查配置是否启用 Parquet 导出 (默认为 true)
        let export_parquet = db_option_arc.export_parquet;

        if export_parquet && export_data.total_instances > 0 {
            println!("📦 正在写入 Parquet 增量缓存 (dbno={})...", db_num);

            // 写入几何实例数据
            let parquet_manager = ParquetManager::new("assets");
            let project_name = get_db_option().project_name.clone();
            match parquet_manager.write_incremental(&export_data, db_num) {
                Ok((inst_path, trans_path)) => {
                    println!(
                        "   ✅ 几何实例 Parquet: Instances({}), Transforms({})",
                        inst_path.display(),
                        trans_path.display()
                    );
                }
                Err(e) => {
                    println!("   ⚠️ 几何实例 Parquet 失败: {}", e);
                }
            }
        }
    }

    println!("✅ Bundle exported for task {}: {:?}", task_id, output_dir);

    Ok(output_dir.to_path_buf())
}
