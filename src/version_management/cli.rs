use crate::options::DbOptionExt;
use clap::{Arg, ArgMatches, Command};
#[cfg(feature = "gen_model")]
use std::path::PathBuf;

pub fn model_version_command() -> Command {
    Command::new("model-version")
        .about("Query RocksDB-versioned data/model history by immutable sesno anchors")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("history")
                .about("specs/022: PE/ATT time-travel by data anchor")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("snapshot")
                        .about("Fetch a PE(+ATT) snapshot at a data anchor")
                        .arg(
                            Arg::new("refno")
                                .long("refno")
                                .value_name("REFNO")
                                .required_unless_present("pe-key"),
                        )
                        .arg(
                            Arg::new("pe-key")
                                .long("pe-key")
                                .value_name("KEY")
                                .help("Override PE record id for verification fixtures"),
                        )
                        .arg(required_u32("sesno"))
                        .arg(required_u32("dbnum"))
                        .arg(json_arg()),
                )
                .subcommand(
                    Command::new("timeline")
                        .about("List content-changing data anchors for one element")
                        .arg(
                            Arg::new("refno")
                                .long("refno")
                                .value_name("REFNO")
                                .required_unless_present("pe-key"),
                        )
                        .arg(
                            Arg::new("pe-key")
                                .long("pe-key")
                                .value_name("KEY")
                                .help("Override PE record id for verification fixtures"),
                        )
                        .arg(required_u32("from-sesno"))
                        .arg(required_u32("to-sesno"))
                        .arg(required_u32("dbnum"))
                        .arg(json_arg()),
                )
                .subcommand(
                    Command::new("diff")
                        .about("Field-level PE/ATT diff between two data anchors")
                        .arg(
                            Arg::new("refnos")
                                .long("refnos")
                                .value_name("CSV")
                                .required_unless_present("pe-key"),
                        )
                        .arg(
                            Arg::new("pe-key")
                                .long("pe-key")
                                .value_name("KEY")
                                .help("Single-element fixture PE key"),
                        )
                        .arg(required_u32("from-sesno"))
                        .arg(required_u32("to-sesno"))
                        .arg(required_u32("dbnum"))
                        .arg(json_arg()),
                )
                .subcommand(
                    Command::new("model-snapshot")
                        .about("Fetch one model snapshot at a model_gen anchor")
                        .arg(
                            Arg::new("refno")
                                .long("refno")
                                .value_name("REFNO")
                                .required(true),
                        )
                        .arg(required_u32("sesno"))
                        .arg(required_u32("dbnum"))
                        .arg(json_arg()),
                )
                .subcommand(
                    Command::new("model-diff")
                        .about("Diff model records between two model_gen anchors")
                        .arg(
                            Arg::new("refnos")
                                .long("refnos")
                                .value_name("CSV")
                                .required(true),
                        )
                        .arg(required_u32("from-sesno"))
                        .arg(required_u32("to-sesno"))
                        .arg(required_u32("dbnum"))
                        .arg(json_arg()),
                ),
        )
        .subcommand(
            Command::new("export")
                .about("Export model records from a resolved model_gen anchor")
                .arg(required_u32("dbnum"))
                .arg(required_u32("sesno"))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_name("FORMAT")
                        .value_parser(["v3-json"])
                        .default_value("v3-json"),
                )
                .arg(
                    Arg::new("output")
                        .long("output")
                        .value_name("DIR")
                        .help("Output directory; defaults to project output/v3_history"),
                )
                .arg(
                    Arg::new("target-unit")
                        .long("target-unit")
                        .value_name("UNIT")
                        .default_value("mm"),
                )
                .arg(
                    Arg::new("rotate-z-up-to-y-up")
                        .long("rotate-z-up-to-y-up")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("verbose")
                        .long("verbose")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(json_arg()),
        )
        .subcommand(
            Command::new("resolve-anchor")
                .about("Resolve a full/incremental data anchor for (dbnum, sesno)")
                .arg(required_u32("dbnum"))
                .arg(required_u32("sesno"))
                .arg(
                    Arg::new("exact-only")
                        .long("exact-only")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(json_arg()),
        )
        .subcommand(
            Command::new("backfill-pe-cata-hash")
                .about("specs/023 M0/T2: backfill pe.cata_hash (D1-A) from ele_reuse_relate edges; optionally compute misses from ATT maps")
                .arg(required_u32("dbnum"))
                .arg(
                    Arg::new("batch-size")
                        .long("batch-size")
                        .value_parser(clap::value_parser!(usize))
                        .default_value("500")
                        .help("Page/apply chunk size (clamped to 50..=1000)"),
                )
                .arg(
                    Arg::new("compute-missing")
                        .long("compute-missing")
                        .help("For rows without a reuse edge, compute cata_hash from ATT maps (slow, per-row query)")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(json_arg()),
        )
}

pub fn repair_missing_meshes_command() -> Command {
    Command::new("repair-missing-meshes")
        .about("Repair missing mesh files independently of model-version delivery")
        .arg(required_u32("dbnum"))
        .arg(
            Arg::new("project")
                .long("project")
                .value_name("PROJECT")
                .help("Project name override"),
        )
        .arg(
            Arg::new("report-file")
                .long("report-file")
                .value_name("FILE")
                .required(true),
        )
        .arg(
            Arg::new("mesh-root")
                .long("mesh-root")
                .value_name("DIR")
                .help("Mesh output root; defaults to DbOption meshes_path"),
        )
        .arg(
            Arg::new("limit")
                .long("limit")
                .value_parser(clap::value_parser!(usize))
                .value_name("N"),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("retry-bad")
                .long("retry-bad")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(json_arg())
}

fn required_u32(name: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .value_parser(clap::value_parser!(u32))
        .required(true)
}

fn json_arg() -> Arg {
    Arg::new("json")
        .long("json")
        .action(clap::ArgAction::SetTrue)
}

pub async fn handle_model_version_command(
    matches: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<bool> {
    let Some(model_matches) = matches.subcommand_matches("model-version") else {
        return Ok(false);
    };
    match model_matches.subcommand() {
        Some(("history", history)) => handle_history_command(history, db_option_ext).await?,
        Some(("export", sub)) => handle_model_export_command(sub, db_option_ext).await?,
        Some(("resolve-anchor", sub)) => handle_resolve_anchor_command(sub).await?,
        Some(("backfill-pe-cata-hash", sub)) => handle_backfill_pe_cata_hash_command(sub).await?,
        _ => unreachable!("subcommand_required by clap"),
    }
    Ok(true)
}

/// specs/023 M0/T2（D1 方案 A）：存量 pe 行回填 `cata_hash` 字段。
///
/// 数据来源两级：
/// 1. **ele_reuse_relate 边**（full 解析期产物，`pe->ele_reuse_relate->inst_info:⟨hash⟩`）：
///    批量图查直接搬运，快路径；
/// 2. `--compute-missing`：无边的行（如旧二进制增量新增的元素）逐行取 ATT map 重算
///    `cal_cata_hash()`（慢路径，逐 refno 查询，显式 opt-in）。
///
/// 幂等可重跑：UPDATE 只 SET cata_hash，不触碰其他字段；重复执行结果一致。
/// 增量常态维护由 sesno_increment 的 UPSERT 注入负责，本命令只管存量。
async fn handle_backfill_pe_cata_hash_command(sub: &ArgMatches) -> anyhow::Result<()> {
    use aios_core::utils::RecordIdExt;
    use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
    use serde::Deserialize;
    use surrealdb::types::{RecordId, SurrealValue};

    #[derive(Debug, Deserialize, SurrealValue)]
    struct EdgeHashRow {
        p: RefnoEnum,
        /// `->ele_reuse_relate.out`（0 或 1 个 inst_info 记录 id，key 即 cata_hash）
        #[serde(default)]
        h: Vec<RecordId>,
    }

    let dbnum = *sub.get_one::<u32>("dbnum").expect("required by clap");
    let batch = (*sub.get_one::<usize>("batch-size").expect("defaulted")).clamp(50, 1000);
    let compute_missing = sub.get_flag("compute-missing");
    let dry_run = sub.get_flag("dry-run");
    let json_output = sub.get_flag("json");
    let started = std::time::Instant::now();

    // D3 索引（幂等）：分页枚举 WHERE dbnum 依赖它
    crate::versioned_db::pe_owner_tree::PeOwnerTreeStore::ensure_pe_dbnum_noun_index().await?;

    let mut scanned = 0usize;
    let mut from_edge = 0usize;
    let mut computed = 0usize;
    let mut no_hash = 0usize;
    let mut updates_applied = 0usize;
    let mut start = 0usize;

    loop {
        let page_sql =
            format!("SELECT VALUE id FROM pe WHERE dbnum = {dbnum} START {start} LIMIT {batch};");
        let ids: Vec<RefnoEnum> = project_primary_db().query_take(&page_sql, 0).await?;
        if ids.is_empty() {
            break;
        }
        start += ids.len();
        scanned += ids.len();

        let keys = ids
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(", ");
        let edge_sql =
            format!("SELECT VALUE {{ p: id, h: ->ele_reuse_relate.out }} FROM [{keys}];");
        let rows: Vec<EdgeHashRow> = project_primary_db().query_take(&edge_sql, 0).await?;

        let mut update_sqls: Vec<String> = Vec::new();
        for row in rows {
            let edge_hash = row
                .h
                .first()
                .map(|rid| rid.to_mesh_id())
                .filter(|h| !h.is_empty());
            let hash = if let Some(h) = edge_hash {
                from_edge += 1;
                Some(h)
            } else if compute_missing {
                match aios_core::get_named_attmap(row.p).await {
                    Ok(att) => att.cal_cata_hash().map(|h| {
                        computed += 1;
                        h.to_string()
                    }),
                    Err(_) => None,
                }
            } else {
                None
            };
            match hash {
                Some(h) => {
                    update_sqls
                        .push(format!("UPDATE {} SET cata_hash = '{h}';", row.p.to_pe_key()));
                }
                None => no_hash += 1,
            }
        }

        if !dry_run && !update_sqls.is_empty() {
            for chunk in update_sqls.chunks(batch) {
                project_primary_db()
                    .query(chunk.join("\n"))
                    .await?
                    .check()?;
            }
        }
        updates_applied += update_sqls.len();

        if scanned % (batch * 20) == 0 {
            eprintln!(
                "[backfill-pe-cata-hash] dbnum={dbnum} scanned={scanned} updates={updates_applied} ..."
            );
        }
    }

    let summary = serde_json::json!({
        "dbnum": dbnum,
        "scanned": scanned,
        "from_edge": from_edge,
        "computed_from_att": computed,
        "no_cata_hash": no_hash,
        "updates": updates_applied,
        "applied": !dry_run,
        "compute_missing": compute_missing,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    });
    if json_output {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "backfill-pe-cata-hash dbnum={dbnum} scanned={scanned} from_edge={from_edge} computed={computed} no_hash={no_hash} updates={updates_applied} applied={} elapsed_ms={}",
            !dry_run,
            started.elapsed().as_millis()
        );
    }
    Ok(())
}

async fn handle_model_export_command(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    #[cfg(not(feature = "gen_model"))]
    {
        let _ = (sub, db_option_ext);
        anyhow::bail!(
            "model-version export 需要 gen_model feature；瘦构建不会回退到当前态或伪造历史导出"
        );
    }
    #[cfg(feature = "gen_model")]
    {
        use crate::fast_model::export_model::AnchorExportContext;
        use crate::fast_model::export_model::export_dbnum_instances_v3::export_dbnum_instances_v3_at_anchor;
        use crate::fast_model::export_model::export_transform_config::ExportTransformConfig;
        use crate::fast_model::unit_converter::LengthUnit;
        use std::str::FromStr;
        use std::sync::Arc;

        let dbnum = *sub.get_one::<u32>("dbnum").expect("required by clap");
        let sesno = *sub.get_one::<u32>("sesno").expect("required by clap");
        let format = sub
            .get_one::<String>("format")
            .map(String::as_str)
            .unwrap_or("v3-json");
        if format != "v3-json" {
            anyhow::bail!("unsupported historical export format: {format}");
        }
        crate::versioned_db::database::ensure_sesno_version_anchor_schema().await?;
        let anchor = AnchorExportContext::resolve(dbnum, sesno).await?;
        let target_unit = LengthUnit::from_str(
            sub.get_one::<String>("target-unit")
                .map(String::as_str)
                .unwrap_or("mm"),
        )
        .map_err(|error| anyhow::anyhow!("invalid --target-unit: {error}"))?;
        let transform_config = ExportTransformConfig {
            source_unit: LengthUnit::Millimeter,
            target_unit,
            apply_rotation: sub.get_flag("rotate-z-up-to-y-up"),
            inline_matrices: false,
        };
        let output_dir = sub
            .get_one::<String>("output")
            .map(PathBuf::from)
            .unwrap_or_else(|| db_option_ext.get_project_output_dir().join("v3_history"));
        let stats = export_dbnum_instances_v3_at_anchor(
            dbnum,
            &output_dir,
            Arc::new((**db_option_ext).clone()),
            sub.get_flag("verbose"),
            transform_config,
            &anchor,
        )
        .await?;
        let summary = serde_json::json!({
            "format": format,
            "dbnum": dbnum,
            "requested_sesno": sesno,
            "resolved_sesno": anchor.resolved_sesno,
            "exact": anchor.exact,
            "source": anchor.source,
            "anchored_at": anchor.anchored_at,
            "output_file": stats.output_filename,
            "bran_group_count": stats.bran_group_count,
            "equi_group_count": stats.equi_group_count,
            "ungrouped_count": stats.ungrouped_count,
            "total_component_instances": stats.total_component_instances,
            "total_tubing_instances": stats.total_tubing_instances,
            "transform_count": stats.transform_count,
            "aabb_count": stats.aabb_count,
            "elapsed_ms": stats.elapsed.as_millis(),
        });
        if sub.get_flag("json") {
            println!("{}", serde_json::to_string_pretty(&summary)?);
        } else {
            println!(
                "historical export dbnum={} requested_sesno={} resolved_sesno={} exact={} source={} anchored_at={} output={}",
                dbnum,
                sesno,
                anchor.resolved_sesno,
                anchor.exact,
                anchor.source,
                anchor.anchored_at,
                stats.output_filename
            );
        }
        Ok(())
    }
}

pub async fn handle_repair_missing_meshes_command(
    matches: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<bool> {
    let Some(sub) = matches.subcommand_matches("repair-missing-meshes") else {
        return Ok(false);
    };
    #[cfg(not(feature = "gen_model"))]
    {
        let _ = (sub, db_option_ext);
        anyhow::bail!(
            "repair-missing-meshes 需要 gen_model feature（sync-cli 瘦构建不含网格生成管线）"
        );
    }
    #[cfg(feature = "gen_model")]
    {
        use crate::version_management::missing_mesh_repair::{
            ModelMissingMeshRepairRequest, repair_missing_meshes,
        };
        let request = ModelMissingMeshRepairRequest {
            project_name: sub
                .get_one::<String>("project")
                .cloned()
                .unwrap_or_else(|| db_option_ext.inner.project_name.clone()),
            dbnum: *sub.get_one::<u32>("dbnum").expect("required by clap"),
            report_file: PathBuf::from(
                sub.get_one::<String>("report-file")
                    .expect("required by clap"),
            ),
            mesh_root: sub
                .get_one::<String>("mesh-root")
                .map(PathBuf::from)
                .or_else(|| {
                    db_option_ext
                        .inner
                        .meshes_path
                        .as_deref()
                        .map(PathBuf::from)
                })
                .unwrap_or_else(|| PathBuf::from("./assets/meshes")),
            limit: sub.get_one::<usize>("limit").copied(),
            dry_run: sub.get_flag("dry-run"),
            retry_bad: sub.get_flag("retry-bad"),
        };
        let response = repair_missing_meshes(db_option_ext, request).await?;
        if sub.get_flag("json") {
            println!("{}", serde_json::to_string_pretty(&response)?);
        } else {
            println!(
                "repaired missing meshes dbnum={} attempted={} generated={} still_missing={} report={} action={}",
                response.dbnum,
                response.attempted_hashes,
                response.generated_hashes,
                response.still_missing_hashes,
                response.report_file.display(),
                response.recommended_action
            );
        }
        Ok(true)
    }
}

async fn handle_resolve_anchor_command(sub: &ArgMatches) -> anyhow::Result<()> {
    let dbnum = *sub.get_one::<u32>("dbnum").expect("required by clap");
    let sesno = *sub.get_one::<u32>("sesno").expect("required by clap");
    crate::versioned_db::database::ensure_sesno_version_anchor_schema().await?;
    let hit = aios_core::resolve_data_anchor(dbnum, sesno)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("未找到 dbnum={dbnum} sesno<={sesno} 的 data version anchor")
        })?;
    if sub.get_flag("exact-only") && !hit.exact {
        anyhow::bail!(
            "exact-only: 无精确数据锚点 dbnum={dbnum} sesno={sesno}；最近不大于为 sesno={} anchored_at={}",
            hit.sesno,
            hit.anchored_at
        );
    }
    if sub.get_flag("json") {
        println!("{}", serde_json::to_string_pretty(&hit)?);
    } else {
        println!(
            "data anchor dbnum={} requested_sesno={} resolved_sesno={} exact={} source={} anchored_at={}",
            dbnum,
            sesno,
            hit.sesno,
            hit.exact,
            hit.source.as_deref().unwrap_or("unknown"),
            hit.anchored_at
        );
    }
    Ok(())
}

async fn handle_history_command(
    history: &ArgMatches,
    _db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    match history.subcommand() {
        Some(("snapshot", sub)) => {
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required by clap");
            let sesno = *sub.get_one::<u32>("sesno").expect("required by clap");
            let pe_key = sub.get_one::<String>("pe-key").map(String::as_str);
            let refno = parse_history_refno(sub.get_one::<String>("refno"), pe_key)?;
            match aios_core::snapshot_at(refno, sesno, Some(dbnum), pe_key).await {
                Ok(snapshot) if sub.get_flag("json") => {
                    println!("{}", serde_json::to_string_pretty(&snapshot)?)
                }
                Ok(snapshot) => {
                    println!(
                        "snapshot pe_key={} requested_sesno={} resolved_sesno={} exact={} exists={} anchored_at={}",
                        snapshot.pe_key,
                        snapshot.requested_sesno,
                        snapshot.resolved_sesno,
                        snapshot.exact_anchor,
                        snapshot.exists,
                        snapshot.anchored_at
                    );
                }
                Err(error) => anyhow::bail!("{}", aios_core::format_history_error(&error)),
            }
        }
        Some(("timeline", sub)) => {
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required by clap");
            let from_sesno = *sub.get_one::<u32>("from-sesno").expect("required by clap");
            let to_sesno = *sub.get_one::<u32>("to-sesno").expect("required by clap");
            let pe_key = sub.get_one::<String>("pe-key").map(String::as_str);
            let refno = parse_history_refno(sub.get_one::<String>("refno"), pe_key)?;
            match aios_core::timeline_with_pe_key(refno, from_sesno, to_sesno, dbnum, pe_key).await
            {
                Ok(points) if sub.get_flag("json") => {
                    println!("{}", serde_json::to_string_pretty(&points)?)
                }
                Ok(points) => {
                    for point in points {
                        println!(
                            "sesno={} changed={} exists={} hash={} at={}",
                            point.sesno,
                            point.changed_from_prev,
                            point.exists,
                            point.content_hash,
                            point.anchored_at
                        );
                    }
                }
                Err(error) => anyhow::bail!("{}", aios_core::format_history_error(&error)),
            }
        }
        Some(("diff", sub)) => {
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required by clap");
            let from_sesno = *sub.get_one::<u32>("from-sesno").expect("required by clap");
            let to_sesno = *sub.get_one::<u32>("to-sesno").expect("required by clap");
            let pe_key = sub.get_one::<String>("pe-key").map(String::as_str);
            let mut refnos = Vec::new();
            if let Some(csv) = sub.get_one::<String>("refnos") {
                for value in csv
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    let owned = value.to_string();
                    refnos.push(parse_history_refno(Some(&owned), None)?);
                }
            } else if pe_key.is_some() {
                refnos.push(parse_history_refno(None, pe_key)?);
            }
            if refnos.is_empty() {
                anyhow::bail!("--refnos or --pe-key is required");
            }
            match aios_core::diff_range_with_pe_keys(&refnos, from_sesno, to_sesno, dbnum, pe_key)
                .await
            {
                Ok(rows) if sub.get_flag("json") => {
                    println!("{}", serde_json::to_string_pretty(&rows)?)
                }
                Ok(rows) => {
                    for row in rows {
                        println!(
                            "{:?} refno={} changes={}",
                            row.kind,
                            row.refno_u64,
                            row.changes.len()
                        );
                    }
                }
                Err(error) => anyhow::bail!("{}", aios_core::format_history_error(&error)),
            }
        }
        Some(("model-snapshot", sub)) => {
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required by clap");
            let sesno = *sub.get_one::<u32>("sesno").expect("required by clap");
            let refno = parse_history_refno(sub.get_one::<String>("refno"), None)?;
            match aios_core::model_snapshot_at(refno, sesno, dbnum).await {
                Ok(snapshot) if sub.get_flag("json") => {
                    println!("{}", serde_json::to_string_pretty(&snapshot)?)
                }
                Ok(snapshot) => {
                    println!(
                        "model snapshot refno={} requested_sesno={} resolved_sesno={} exact={} source={} exists={} anchored_at={}",
                        snapshot.refno_u64,
                        snapshot.requested_sesno,
                        snapshot.anchor.sesno,
                        snapshot.anchor.exact,
                        snapshot.anchor.source.as_deref().unwrap_or("unknown"),
                        snapshot.exists,
                        snapshot.anchor.anchored_at
                    );
                }
                Err(error) => anyhow::bail!("{}", aios_core::format_history_error(&error)),
            }
        }
        Some(("model-diff", sub)) => {
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required by clap");
            let from_sesno = *sub.get_one::<u32>("from-sesno").expect("required by clap");
            let to_sesno = *sub.get_one::<u32>("to-sesno").expect("required by clap");
            let refnos =
                parse_history_refnos(sub.get_one::<String>("refnos").expect("required by clap"))?;
            match aios_core::model_diff(&refnos, from_sesno, to_sesno, dbnum).await {
                Ok(rows) if sub.get_flag("json") => {
                    println!("{}", serde_json::to_string_pretty(&rows)?)
                }
                Ok(rows) => {
                    for row in rows {
                        println!(
                            "model diff refno={} kind={:?} from={}->{}({},{}) to={}->{}({},{}) changes={}",
                            row.refno_u64,
                            row.kind,
                            row.from_requested_sesno,
                            row.from_anchor.sesno,
                            row.from_anchor.source.as_deref().unwrap_or("unknown"),
                            row.from_anchor.anchored_at,
                            row.to_requested_sesno,
                            row.to_anchor.sesno,
                            row.to_anchor.source.as_deref().unwrap_or("unknown"),
                            row.to_anchor.anchored_at,
                            row.changes.len()
                        );
                    }
                }
                Err(error) => anyhow::bail!("{}", aios_core::format_history_error(&error)),
            }
        }
        _ => unreachable!("history subcommand_required by clap"),
    }
    Ok(())
}

fn parse_history_refnos(raw: &str) -> anyhow::Result<Vec<aios_core::RefnoEnum>> {
    let mut refnos = Vec::new();
    for value in raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let owned = value.to_string();
        refnos.push(parse_history_refno(Some(&owned), None)?);
    }
    if refnos.is_empty() {
        anyhow::bail!("--refnos must contain at least one refno");
    }
    Ok(refnos)
}

fn parse_history_refno(
    refno: Option<&String>,
    pe_key: Option<&str>,
) -> anyhow::Result<aios_core::RefnoEnum> {
    use std::str::FromStr;
    if let Some(refno) = refno {
        let normalized = refno.trim().trim_start_matches('/').replace('\\', "/");
        return aios_core::RefnoEnum::from_str(&normalized)
            .map_err(|error| anyhow::anyhow!("invalid --refno '{refno}': {error}"));
    }
    if pe_key.is_some() {
        return Ok(aios_core::RefnoEnum::from(aios_core::RefU64(0)));
    }
    anyhow::bail!("--refno or --pe-key is required")
}
