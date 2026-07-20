//! `watch-incremental` 的唯一轮询实现。
//!
//! CLI、`sync_live` 与 web remote runtime 只能通过本模块触发增量。调用方须先
//! 初始化项目 SurrealDB；本模块持有项目写锁，并让每个 dbnum 都从 Committed
//! Watermark 续跑到源文件当前 sesno。

#[cfg(feature = "mqtt")]
use std::path::PathBuf;

use crate::options::DbOptionExt;

use super::increment_run::{IncrementRunOptions, IncrementRunResult};
use super::project_mutation_lock::ProjectMutationLock;

#[derive(Debug, Clone)]
pub struct WatchIncrementalOptions {
    pub requested_dbnums: Vec<u32>,
    pub interval_secs: u64,
    pub once: bool,
    pub force_initial_scan: bool,
    pub generate_model: bool,
    pub require_tree_index: bool,
    pub json_output: bool,
    pub verbose: bool,
}

impl Default for WatchIncrementalOptions {
    fn default() -> Self {
        Self {
            requested_dbnums: Vec::new(),
            interval_secs: 30,
            once: false,
            force_initial_scan: false,
            generate_model: false,
            require_tree_index: false,
            json_output: false,
            verbose: false,
        }
    }
}

/// 运行统一增量轮询。
///
/// 项目锁覆盖整个 watch 生命周期；单轮模式也使用同一把锁，因而不会与
/// `incremental-sesno`、regen 或另一个 watcher 交错写入。
pub async fn run_watch_incremental(
    db_option_ext: &DbOptionExt,
    options: WatchIncrementalOptions,
) -> anyhow::Result<()> {
    let _mutation_lock = ProjectMutationLock::acquire_for_current_command(db_option_ext)?;

    #[cfg(feature = "sqlite-index")]
    {
        run_with_sqlite_index(db_option_ext, options).await
    }

    #[cfg(not(feature = "sqlite-index"))]
    {
        let _ = (db_option_ext, options);
        anyhow::bail!("watch-incremental 需要 sqlite-index feature")
    }
}

#[cfg(feature = "sqlite-index")]
async fn run_with_sqlite_index(
    db_option_ext: &DbOptionExt,
    mut options: WatchIncrementalOptions,
) -> anyhow::Result<()> {
    options.interval_secs = options.interval_secs.max(1);
    options.requested_dbnums.sort_unstable();
    options.requested_dbnums.dedup();

    let index_path =
        crate::data_interface::db_index::default_index_path(&db_option_ext.inner.project_name);
    if !index_path.exists() || options.force_initial_scan {
        let report = crate::data_interface::db_index::rebuild_from_config(true).await?;
        println!(
            "✅ watch-incremental 初始 db_index: {} 个库, {} 条 ref0 映射",
            report.db_files, report.ref0_total
        );
    }

    let candidate_count = {
        let store = crate::data_interface::db_index::DbIndexStore::open(&index_path)?;
        store
            .all_db_files()
            .into_iter()
            .filter(|record| {
                options.requested_dbnums.is_empty()
                    || options.requested_dbnums.contains(&record.dbnum)
            })
            .count()
    };
    if candidate_count == 0 {
        anyhow::bail!("watch-incremental 未找到可监控 db 文件，请先检查 DbOption 工程路径");
    }

    println!(
        "👀 watch-incremental 启动: dbnums={} interval={}s generate_model={}",
        candidate_count, options.interval_secs, options.generate_model
    );
    println!(
        "   起点语义: 每轮以 Committed Watermark 为增量起点，文件 latest sesno 仅作探测；第一轮自动补齐停机期间缺口"
    );

    #[cfg(feature = "mqtt")]
    let mqtt_file_publisher = crate::data_interface::mqtt_file_sync::MqttFilePublisher::start();
    let mut never_parsed_warned = std::collections::BTreeSet::new();
    loop {
        let report = crate::data_interface::db_index::rebuild_from_config(false).await?;
        let records = {
            let store = crate::data_interface::db_index::DbIndexStore::open(&index_path)?;
            store.all_db_files()
        };
        let mut cycle_summaries = Vec::new();
        let mut cycle_failures: Vec<(u32, String)> = Vec::new();
        let mut update_count = 0usize;

        for record in records {
            if !options.requested_dbnums.is_empty()
                && !options.requested_dbnums.contains(&record.dbnum)
            {
                continue;
            }
            let watermark = match crate::versioned_db::version_commit::committed_watermark(
                record.dbnum,
            )
            .await
            {
                Ok(value) => value,
                Err(error) => {
                    eprintln!(
                        "⚠️ dbnum={} 查询 Committed Watermark 失败，本轮跳过: {:#}",
                        record.dbnum, error
                    );
                    continue;
                }
            };
            if watermark == 0 {
                if never_parsed_warned.insert(record.dbnum) {
                    println!(
                        "⏭️ dbnum={} 无 Committed Watermark（从未全量解析），不做增量；请先全量建库",
                        record.dbnum
                    );
                }
                continue;
            }
            if record.latest_sesno <= watermark {
                continue;
            }

            println!(
                "🔄 dbnum={} sesno {} -> {}，开始增量更新",
                record.dbnum, watermark, record.latest_sesno
            );
            let run_result = super::increment_run::run_increment(
                db_option_ext,
                IncrementRunOptions {
                    file: None,
                    dbnums: vec![record.dbnum],
                    from_sesno: watermark,
                    to_sesno: Some(record.latest_sesno),
                    rescan_index: false,
                    persist_data: true,
                    recover_pending: false,
                    generate_model: options.generate_model,
                    require_tree_index: options.require_tree_index,
                    verbose: options.verbose,
                },
                || async { Ok(()) },
            )
            .await;

            match run_result {
                Ok(result) => {
                    update_count += 1;
                    #[cfg(feature = "mqtt")]
                    if let Err(error) = mqtt_file_publisher
                        .publish_source_files(&[PathBuf::from(&record.file_path)])
                        .await
                    {
                        log::error!(
                            "dbnum={} 已提交，但 MQTT 源文件发布失败: {error:#}",
                            record.dbnum
                        );
                    }
                    if options.json_output {
                        cycle_summaries.push(result.summary);
                    } else {
                        print_incremental_sesno_summary(&result);
                    }
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    if error_is_normal_contention(&message) {
                        println!(
                            "ℹ️ dbnum={} 本轮让路（正常竞争/待人工恢复）: {}",
                            record.dbnum, message
                        );
                    } else {
                        eprintln!(
                            "❌ dbnum={} 增量更新失败（不影响其它 dbnum，下一轮重试）: {}",
                            record.dbnum, message
                        );
                        cycle_failures.push((record.dbnum, message));
                    }
                }
            }
        }

        if options.json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "scanned": report.scanned,
                    "skipped": report.skipped,
                    "db_files": report.db_files,
                    "updates": cycle_summaries,
                    "failures": cycle_failures
                        .iter()
                        .map(|(dbnum, error)| serde_json::json!({
                            "dbnum": dbnum,
                            "error": error,
                        }))
                        .collect::<Vec<_>>(),
                }))?
            );
        } else if update_count == 0 && cycle_failures.is_empty() {
            println!(
                "ℹ️ watch-incremental 本轮无 sesno 增长 (scanned={} skipped={})",
                report.scanned, report.skipped
            );
        }

        if options.once {
            if !cycle_failures.is_empty() {
                anyhow::bail!(
                    "watch-incremental --once 存在 {} 个失败 dbnum: {}",
                    cycle_failures.len(),
                    cycle_failures
                        .iter()
                        .map(|(dbnum, _)| dbnum.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(options.interval_secs)).await;
    }

    Ok(())
}

#[cfg(feature = "sqlite-index")]
fn error_is_normal_contention(message: &str) -> bool {
    message.contains("LeaseBusy")
        || message.contains("already held")
        || message.contains("PendingCommit")
        || message.contains("pending version commit")
}

pub fn print_incremental_sesno_summary(result: &IncrementRunResult) {
    println!(
        "✅ incremental-sesno 完成: files={} sessions={} elements={} total_changes={}",
        result.summary["file_count"],
        result.summary["session_count"],
        result.summary["element_count"],
        result.summary["category_counts"]["total"]
    );
    println!(
        "   prim={} loop_owner={} bran_hanger={} basic_cata={} delete={}",
        result.summary["category_counts"]["prim"],
        result.summary["category_counts"]["loop_owner"],
        result.summary["category_counts"]["bran_hanger"],
        result.summary["category_counts"]["basic_cata"],
        result.summary["category_counts"]["delete"]
    );
    if let Some(success) = result.generation_success {
        println!("   generate_model_success={success}");
    }
    if let Some(tree_index) = result.summary.get("tree_index")
        && !tree_index.is_null()
    {
        let ready = tree_index
            .get("ready")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mode = tree_index
            .get("mode")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let missing = tree_index
            .get("missing_dbnums")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "[]".to_string());
        println!("   tree_index: ready={ready} mode={mode} missing_dbnums={missing}");
    }
    if let Some(export) = &result.parquet_export
        && export
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        println!(
            "   parquet_export: dbnums={} skipped={:?}",
            export
                .get("exported_dbnums")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "[]".to_string()),
            export
                .get("skipped_reason")
                .and_then(|value| value.as_str())
        );
    }
    if result
        .summary
        .get("data_persist_enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
    {
        println!(
            "   data_persist: sessions={} pe={} att={} uda={} deletes={} dbnum_info={}",
            result.persist_stats.session_count,
            result.persist_stats.pe_rows,
            result.persist_stats.att_rows,
            result.persist_stats.uda_rows,
            result.persist_stats.delete_count,
            result.persist_stats.dbnum_info_updates
        );
    } else {
        let reason = result
            .summary
            .get("data_persist_skipped_reason")
            .and_then(|value| value.as_str())
            .unwrap_or("--no-persist requested");
        println!("   data_persist: skipped ({reason})");
    }
}
