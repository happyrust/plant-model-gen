#[cfg(feature = "parquet-export")]
use crate::fast_model::export_model::parquet_writer::ParquetManager;
use crate::fast_model::export_instanced_bundle::export_instanced_bundle_for_refnos;
use aios_core::{RefnoEnum, get_db_option};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

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
///
/// 当未启用 `parquet-export` feature 时，仅完成 GLB/JSON 导出，跳过 Parquet 增量写入。
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
    #[cfg(feature = "parquet-export")]
    if let Some(db_num) = dbno {
        let export_parquet = db_option_arc.export_parquet;
        if export_parquet && export_data.total_instances > 0 {
            println!("📦 正在写入 Parquet 增量缓存 (dbno={})...", db_num);

            let parquet_manager = ParquetManager::new("assets");
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

    #[cfg(not(feature = "parquet-export"))]
    {
        let _ = (dbno, &export_data, &db_option_arc);
    }

    println!("✅ Bundle exported for task {}: {:?}", task_id, output_dir);

    Ok(output_dir.to_path_buf())
}
