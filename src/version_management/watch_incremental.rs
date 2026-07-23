//! `watch-incremental` 的唯一轮询实现。
//!
//! CLI、`sync_live` 与 web remote runtime 只能通过本模块触发增量。调用方须先
//! 初始化项目 SurrealDB；每轮把所有待更新 dbnum 交给同一个深层 IncrementRun，
//! 由该 seam 自持项目写锁并从各自 Committed Watermark 续跑到源文件当前 sesno。

#[cfg(feature = "mqtt")]
use std::path::PathBuf;

use crate::options::DbOptionExt;

use super::increment_run::{DbnumIncrementRange, IncrementRunOptions, IncrementRunResult};

#[derive(Debug, Clone)]
pub struct WatchIncrementalOptions {
    pub requested_dbnums: Vec<u32>,
    pub interval_secs: u64,
    pub once: bool,
    pub force_initial_scan: bool,
    pub generate_model: bool,
    pub model_impact_filter: bool,
    /// specs/023 M3/T8：触发增量模型生成前要求 pe_owner 完整性证据就绪。
    pub require_pe_owner_ready: bool,
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
            generate_model: true,
            model_impact_filter: true,
            require_pe_owner_ready: false,
            json_output: false,
            verbose: false,
        }
    }
}

/// 运行统一增量轮询。
///
/// 每次写入/追赶由深层 seam 自持项目锁，watch 本身不长期占锁。
pub async fn run_watch_incremental(
    db_option_ext: &DbOptionExt,
    options: WatchIncrementalOptions,
) -> anyhow::Result<()> {
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
        // 常驻 watch：主循环级的 db_index 刷新/打开属瞬时可恢复错误（配置文件被
        // 临时占用、网络盘抖动、sqlite 短暂被锁），记录后下一轮重试，绝不让常驻
        // watcher 因一次抖动整体退出；仅 --once（CLI 一次性运行）保持失败即返回。
        let report = match crate::data_interface::db_index::rebuild_from_config(false).await {
            Ok(report) => report,
            Err(error) => {
                eprintln!(
                    "⚠️ watch-incremental 本轮 db_index 刷新失败，{}s 后重试: {:#}",
                    options.interval_secs, error
                );
                if options.once {
                    return Err(error);
                }
                tokio::time::sleep(std::time::Duration::from_secs(options.interval_secs)).await;
                continue;
            }
        };
        let records = match crate::data_interface::db_index::DbIndexStore::open(&index_path) {
            Ok(store) => store.all_db_files(),
            Err(error) => {
                eprintln!(
                    "⚠️ watch-incremental 本轮打开 db_index 失败，{}s 后重试: {:#}",
                    options.interval_secs, error
                );
                if options.once {
                    return Err(error);
                }
                tokio::time::sleep(std::time::Duration::from_secs(options.interval_secs)).await;
                continue;
            }
        };
        let mut cycle_summaries = Vec::new();
        let mut cycle_failures: Vec<(u32, String)> = Vec::new();
        let mut update_count = 0usize;
        let mut increment_targets = Vec::new();
        #[cfg(feature = "mqtt")]
        let mut increment_records = Vec::new();
        let mut catch_up_records = Vec::new();

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
                catch_up_records.push(record);
                continue;
            }

            println!(
                "🔄 dbnum={} sesno {} -> {}，开始增量更新",
                record.dbnum, watermark, record.latest_sesno
            );
            increment_targets.push(DbnumIncrementRange {
                dbnum: record.dbnum,
                from_sesno: watermark,
                to_sesno: record.latest_sesno,
            });
            #[cfg(feature = "mqtt")]
            increment_records.push(record.clone());
        }

        let mut generation_barrier_blocked = false;
        if !increment_targets.is_empty() {
            let from_sesno = increment_targets
                .iter()
                .map(|target| target.from_sesno)
                .min()
                .unwrap_or_default();
            let to_sesno = increment_targets.iter().map(|target| target.to_sesno).max();
            let run_result = super::increment_run::run_increment(
                db_option_ext,
                IncrementRunOptions {
                    file: None,
                    dbnums: Vec::new(),
                    dbnum_ranges: increment_targets,
                    from_sesno,
                    to_sesno,
                    rescan_index: false,
                    persist_data: true,
                    recover_pending: false,
                    generate_model: options.generate_model,
                    model_impact_filter: options.model_impact_filter,
                    require_pe_owner_ready: options.require_pe_owner_ready,
                    verbose: options.verbose,
                },
                || async { Ok(()) },
            )
            .await;

            match run_result {
                Ok(result) => {
                    generation_barrier_blocked = result
                        .summary
                        .pointer("/generation_barrier/status")
                        .and_then(serde_json::Value::as_str)
                        == Some("skipped_due_to_data_barrier");
                    update_count += result.persist_stats.anchors.len();
                    #[cfg(feature = "mqtt")]
                    {
                        let committed_dbnums = result
                            .persist_stats
                            .anchors
                            .iter()
                            .map(|anchor| anchor.dbnum)
                            .collect::<std::collections::BTreeSet<_>>();
                        let committed_files = increment_records
                            .iter()
                            .filter(|record| committed_dbnums.contains(&record.dbnum))
                            .map(|record| PathBuf::from(&record.file_path))
                            .collect::<Vec<_>>();
                        if !committed_files.is_empty()
                            && let Err(error) = mqtt_file_publisher
                                .publish_source_files(&committed_files)
                                .await
                        {
                            log::error!(
                                "多库增量已部分或全部提交，但 MQTT 源文件发布失败: {error:#}"
                            );
                        }
                    }
                    if !result.failures.is_empty() {
                        cycle_failures.push((0, result.failures.join("; ")));
                    }
                    if options.json_output {
                        cycle_summaries.push(result.summary);
                    } else {
                        print_incremental_sesno_summary(&result);
                    }
                }
                Err(error) => {
                    generation_barrier_blocked = true;
                    let message = format!("{error:#}");
                    if error_is_normal_contention(&message) {
                        println!("ℹ️ 多库增量轮次让路（正常竞争/待人工恢复）: {message}");
                    } else {
                        eprintln!("❌ 多库增量轮次失败，下一轮重试: {message}");
                        cycle_failures.push((0, message));
                    }
                }
            }
        }

        if generation_barrier_blocked && options.generate_model && !catch_up_records.is_empty() {
            eprintln!(
                "⚠️ 本轮数据/欠账 barrier 未通过，跳过 {} 个无新数据 dbnum 的模型欠账追赶",
                catch_up_records.len()
            );
            if options.json_output {
                cycle_summaries.push(serde_json::json!({
                    "generation_barrier": {
                        "status": "skipped_due_to_data_barrier",
                        "skipped_catch_up_dbnums": catch_up_records
                            .iter()
                            .map(|record| record.dbnum)
                            .collect::<Vec<_>>(),
                    }
                }));
            }
        } else if options.generate_model {
            for record in catch_up_records {
                match super::model_gen_catchup::catch_up_model_generation(
                    db_option_ext,
                    record.dbnum,
                    super::model_gen_catchup::ModelGenCatchUpOptions {
                        require_pe_owner_ready: options.require_pe_owner_ready,
                        allow_full_regen: false,
                        dry_run: db_option_ext.gen_model_dry_run,
                    },
                )
                .await
                {
                    Ok(result) if result.coverage.needs_full_regen => {
                        eprintln!(
                            "⚠️ dbnum={} 模型欠账存在区间洞，需要绑定数据锚点的受控 repair",
                            record.dbnum
                        );
                        if options.json_output {
                            cycle_summaries.push(serde_json::to_value(result)?);
                        }
                    }
                    Ok(result) => {
                        if let Some(anchor) = &result.model_gen_anchor {
                            update_count += 1;
                            println!(
                                "✅ dbnum={} 模型欠账已追平到 sesno={}",
                                anchor.dbnum, anchor.sesno
                            );
                        }
                        if options.json_output
                            && (result.generation_success.is_some()
                                || !result.coverage.coverage_complete)
                        {
                            cycle_summaries.push(serde_json::to_value(result)?);
                        }
                    }
                    Err(error) => {
                        let message = format!("模型欠账追赶失败: {error:#}");
                        eprintln!("❌ dbnum={} {}", record.dbnum, message);
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
    if let Some(evidence) = result.summary.get("pe_owner_evidence")
        && !evidence.is_null()
    {
        let ready = evidence
            .get("ready")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mode = evidence
            .get("mode")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let not_ready = evidence
            .get("not_ready_dbnums")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "[]".to_string());
        println!("   pe_owner_evidence: ready={ready} mode={mode} not_ready_dbnums={not_ready}");
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
