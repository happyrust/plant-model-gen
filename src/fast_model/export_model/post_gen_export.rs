use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::options::DbOptionExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostGenerationParquetExportReport {
    pub enabled: bool,
    pub exported_dbnums: Vec<u32>,
    pub output_dir: Option<PathBuf>,
    pub dbnum_source: Option<String>,
    pub skipped_reason: Option<String>,
}

impl PostGenerationParquetExportReport {
    fn disabled() -> Self {
        Self {
            enabled: false,
            exported_dbnums: Vec::new(),
            output_dir: None,
            dbnum_source: None,
            skipped_reason: Some("export_parquet_after_gen=false".to_string()),
        }
    }

    fn skipped(reason: impl Into<String>) -> Self {
        Self {
            enabled: true,
            exported_dbnums: Vec::new(),
            output_dir: None,
            dbnum_source: None,
            skipped_reason: Some(reason.into()),
        }
    }
}

pub async fn export_parquet_after_generation_if_enabled(
    db_option_ext: &DbOptionExt,
    dbnums_hint: Option<Vec<u32>>,
) -> Result<PostGenerationParquetExportReport> {
    if !db_option_ext.export_parquet_after_gen {
        return Ok(PostGenerationParquetExportReport::disabled());
    }

    #[cfg(feature = "parquet-export")]
    {
        export_parquet_after_generation_impl(db_option_ext, dbnums_hint).await
    }

    #[cfg(not(feature = "parquet-export"))]
    {
        let _ = dbnums_hint;
        log::warn!(
            "export_parquet_after_gen 已启用，但 parquet-export 特性未编译，跳过 Parquet 导出"
        );
        Ok(PostGenerationParquetExportReport::skipped(
            "parquet-export feature is disabled",
        ))
    }
}

#[cfg(feature = "parquet-export")]
async fn export_parquet_after_generation_impl(
    db_option_ext: &DbOptionExt,
    dbnums_hint: Option<Vec<u32>>,
) -> Result<PostGenerationParquetExportReport> {
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Instant;

    use aios_core::pdms_types::RefnoEnum;

    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::export_model::export_dbnum_instances_parquet::{
        export_dbnum_instances_parquet, query_distinct_dbnums_from_inst_relate,
    };

    let mut dbnums = dbnums_hint
        .filter(|values| !values.is_empty())
        .unwrap_or_default();

    let dbnum_source = if !dbnums.is_empty() {
        "hint"
    } else if let Some(values) = db_option_ext
        .inner
        .manual_db_nums
        .clone()
        .filter(|v| !v.is_empty())
    {
        dbnums = values;
        "manual_db_nums"
    } else if db_meta().ensure_loaded().is_ok() {
        dbnums = db_meta().get_dbnums_by_type(&db_option_ext.inner.module);
        "db_meta_module"
    } else {
        match query_distinct_dbnums_from_inst_relate().await {
            Ok(discovered) if !discovered.is_empty() => {
                log::warn!(
                    "db_meta_info.json 不可用，临时从 inst_relate 发现 Parquet 导出 dbnum；可能包含非 DESI 库: {:?}",
                    discovered
                );
                dbnums = discovered;
                "inst_relate_fallback"
            }
            Ok(_) => {
                log::warn!("export_parquet_after_gen 已启用，但 inst_relate 未发现可导出的 dbnum");
                "inst_relate_empty"
            }
            Err(err) => {
                log::error!("自动发现 Parquet 导出 dbnum 失败: {}", err);
                "inst_relate_error"
            }
        }
    };

    if let Some(exclude_nums) = &db_option_ext.inner.exclude_db_nums {
        let exclude: std::collections::HashSet<u32> = exclude_nums.iter().copied().collect();
        dbnums.retain(|dbnum| !exclude.contains(dbnum));
    }
    dbnums.sort_unstable();
    dbnums.dedup();

    if dbnums.is_empty() {
        let reason = format!("no exportable dbnum (source={dbnum_source})");
        log::warn!("export_parquet_after_gen 已启用，但没有可导出的 dbnum ({reason})");
        return Ok(PostGenerationParquetExportReport {
            enabled: true,
            exported_dbnums: Vec::new(),
            output_dir: None,
            dbnum_source: Some(dbnum_source.to_string()),
            skipped_reason: Some(reason),
        });
    }

    log::info!(
        "📦 自动导出 Parquet: source={}, dbnums={:?}",
        dbnum_source,
        dbnums
    );
    crate::perf_metrics::record_generate_progress(
        "post_gen_export_started",
        Some(&format!("source={dbnum_source} dbnums={dbnums:?}")),
        0,
    );

    // 生成阶段的专门 persist_pe_transform 已按实例落库 pe_transform，因此这里改用
    // **实例级覆盖探测**：命中即跳过整库 BFS 刷新。整库 refresh 仅在未覆盖（旧库/
    // 未按新阶段生成的数据）时作为兜底运行，不再是常规路径。
    let mut uncovered_dbnums = Vec::new();
    for dbnum in &dbnums {
        match crate::pe_transform_refresh::pe_transform_covers_instances_for_dbnum(*dbnum).await {
            Ok(true) => log::info!(
                "✅ Parquet 导出前 pe_transform 实例覆盖完好（生成阶段已就地落库），dbnum={}",
                dbnum
            ),
            Ok(false) => uncovered_dbnums.push(*dbnum),
            Err(err) => {
                log::warn!(
                    "⚠️ Parquet 导出前探测 pe_transform 实例覆盖失败，按未覆盖处理: dbnum={} err={}",
                    dbnum,
                    err
                );
                uncovered_dbnums.push(*dbnum);
            }
        }
    }

    if uncovered_dbnums.is_empty() {
        log::info!(
            "✅ Parquet 导出前 pe_transform 实例覆盖完好，跳过整库刷新: dbnums={:?}",
            dbnums
        );
    } else {
        log::info!(
            "🔄 Parquet 导出前兜底刷新未覆盖 dbnum 的 pe_transform（旧库/未按新阶段生成）: dbnums={:?}",
            uncovered_dbnums
        );
        let refreshed = crate::pe_transform_refresh::refresh_pe_transform_for_dbnums(
            &uncovered_dbnums,
            db_option_ext,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Parquet 导出前刷新 pe_transform 失败: {}", e))?;
        log::info!(
            "✅ Parquet 导出前 pe_transform 兜底刷新完成: {} 个节点",
            refreshed
        );
    }

    let base_output_dir = db_option_ext.get_project_output_dir().join("parquet");
    let db_option = Arc::new(db_option_ext.inner.clone());
    let parquet_root_refno = db_option_ext
        .inner
        .debug_model_refnos
        .as_ref()
        .and_then(|values| values.first())
        .and_then(|value| RefnoEnum::from_str(&value.replace('_', "/")).ok());
    let export_started = Instant::now();

    for (dbnum_idx, dbnum) in dbnums.iter().enumerate() {
        log::info!("📦 自动导出 dbnum={} 的 Parquet...", dbnum);
        crate::perf_metrics::record_generate_progress(
            "post_gen_export_dbnum_started",
            Some(&format!(
                "dbnum_index={}/{} dbnum={}",
                dbnum_idx + 1,
                dbnums.len(),
                dbnum
            )),
            export_started.elapsed().as_millis() as u64,
        );
        let output_dir = if parquet_root_refno.is_none() {
            let (_, artifact_dir) = crate::fast_model::export_model::export_dbnum_instances_parquet::export_dbnum_instances_parquet_latest(
                *dbnum,
                &base_output_dir,
                db_option.clone(),
                true,
                None,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Parquet 导出 dbnum={} 失败: {}", dbnum, e))?;
            artifact_dir
        } else {
            let output_dir = base_output_dir.join(dbnum.to_string());
            export_dbnum_instances_parquet(
                *dbnum,
                &output_dir,
                db_option.clone(),
                true,
                None,
                parquet_root_refno,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Parquet 导出 dbnum={} 失败: {}", dbnum, e))?;
            output_dir
        };

        #[cfg(feature = "sqlite-index")]
        {
            use crate::spatial_index::SqliteSpatialIndex;
            use crate::sqlite_index::SqliteAabbIndex;

            let idx_path = SqliteSpatialIndex::default_path();
            if let Some(parent) = idx_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let idx = SqliteAabbIndex::open(&idx_path)?;
            let import_stats = idx.refresh_dbnum_from_parquet_dir(*dbnum, &output_dir)?;
            log::info!(
                "SQLite spatial index refreshed from Parquet: dbnum={}, inserted={}, path={}",
                dbnum,
                import_stats.total_inserted,
                idx_path.display()
            );
        }

        #[cfg(not(feature = "sqlite-index"))]
        {
            log::warn!(
                "SQLite spatial index refresh skipped because sqlite-index feature is disabled"
            );
        }
        crate::perf_metrics::record_generate_progress(
            "post_gen_export_dbnum_finished",
            Some(&format!(
                "dbnum_index={}/{} dbnum={} output_dir={}",
                dbnum_idx + 1,
                dbnums.len(),
                dbnum,
                output_dir.display()
            )),
            export_started.elapsed().as_millis() as u64,
        );
    }

    let mut parquet_files = 0usize;
    let mut parquet_bytes = 0u64;
    let mut json_files = 0usize;
    let mut json_bytes = 0u64;
    for entry in walkdir::WalkDir::new(&base_output_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        match entry.path().extension().and_then(|s| s.to_str()) {
            Some("parquet") => {
                parquet_files += 1;
                parquet_bytes += len;
            }
            Some("json") => {
                json_files += 1;
                json_bytes += len;
            }
            _ => {}
        }
    }
    crate::perf_metrics::record_export_stage(
        parquet_files,
        parquet_bytes,
        json_files,
        json_bytes,
        export_started.elapsed().as_millis() as u64,
    );
    crate::perf_metrics::record_generate_progress(
        "post_gen_export_finished",
        Some(&format!(
            "parquet_files={parquet_files} parquet_bytes={parquet_bytes} json_files={json_files} json_bytes={json_bytes}"
        )),
        export_started.elapsed().as_millis() as u64,
    );

    Ok(PostGenerationParquetExportReport {
        enabled: true,
        exported_dbnums: dbnums,
        output_dir: Some(base_output_dir),
        dbnum_source: Some(dbnum_source.to_string()),
        skipped_reason: None,
    })
}
