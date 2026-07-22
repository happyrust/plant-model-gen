use crate::options::DbOptionExt;
use anyhow::Context;
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
            Command::new("unit-export")
                .about("Export and record one minimum delivery unit model commit in DuckLake")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .value_parser(clap::value_parser!(u32))
                        .help("Database number; resolved from unit-refno when omitted"),
                )
                .arg(
                    Arg::new("unit-refno")
                        .long("unit-refno")
                        .value_name("REFNO")
                        .required(true),
                )
                .arg(
                    required_u32("sesno"),
                )
                .arg(
                    Arg::new("verbose")
                        .long("verbose")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(json_arg()),
        )
        .subcommand(
            Command::new("unit-list")
                .about("List model commits for one minimum delivery unit")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .value_parser(clap::value_parser!(u32))
                        .help("Database number; resolved from unit-refno when omitted"),
                )
                .arg(
                    Arg::new("unit-refno")
                        .long("unit-refno")
                        .value_name("REFNO")
                        .required(true),
                )
                .arg(json_arg()),
        )
        .subcommand(
            Command::new("unit-simulate-position")
                .about("LOCAL SMOKE ONLY: clone one unit artifact and shift one component position")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .value_parser(clap::value_parser!(u32))
                        .help("Database number; resolved from unit-refno when omitted"),
                )
                .arg(
                    Arg::new("unit-refno")
                        .long("unit-refno")
                        .value_name("REFNO")
                        .required(true),
                )
                .arg(required_u32("from-sesno"))
                .arg(required_u32("sesno"))
                .arg(
                    Arg::new("component-refno")
                        .long("component-refno")
                        .value_name("REFNO")
                        .help("Non-root component to move; defaults to the first movable child"),
                )
                .arg(
                    Arg::new("dx")
                        .long("dx")
                        .value_parser(clap::value_parser!(f64))
                        .default_value("1000")
                        .help("X translation delta in millimeters"),
                )
                .arg(
                    Arg::new("dy")
                        .long("dy")
                        .value_parser(clap::value_parser!(f64))
                        .default_value("0")
                        .help("Y translation delta in millimeters"),
                )
                .arg(
                    Arg::new("dz")
                        .long("dz")
                        .value_parser(clap::value_parser!(f64))
                        .default_value("0")
                        .help("Z translation delta in millimeters"),
                )
                .arg(
                    Arg::new("confirm-simulation")
                        .long("confirm-simulation")
                        .action(clap::ArgAction::SetTrue)
                        .required(true)
                        .help("Acknowledge that this writes a synthetic local model commit"),
                )
                .arg(json_arg()),
        )
        .subcommand(
            Command::new("bootstrap-generation-read")
                .about(
                    "Migrate the current committed Surreal state into the first DuckLake authority snapshot and bind its read replica",
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .value_parser(clap::value_parser!(u32))
                        .action(clap::ArgAction::Append)
                        .help("Optional dbnum filter; repeatable. Default: all committed dbnums"),
                )
                .arg(
                    Arg::new("authority-only")
                        .long("authority-only")
                        .action(clap::ArgAction::SetTrue)
                        .help(
                            "Only commit DuckLake authority (skip Surreal replica apply). Use with generation_read_backend=ducklake",
                        ),
                )
                .arg(
                    Arg::new("max-elements")
                        .long("max-elements")
                        .value_parser(clap::value_parser!(usize))
                        .help(
                            "Smoke/debug: truncate loaded PE to the first N elements (by id order). Hierarchy/transforms are filtered to the kept set.",
                        ),
                )
                .arg(
                    Arg::new("root-refno")
                        .long("root-refno")
                        .action(clap::ArgAction::Append)
                        .help(
                            "Bootstrap only the pe.children closure of these roots (plus attribute-referenced CATA). Enables fast Surreal-replica smoke for a BRAN/SITE.",
                        ),
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
        .subcommand(
            Command::new("rebuild-pe-owner")
                .about(
                    "specs/023: rebuild pe_owner edges for a dbnum from current pe.children, then mark pe_owner_version_meta (source=rebuild_cli)",
                )
                .arg(required_u32("dbnum"))
                .arg(
                    Arg::new("batch-size")
                        .long("batch-size")
                        .value_parser(clap::value_parser!(usize))
                        .default_value("200")
                        .help("Statements per SurrealDB request batch"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .action(clap::ArgAction::SetTrue)
                        .help("Enumerate and count only; do not write edges or meta"),
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
        Some(("unit-export", sub)) => handle_unit_export_command(sub, db_option_ext).await?,
        Some(("unit-list", sub)) => handle_unit_list_command(sub, db_option_ext).await?,
        Some(("unit-simulate-position", sub)) => {
            handle_unit_simulate_position_command(sub, db_option_ext).await?
        }
        Some(("bootstrap-generation-read", sub)) => {
            handle_generation_read_bootstrap_command(sub, db_option_ext).await?
        }
        Some(("backfill-pe-cata-hash", sub)) => handle_backfill_pe_cata_hash_command(sub).await?,
        Some(("rebuild-pe-owner", sub)) => handle_rebuild_pe_owner_command(sub).await?,
        _ => unreachable!("subcommand_required by clap"),
    }
    Ok(true)
}

#[cfg(all(
    feature = "generation-read-ducklake",
    feature = "gen_model",
    feature = "parquet-export"
))]
fn validate_unit_manifest(
    path: &std::path::Path,
    dbnum: u32,
    unit_refno: &str,
) -> anyhow::Result<serde_json::Value> {
    let bytes = std::fs::read(path).map_err(|error| {
        anyhow::anyhow!(
            "读取最小交付单元 manifest 失败: {}: {error}",
            path.display()
        )
    })?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        anyhow::anyhow!(
            "解析最小交付单元 manifest 失败: {}: {error}",
            path.display()
        )
    })?;
    anyhow::ensure!(
        manifest.get("dbnum").and_then(serde_json::Value::as_u64) == Some(u64::from(dbnum)),
        "manifest dbnum 与提交不一致: expected={dbnum} path={}",
        path.display()
    );
    anyhow::ensure!(
        manifest
            .get("root_refno")
            .and_then(serde_json::Value::as_str)
            == Some(unit_refno),
        "manifest root_refno 与提交不一致: expected={unit_refno} path={}",
        path.display()
    );
    let table_rows = |table: &str| {
        manifest
            .pointer(&format!("/tables/{table}/rows"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default()
    };
    anyhow::ensure!(
        table_rows("geo_instances") > 0 || table_rows("tubings") > 0,
        "最小交付单元 manifest 无可渲染几何，拒绝记录模型提交: {}",
        path.display()
    );
    Ok(manifest)
}

#[cfg(all(
    feature = "generation-read-ducklake",
    feature = "gen_model",
    feature = "parquet-export"
))]
fn unit_manifest_refnos(
    manifest_path: &std::path::Path,
) -> anyhow::Result<Vec<aios_core::RefnoEnum>> {
    let instances_path = manifest_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("unit manifest 缺少父目录"))?
        .join("instances.parquet");
    anyhow::ensure!(
        instances_path.is_file(),
        "unit manifest 缺少 instances.parquet: {}",
        instances_path.display()
    );
    let connection = duckdb::Connection::open_in_memory()?;
    let mut statement = connection
        .prepare("SELECT DISTINCT refno_str FROM read_parquet(?) WHERE refno_str IS NOT NULL")?;
    let refnos = statement
        .query_map([instances_path.to_string_lossy().as_ref()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|value| aios_core::RefnoEnum::from(value.as_str()))
        .collect();
    Ok(refnos)
}

#[cfg(all(
    feature = "generation-read-ducklake",
    feature = "gen_model",
    feature = "parquet-export"
))]
async fn resolve_unit_impact(
    dbnum: u32,
    root_refno: aios_core::RefnoEnum,
    sesno: u32,
    previous: Option<&crate::version_store::ModelUnitCommit>,
    project_output_dir: &std::path::Path,
) -> anyhow::Result<(crate::version_store::ModelUnitImpactKind, serde_json::Value)> {
    use std::collections::BTreeSet;

    use crate::version_store::ModelUnitImpactKind;
    use aios_core::DiffKind;

    let Some(previous) = previous else {
        return Ok((
            ModelUnitImpactKind::Mesh,
            serde_json::json!({"reason": "first_unit_commit"}),
        ));
    };
    anyhow::ensure!(
        previous.sesno < sesno,
        "unit-export 只允许在最新提交之后追加 sesno: latest={} requested={sesno}",
        previous.sesno
    );

    let previous_manifest = project_output_dir.join(&previous.manifest_path);
    let mut refnos = unit_manifest_refnos(&previous_manifest)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    refnos.insert(root_refno);
    refnos.extend(
        crate::fast_model::export_model::model_exporter::collect_export_refnos(
            &[root_refno],
            true,
            None,
            false,
        )
        .await?,
    );
    let refnos = refnos.into_iter().collect::<Vec<_>>();
    let diffs = aios_core::diff_range(&refnos, previous.sesno, sesno, dbnum).await?;

    let mut changed_refnos = 0usize;
    let mut relevant_fields = BTreeSet::new();
    for diff in &diffs {
        let from = diff
            .from_snapshot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("unit impact 缺少 from snapshot"))?;
        let to = diff
            .to_snapshot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("unit impact 缺少 to snapshot"))?;
        anyhow::ensure!(
            from.exact_anchor && from.resolved_sesno == previous.sesno,
            "unit impact 必须精确命中上一提交 sesno={}，refno={}",
            previous.sesno,
            diff.pe_key
        );
        anyhow::ensure!(
            to.exact_anchor && to.resolved_sesno == sesno,
            "unit impact 必须精确命中目标 sesno={sesno}，refno={}",
            diff.pe_key
        );
        if diff.kind == DiffKind::Unchanged {
            continue;
        }
        changed_refnos += 1;
        if matches!(
            diff.kind,
            DiffKind::Added | DiffKind::Deleted | DiffKind::Removed
        ) {
            relevant_fields.insert("record_membership".to_string());
            continue;
        }
        for change in &diff.changes {
            if crate::version_management::model_impact::field_path_affects_model(&change.path) {
                relevant_fields.insert(change.path.clone());
            }
        }
    }
    let impact_kind = if relevant_fields.is_empty() {
        ModelUnitImpactKind::Noop
    } else {
        ModelUnitImpactKind::Mesh
    };
    Ok((
        impact_kind,
        serde_json::json!({
            "reason": if impact_kind == ModelUnitImpactKind::Noop {
                "metadata_only_or_no_change"
            } else {
                "generator_input_changed"
            },
            "from_sesno": previous.sesno,
            "to_sesno": sesno,
            "examined_refnos": refnos.len(),
            "changed_refnos": changed_refnos,
            "relevant_fields": relevant_fields,
        }),
    ))
}

#[cfg(all(
    feature = "generation-read-ducklake",
    feature = "gen_model",
    feature = "parquet-export"
))]
async fn handle_unit_export_command(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    use crate::version_store::{DuckLakeAuthority, ModelUnitCommit, ModelUnitImpactKind};

    let unit_refno = sub
        .get_one::<String>("unit-refno")
        .expect("required by clap")
        .trim()
        .replace('/', "_");
    let root_refno = aios_core::RefnoEnum::from(unit_refno.as_str());
    let dbnum = match sub.get_one::<u32>("dbnum").copied() {
        Some(value) => value,
        None => crate::data_interface::db_meta_manager::resolve_dbnum_for_refno(root_refno)?,
    };
    let unit_noun = aios_core::get_type_name(root_refno)
        .await?
        .trim()
        .to_ascii_uppercase();
    anyhow::ensure!(
        crate::version_management::model_impact::is_delivery_unit_root_noun(&unit_noun),
        "{} 的 noun={}，不是最小交付单元根 BRAN/HANG/EQUI/WALL/FLOOR；其他模型仍按 dbnum 汇总导出",
        unit_refno,
        unit_noun
    );
    let sesno = *sub.get_one::<u32>("sesno").expect("required by clap");
    let project_name = db_option_ext.inner.project_name.trim().to_string();
    let authority = DuckLakeAuthority::open(db_option_ext.ducklake_config())?;

    if let Some(existing) = authority.model_unit_commit(dbnum, &unit_refno, sesno)? {
        anyhow::ensure!(
            existing.unit_noun == unit_noun && existing.project_name == project_name,
            "已存在提交与当前 unit/project 不一致"
        );
        if existing.impact_kind != ModelUnitImpactKind::Tombstone {
            let manifest = db_option_ext
                .get_project_output_dir()
                .join(&existing.manifest_path);
            let _ = validate_unit_manifest(&manifest, dbnum, &unit_refno)?;
        }
        let outcome = authority.commit_model_unit(existing)?;
        let output = serde_json::json!({
            "success": true,
            "snapshot_id": outcome.snapshot_id,
            "idempotent": true,
            "manifest_url": outcome.commit.manifest_url(),
            "commit": outcome.commit,
            "impact_evidence": {"reason": "existing_tuple"},
            "stats": serde_json::Value::Null,
        });
        if sub.get_flag("json") {
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!(
                "model unit commit already exists: ({}, {}, {}) artifact_sesno={} manifest={}",
                outcome.commit.dbnum,
                outcome.commit.unit_refno,
                outcome.commit.sesno,
                outcome.commit.artifact_sesno,
                outcome.commit.manifest_url(),
            );
        }
        return Ok(());
    }

    let previous = authority.latest_model_unit_commit(dbnum, &unit_refno)?;
    let (impact_kind, impact_evidence) = resolve_unit_impact(
        dbnum,
        root_refno,
        sesno,
        previous.as_ref(),
        &db_option_ext.get_project_output_dir(),
    )
    .await?;

    let mut stats = None;
    let (artifact_sesno, manifest_path) = match impact_kind {
        ModelUnitImpactKind::Mesh => {
            let relative_dir = PathBuf::from("model_units")
                .join(dbnum.to_string())
                .join(&unit_refno)
                .join(sesno.to_string());
            let output_dir = db_option_ext.get_project_output_dir().join(&relative_dir);
            let manifest = output_dir.join("manifest.json");
            if !manifest.is_file() {
                let export = crate::fast_model::export_model::export_dbnum_instances_parquet::export_dbnum_instances_parquet(
                        dbnum,
                        &output_dir,
                        db_option_ext.inner.clone().into(),
                        sub.get_flag("verbose"),
                        Some(crate::fast_model::unit_converter::LengthUnit::Millimeter),
                        Some(root_refno),
                    )
                    .await?;
                anyhow::ensure!(
                    export.instance_count > 0 || export.tubing_count > 0,
                    "最小交付单元导出为空，拒绝记录模型提交"
                );
                stats = Some(serde_json::json!({
                    "instances": export.instance_count,
                    "geo_instances": export.geo_instance_count,
                    "tubings": export.tubing_count,
                    "total_bytes": export.total_bytes,
                }));
            }
            let _ = validate_unit_manifest(&manifest, dbnum, &unit_refno)?;
            (
                sesno,
                relative_dir
                    .join("manifest.json")
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        }
        ModelUnitImpactKind::Noop => {
            let previous = previous
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("NoOp 提交缺少可复用的上一模型提交"))?;
            anyhow::ensure!(
                previous.unit_noun == unit_noun,
                "NoOp unit_noun 与被复用提交不一致"
            );
            anyhow::ensure!(
                previous.project_name == project_name,
                "NoOp project_name 与被复用提交不一致"
            );
            let reused_manifest = db_option_ext
                .get_project_output_dir()
                .join(&previous.manifest_path);
            let _ = validate_unit_manifest(&reused_manifest, dbnum, &unit_refno)?;
            (previous.artifact_sesno, previous.manifest_path.clone())
        }
        _ => anyhow::bail!("unit-export 当前只接受 mesh 或 noop"),
    };

    let outcome = authority.commit_model_unit(ModelUnitCommit {
        dbnum,
        unit_refno,
        unit_noun,
        sesno,
        impact_kind,
        artifact_sesno,
        project_name,
        manifest_path,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })?;
    let output = serde_json::json!({
        "success": true,
        "snapshot_id": outcome.snapshot_id,
        "idempotent": outcome.idempotent,
        "manifest_url": outcome.commit.manifest_url(),
        "commit": outcome.commit,
        "impact_evidence": impact_evidence,
        "stats": stats,
    });
    if sub.get_flag("json") {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "model unit commit: ({}, {}, {}) impact={} artifact_sesno={} manifest={}",
            outcome.commit.dbnum,
            outcome.commit.unit_refno,
            outcome.commit.sesno,
            outcome.commit.impact_kind.as_str(),
            outcome.commit.artifact_sesno,
            outcome.commit.manifest_url(),
        );
    }
    Ok(())
}

#[cfg(not(all(
    feature = "generation-read-ducklake",
    feature = "gen_model",
    feature = "parquet-export"
)))]
async fn handle_unit_export_command(
    _sub: &ArgMatches,
    _db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    anyhow::bail!("unit-export 需要 generation-read-ducklake、gen_model 与 parquet-export features")
}

#[cfg(feature = "generation-read-ducklake")]
async fn handle_unit_list_command(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    let unit_refno = sub
        .get_one::<String>("unit-refno")
        .expect("required by clap")
        .trim()
        .replace('/', "_");
    let root_refno = aios_core::RefnoEnum::from(unit_refno.as_str());
    let dbnum = match sub.get_one::<u32>("dbnum").copied() {
        Some(value) => value,
        None => crate::data_interface::db_meta_manager::resolve_dbnum_for_refno(root_refno)?,
    };
    let authority = crate::version_store::DuckLakeAuthority::open(db_option_ext.ducklake_config())?;
    let commits = authority.list_model_unit_commits(dbnum, &unit_refno)?;
    let output = commits
        .iter()
        .map(|commit| {
            serde_json::json!({
                "manifest_url": commit.manifest_url(),
                "commit": commit,
            })
        })
        .collect::<Vec<_>>();
    if sub.get_flag("json") {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        for commit in commits {
            println!(
                "({}, {}, {}) impact={} artifact_sesno={} manifest={}",
                commit.dbnum,
                commit.unit_refno,
                commit.sesno,
                commit.impact_kind.as_str(),
                commit.artifact_sesno,
                commit.manifest_url(),
            );
        }
    }
    Ok(())
}

#[cfg(all(
    feature = "generation-read-ducklake",
    feature = "gen_model",
    feature = "parquet-export"
))]
async fn handle_unit_simulate_position_command(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    use crate::version_store::{DuckLakeAuthority, ModelUnitCommit, ModelUnitImpactKind};

    let unit_refno = sub
        .get_one::<String>("unit-refno")
        .expect("required by clap")
        .trim()
        .replace('/', "_");
    let root_refno = aios_core::RefnoEnum::from(unit_refno.as_str());
    let dbnum = match sub.get_one::<u32>("dbnum").copied() {
        Some(value) => value,
        None => crate::data_interface::db_meta_manager::resolve_dbnum_for_refno(root_refno)?,
    };
    let from_sesno = *sub.get_one::<u32>("from-sesno").expect("required by clap");
    let target_sesno = *sub.get_one::<u32>("sesno").expect("required by clap");
    anyhow::ensure!(
        target_sesno > from_sesno,
        "simulation target sesno must be newer than source: source={from_sesno} target={target_sesno}"
    );

    let authority = DuckLakeAuthority::open(db_option_ext.ducklake_config())?;
    let source_commit = authority
        .model_unit_commit(dbnum, &unit_refno, from_sesno)?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "source model unit commit does not exist: ({dbnum}, {unit_refno}, {from_sesno})"
            )
        })?;
    let latest = authority
        .latest_model_unit_commit(dbnum, &unit_refno)?
        .ok_or_else(|| anyhow::anyhow!("model unit has no commits: ({dbnum}, {unit_refno})"))?;
    anyhow::ensure!(
        latest.sesno == from_sesno,
        "simulation must append to the latest model unit commit: latest={} source={from_sesno}",
        latest.sesno
    );
    anyhow::ensure!(
        authority
            .model_unit_commit(dbnum, &unit_refno, target_sesno)?
            .is_none(),
        "target model unit commit already exists: ({dbnum}, {unit_refno}, {target_sesno})"
    );
    anyhow::ensure!(
        source_commit.impact_kind != ModelUnitImpactKind::Tombstone,
        "cannot simulate from a tombstone model unit commit"
    );

    let relative_dir = PathBuf::from("model_units")
        .join(dbnum.to_string())
        .join(&unit_refno)
        .join(target_sesno.to_string());
    let project_output_dir = db_option_ext.get_project_output_dir();
    let source_manifest = project_output_dir.join(&source_commit.manifest_path);
    let source_dir = source_manifest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("source model unit manifest has no parent directory"))?;
    let target_dir = project_output_dir.join(&relative_dir);
    let delta_mm = [
        *sub.get_one::<f64>("dx").expect("defaulted by clap"),
        *sub.get_one::<f64>("dy").expect("defaulted by clap"),
        *sub.get_one::<f64>("dz").expect("defaulted by clap"),
    ];
    let simulation =
        crate::version_management::model_unit_simulation::create_position_shifted_artifact(
            source_dir,
            &target_dir,
            &unit_refno,
            from_sesno,
            target_sesno,
            sub.get_one::<String>("component-refno").map(String::as_str),
            delta_mm,
        )?;
    let manifest_path = relative_dir
        .join("manifest.json")
        .to_string_lossy()
        .replace('\\', "/");
    let _ = validate_unit_manifest(&target_dir.join("manifest.json"), dbnum, &unit_refno)?;
    let outcome = authority
        .commit_model_unit(ModelUnitCommit {
            dbnum,
            unit_refno,
            unit_noun: source_commit.unit_noun,
            sesno: target_sesno,
            impact_kind: ModelUnitImpactKind::Placement,
            artifact_sesno: target_sesno,
            project_name: source_commit.project_name,
            manifest_path,
            generated_at: chrono::Utc::now().to_rfc3339(),
        })
        .with_context(|| {
            format!(
                "synthetic artifact was written but model commit failed: {}",
                target_dir.display()
            )
        })?;
    let output = serde_json::json!({
        "success": true,
        "synthetic": true,
        "snapshot_id": outcome.snapshot_id,
        "manifest_url": outcome.commit.manifest_url(),
        "commit": outcome.commit,
        "simulation": {
            "source_sesno": from_sesno,
            "moved_refno": simulation.moved_refno,
            "delta_mm": simulation.delta_mm,
        },
    });
    if sub.get_flag("json") {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "synthetic model unit commit: ({}, {}, {}) moved={} delta_mm={:?} manifest={}",
            outcome.commit.dbnum,
            outcome.commit.unit_refno,
            outcome.commit.sesno,
            simulation.moved_refno,
            simulation.delta_mm,
            outcome.commit.manifest_url(),
        );
    }
    Ok(())
}

#[cfg(not(all(
    feature = "generation-read-ducklake",
    feature = "gen_model",
    feature = "parquet-export"
)))]
async fn handle_unit_simulate_position_command(
    _sub: &ArgMatches,
    _db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "unit-simulate-position requires generation-read-ducklake, gen_model and parquet-export features"
    )
}

#[cfg(not(feature = "generation-read-ducklake"))]
async fn handle_unit_list_command(
    _sub: &ArgMatches,
    _db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    anyhow::bail!("unit-list 需要 generation-read-ducklake feature")
}

#[cfg(feature = "generation-read-ducklake")]
async fn handle_generation_read_bootstrap_command(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    let selected = sub
        .get_many::<u32>("dbnum")
        .map(|values| values.copied().collect::<Vec<_>>());
    let authority_only = sub.get_flag("authority-only");
    let max_elements = sub.get_one::<usize>("max-elements").copied();
    let root_refnos = sub
        .get_many::<String>("root-refno")
        .map(|values| {
            values
                .map(|raw| {
                    let normalized = raw.trim().replace('/', "_");
                    aios_core::RefnoEnum::from(normalized.as_str())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let options = crate::version_store::BootstrapOptions { authority_only };

    let config = db_option_ext.ducklake_config();
    let authority =
        tokio::task::spawn_blocking(move || crate::version_store::DuckLakeAuthority::open(config))
            .await
            .map_err(|error| {
                anyhow::anyhow!("open DuckLake authority task join failed: {error}")
            })??;
    // 大库 attribute 并发过高易触发 Surreal WS Connection reset；64→24。
    let source = crate::version_store::SurrealCurrentStateBootstrapSource::new(24)?;
    let replica = crate::version_store::SurrealReplicaStore;

    let report = if !root_refnos.is_empty() {
        anyhow::ensure!(
            max_elements.is_none(),
            "--max-elements 与 --root-refno 不能同时使用"
        );
        println!(
            "generation-read bootstrap: root-refno={root_refnos:?} authority_only={authority_only}"
        );
        let state = source.load_refno_closure_state(&root_refnos).await?;
        println!(
            "generation-read bootstrap: committing elements={} edges={} transforms={} dbnums={:?}",
            state.elements.len(),
            state.hierarchy_rows.len(),
            state.transforms.len(),
            state.dbnum_sesnos.keys().copied().collect::<Vec<_>>()
        );
        crate::version_store::bootstrap_state(state, &authority, &replica, options).await?
    } else if let Some(dbnums) = selected.as_deref() {
        let dbnum_sesnos =
            crate::version_store::resolve_bootstrap_dbnum_sesnos(Some(dbnums)).await?;
        println!(
            "generation-read bootstrap: selected dbnums={:?} authority_only={authority_only} max_elements={max_elements:?}",
            dbnum_sesnos.keys().copied().collect::<Vec<_>>()
        );
        let mut state = source
            .load_selected_current_state_limited(dbnum_sesnos, max_elements)
            .await?;
        if max_elements.is_some() {
            // 截断后 children 可能指向未加载节点；只保留两端都在集合内的边。
            let keep = state.elements.len();
            truncate_bootstrap_state(&mut state, keep);
        }
        println!(
            "generation-read bootstrap: committing elements={} edges={} transforms={}",
            state.elements.len(),
            state.hierarchy_rows.len(),
            state.transforms.len()
        );
        crate::version_store::bootstrap_state(state, &authority, &replica, options).await?
    } else {
        anyhow::ensure!(
            max_elements.is_none(),
            "--max-elements 仅支持与 --dbnum 或 --root-refno 联用（避免误截断全库 bootstrap）"
        );
        println!("generation-read bootstrap: all committed dbnums authority_only={authority_only}");
        crate::version_store::bootstrap_current_state_with_options(
            &source, &authority, &replica, options,
        )
        .await?
    };

    if sub.get_flag("json") {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "generation-read bootstrap complete: snapshot={} history_start={} elements={} edges={} transforms={} replica_time={}",
            report.authoritative_snapshot_id,
            report.history_start_snapshot,
            report.element_count,
            report.hierarchy_edge_count,
            report.transform_count,
            report.replica_version_time
        );
    }
    Ok(())
}

#[cfg(feature = "generation-read-ducklake")]
fn truncate_bootstrap_state(state: &mut crate::version_store::BootstrapState, limit: usize) {
    if limit == 0 || state.elements.len() <= limit {
        return;
    }
    println!(
        "generation-read bootstrap: truncating elements {} -> {limit} (--max-elements)",
        state.elements.len()
    );
    state.elements.truncate(limit);
    let kept: std::collections::BTreeSet<_> = state
        .elements
        .iter()
        .map(|item| item.element.refno)
        .collect();
    state
        .hierarchy_rows
        .retain(|row| kept.contains(&row.parent) && kept.contains(&row.child));
    state
        .transforms
        .retain(|transform| kept.contains(&transform.refno));
}

#[cfg(not(feature = "generation-read-ducklake"))]
async fn handle_generation_read_bootstrap_command(
    _sub: &ArgMatches,
    _db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    anyhow::bail!("bootstrap-generation-read 需要 generation-read-ducklake feature")
}

/// specs/023 T018 / M3-T8：存量 versioned 站点重建 pe_owner 边。
///
/// 候选 owner 由 **pe 表 cursor 分页**枚举（`WHERE dbnum AND id > <last> ORDER BY id`，
/// 与 pe_owner_snapshot 的分页形态一致）——M3/T8 起不再依赖 scene_tree/*.tree
/// （旧 TreeIndex 枚举的"tree 与库内同源新鲜"前提在增量常态化后不成立，修 §0-4）。
/// children 取值以 pe 行 `children` 字段为准（权威来源，批量点查）。
///
/// verify-and-skip 重建（T021 实测教训，逻辑自旧实现原样保留）：
/// - 先把现存边全量读入内存（ORDER BY id 分页；无排序的 START/LIMIT 页序不稳定会漏读/重读）；
/// - 与权威 pe.children 对比，**只重写不一致的 owner**（幂等重跑零写放大）；
/// - 每 owner 先删后插；删段与插段分批 flush（versioned 引擎"同请求删边→重插同 id"
///   撞 unique_pe_owner 的边界见 sesno_increment.rs 注释）；
/// - flush 失败走逐语句慢路径：内容一致的唯一索引冲突视为幂等成功，内容不同的
///   真实冲突做"清边→重插"带核实重试；
/// - 幽灵 owner（元素已删但残留边）清理；
/// - 成功后 UPSERT `pe_owner_version_meta`（source=rebuild_cli，值=dbnum_info_table
///   该 dbnum latest sesno；查不到 sesno 拒绝写 meta）。
async fn handle_rebuild_pe_owner_command(sub: &ArgMatches) -> anyhow::Result<()> {
    use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
    use surrealdb::types::SurrealValue;

    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct PeChildrenRow {
        id: RefnoEnum,
        #[serde(default)]
        children: Option<Vec<RefnoEnum>>,
    }

    let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
    let batch_size = (*sub.get_one::<usize>("batch-size").expect("defaulted")).clamp(20, 1000);
    let dry_run = sub.get_flag("dry-run");
    let json = sub.get_flag("json");
    let started = std::time::Instant::now();

    // 0) D3 索引（幂等）：cursor 分页 `WHERE dbnum` 依赖它
    crate::versioned_db::pe_owner_tree::PeOwnerTreeStore::ensure_pe_dbnum_noun_index().await?;

    // 1) 该 dbnum 当前 latest_sesno（meta 值；缺失则拒绝，避免错误分界）
    let latest_sesno: Option<i64> = project_primary_db()
        .query_take(
            &format!(
                "SELECT VALUE sesno FROM dbnum_info_table WHERE dbnum = {dbnum} ORDER BY sesno DESC LIMIT 1;"
            ),
            0,
        )
        .await
        .map(|rows: Vec<i64>| rows.into_iter().next())
        .map_err(|e| anyhow::anyhow!("查询 dbnum_info_table latest_sesno 失败: {e}"))?;
    let Some(latest_sesno) = latest_sesno.filter(|s| *s > 0) else {
        anyhow::bail!(
            "dbnum={dbnum} 在 dbnum_info_table 无 sesno 记录，无法确定 pe_owner 可信分界；请先完成解析/增量落库"
        );
    };

    const RELATION_ROWS_PER_INSERT: usize = 500;
    let mut owners_with_children = 0usize;
    let mut edges_inserted = 0usize;
    let mut nodes_processed = 0usize;
    let mut owners_skipped = 0usize;
    let mut owners_rewritten = 0usize;
    let mut stale_edge_deleted = 0usize;

    // 2) 现存边全量读入（分页 VALUE 投影；ord 取 id 第二段）。
    // 分页必须 ORDER BY id：无排序的 START/LIMIT 页序不稳定会漏读/重读，
    // 把本已一致的 owner 误判为不一致，触发无谓重写（T021 实测教训）。
    let mut existing: std::collections::HashMap<u64, std::collections::BTreeMap<i64, u64>> =
        std::collections::HashMap::new();
    let mut existing_edge_count = 0usize;
    {
        const PAGE: usize = 100_000;
        let mut start = 0usize;
        loop {
            let sql = format!(
                "SELECT VALUE [type::string(record::id(id)[0]), type::string(in), record::id(id)[1]] FROM pe_owner ORDER BY id LIMIT {PAGE} START {start};"
            );
            let rows: Vec<(String, String, i64)> =
                project_primary_db()
                    .query_take(&sql, 0)
                    .await
                    .map_err(|e| anyhow::anyhow!("读取现存 pe_owner 边失败(start={start}): {e}"))?;
            let fetched = rows.len();
            existing_edge_count += fetched;
            for (owner_raw, child_raw, ord) in rows {
                let (Ok(owner), Ok(child)) = (
                    owner_raw.parse::<aios_core::RefU64>(),
                    child_raw.parse::<aios_core::RefU64>(),
                ) else {
                    continue;
                };
                existing.entry(owner.0).or_default().insert(ord, child.0);
            }
            if fetched < PAGE {
                break;
            }
            start += PAGE;
        }
    }
    log::info!(
        "rebuild: 现存 pe_owner {} 条边，覆盖 {} 个 owner",
        existing_edge_count,
        existing.len()
    );

    async fn exec_one(sql: &str) -> anyhow::Result<()> {
        aios_core::project_primary_db()
            .query(sql)
            .await?
            .check()
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// 从本命令生成的 INSERT RELATION 语句里提取 owner 键（`pe_owner:[<owner>, n]` 的第一段）。
    fn owner_key_from_insert_stmt(stmt: &str) -> Option<String> {
        let start = stmt.find("pe_owner:[")? + "pe_owner:[".len();
        let rest = &stmt[start..];
        let end = rest.find(',')?;
        let owner = rest[..end].trim();
        (!owner.is_empty()).then(|| owner.to_string())
    }

    /// 把本命令生成的 `INSERT RELATION INTO pe_owner [{..},{..}];` 拆成单行对象文本。
    fn split_relation_rows(stmt: &str) -> Vec<String> {
        let Some(open) = stmt.find('[') else {
            return vec![];
        };
        let Some(close) = stmt.rfind(']') else {
            return vec![];
        };
        let inner = &stmt[open + 1..close];
        // 行对象内不含嵌套花括号（本命令生成格式固定），按 `},{` 拆分
        inner
            .split("},")
            .map(|part| {
                let mut s = part.trim().to_string();
                if !s.ends_with('}') {
                    s.push('}');
                }
                s
            })
            .filter(|s| s.starts_with('{'))
            .collect()
    }

    /// 唯一索引冲突错误是否与本行意图一致（同 id 且同 [in, out]）→ 幂等成功。
    /// 错误样式：Database index `unique_pe_owner` already contains [pe:`A`, pe:`B`],
    ///           with record `pe_owner:[pe:`B`, n]`
    fn conflict_matches_intent(err_msg: &str, row_sql: &str) -> bool {
        if !err_msg.contains("unique_pe_owner") && !err_msg.contains("already exists") {
            return false;
        }
        let norm = |s: &str| s.replace(['`', ' '], "");
        let row = norm(row_sql);
        // row 形如 {id:pe_owner:[pe:B,n],in:pe:A,out:pe:B}
        let extract = |key: &str| -> Option<String> {
            let start = row.find(key)? + key.len();
            let rest = &row[start..];
            let end = rest.find([',', '}'])?;
            Some(rest[..end].to_string())
        };
        let (Some(want_in), Some(want_out)) = (extract("in:"), extract("out:")) else {
            return false;
        };
        let err = norm(err_msg);
        // "already exists"（同 id 重建同内容）：id 在错误里，in/out 不在——退化为仅比对 record id
        if err.contains("alreadyexists") && !err.contains("contains[") {
            let want_id = extract("id:").unwrap_or_default();
            return !want_id.is_empty() && err.contains(&want_id);
        }
        err.contains(&format!("contains[{want_in},{want_out}]"))
    }

    /// 批量执行；失败时逐语句慢路径重放。
    ///
    /// 慢路径存在的原因：fixture 的 versioned 引擎在同连接高频写入下偶发 **DELETE 返回
    /// OK 但未生效**（T021 实测：随机 owner、跨运行不复现同一批），随后 INSERT 撞
    /// unique_pe_owner。兜底：对冲突的 INSERT 做"清边→点查核实确已清空→重插"的
    /// 带核实重试。id 区间删仅用于**当前态写清理**，与 research C3 的
    /// "区间扫+VERSION 读"禁令无关。
    async fn flush(stmts: &mut Vec<String>, dry_run: bool) -> anyhow::Result<()> {
        if stmts.is_empty() || dry_run {
            stmts.clear();
            return Ok(());
        }
        let sql = stmts.join("\n");
        if exec_one(&sql).await.is_ok() {
            stmts.clear();
            return Ok(());
        }
        log::warn!(
            "rebuild 批次执行失败，进入逐语句慢路径（{} 条）",
            stmts.len()
        );
        for stmt in stmts.iter() {
            if exec_one(stmt).await.is_ok() {
                continue;
            }
            let Some(owner) = owner_key_from_insert_stmt(stmt) else {
                let dump =
                    std::path::Path::new("db-data").join("rebuild_pe_owner_failed_batch.sql");
                let _ = std::fs::write(&dump, &sql);
                anyhow::bail!(
                    "rebuild 语句失败且无法按 owner 兜底（已存 {}）: {stmt}",
                    dump.display()
                );
            };
            // 逐行重放：行内容与库中完全一致的冲突视为幂等成功。
            // 背景（T021 实测）：长连接在高写入量后图遍历/条件读偶发读到陈旧视图
            // （看不到边→bulk 对比误判 mismatch→DELETE 无的放矢），而唯一索引在写入侧
            // 看到的是真实状态并报出准确的 [in, out] 与 record id——以冲突错误文本为准，
            // 与本行意图一致即数据已达目标态。
            let rows = split_relation_rows(stmt);
            if rows.is_empty() {
                let dump =
                    std::path::Path::new("db-data").join("rebuild_pe_owner_failed_batch.sql");
                let _ = std::fs::write(&dump, &sql);
                anyhow::bail!(
                    "rebuild 语句失败且无法拆行（已存 {}）: {stmt}",
                    dump.display()
                );
            }
            for row_sql in rows {
                let single = format!("INSERT RELATION INTO pe_owner [{row_sql}];");
                match exec_one(&single).await {
                    Ok(()) => {}
                    Err(e) => {
                        let msg = e.to_string();
                        if conflict_matches_intent(&msg, &row_sql) {
                            log::debug!("owner {owner} 行已达目标态（幂等冲突）");
                            continue;
                        }
                        // 内容不同的真实冲突：清边后单行重试一次
                        exec_one(&format!("DELETE {owner}<-pe_owner;")).await?;
                        exec_one(&format!(
                            "DELETE pe_owner:[{owner}, 0]..=[{owner}, 4294967295];"
                        ))
                        .await?;
                        if let Err(e2) = exec_one(&single).await {
                            let msg2 = e2.to_string();
                            if conflict_matches_intent(&msg2, &row_sql) {
                                continue;
                            }
                            let dump = std::path::Path::new("db-data")
                                .join("rebuild_pe_owner_failed_batch.sql");
                            let _ = std::fs::write(&dump, &sql);
                            anyhow::bail!(
                                "rebuild owner {owner} 行清边重试后仍失败（已存 {}）: {e2}",
                                dump.display()
                            );
                        }
                    }
                }
            }
        }
        stmts.clear();
        Ok(())
    }

    // 3) pe cursor 分页枚举候选 owner（M3/T8：替代 TreeIndex 枚举）+ verify-and-skip 重建
    const ENUM_PAGE: usize = 500;
    let mut candidate_set: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut last_key: Option<String> = None;
    loop {
        let page_sql = match &last_key {
            Some(key) => format!(
                "SELECT VALUE id FROM pe WHERE dbnum = {dbnum} AND id > {key} ORDER BY id LIMIT {ENUM_PAGE};"
            ),
            None => format!(
                "SELECT VALUE id FROM pe WHERE dbnum = {dbnum} ORDER BY id LIMIT {ENUM_PAGE};"
            ),
        };
        let ids: Vec<RefnoEnum> = project_primary_db()
            .query_take(&page_sql, 0)
            .await
            .map_err(|e| anyhow::anyhow!("pe 候选分页失败(last={last_key:?}): {e}"))?;
        if ids.is_empty() {
            break;
        }
        last_key = ids.last().map(|r| r.to_pe_key());
        candidate_set.extend(ids.iter().map(|r| r.refno().0));

        // 本页批量读 children（页大小即 chunk 大小）
        let keys = ids
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(", ");
        let rows: Vec<PeChildrenRow> = project_primary_db()
            .query_take(&format!("SELECT id, children FROM [{keys}];"), 0)
            .await
            .map_err(|e| anyhow::anyhow!("批量读取 pe.children 失败: {e}"))?;

        // Phase 1（本页）：先删；Phase 2（本页）：后插——仅针对不一致 owner
        let mut delete_stmts: Vec<String> = Vec::new();
        let mut insert_stmts: Vec<String> = Vec::new();
        for row in &rows {
            nodes_processed += 1;
            let owner_u64 = row.id.refno().0;
            let children = row.children.clone().unwrap_or_default();
            let desired: Vec<u64> = children.iter().map(|c| c.refno().0).collect();
            let current: Vec<u64> = existing
                .get(&owner_u64)
                .map(|m| m.values().copied().collect())
                .unwrap_or_default();
            if !children.is_empty() {
                owners_with_children += 1;
            }
            if desired == current {
                owners_skipped += 1;
                continue;
            }
            owners_rewritten += 1;
            let owner_key = row.id.to_pe_key();
            if !current.is_empty() {
                delete_stmts.push(format!("DELETE {owner_key}<-pe_owner;"));
            }
            if !children.is_empty() {
                edges_inserted += children.len();
                for (chunk_idx, ch) in children.chunks(RELATION_ROWS_PER_INSERT).enumerate() {
                    let rows_sql = ch
                        .iter()
                        .enumerate()
                        .map(|(i, child)| {
                            let order = chunk_idx * RELATION_ROWS_PER_INSERT + i;
                            format!(
                                "{{ id: pe_owner:[{owner_key}, {order}], in: {}, out: {owner_key} }}",
                                child.to_pe_key()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",");
                    insert_stmts.push(format!("INSERT RELATION INTO pe_owner [{rows_sql}];"));
                }
            }
        }
        for batch in delete_stmts.chunks(batch_size) {
            let mut stmts = batch.to_vec();
            flush(&mut stmts, dry_run).await?;
        }
        for batch in insert_stmts.chunks(batch_size) {
            let mut stmts = batch.to_vec();
            flush(&mut stmts, dry_run).await?;
        }

        if ids.len() < ENUM_PAGE {
            break;
        }
    }

    // 4) 幽灵 owner 清理：现存边的 owner 已不在候选集（元素已删但残留边）。
    // 注意：existing 是全库边快照，候选集只含本 dbnum——非本 dbnum 的 owner 不能算幽灵。
    {
        let _ = crate::data_interface::db_meta_manager::db_meta().ensure_loaded();
        let mut ghost_stmts: Vec<String> = Vec::new();
        for (owner_u64, edges) in &existing {
            if candidate_set.contains(owner_u64) {
                continue;
            }
            let owner_refno = aios_core::RefU64(*owner_u64);
            // 只清理属于本 dbnum 的幽灵 owner（ref0→dbnum 由 db_meta 映射；映射不到时跳过，
            // 避免误删其它 dbnum 的合法边）
            let owner_dbnum = crate::data_interface::db_meta_manager::db_meta()
                .get_dbnum_by_refno(aios_core::RefnoEnum::from(owner_refno));
            if owner_dbnum != Some(dbnum) {
                continue;
            }
            let owner_key = owner_refno.to_pe_key();
            stale_edge_deleted += edges.len();
            ghost_stmts.push(format!("DELETE {owner_key}<-pe_owner;"));
            ghost_stmts.push(format!(
                "DELETE pe_owner:[{owner_key}, 0]..=[{owner_key}, 4294967295];"
            ));
        }
        if !ghost_stmts.is_empty() {
            log::info!("rebuild: 清理幽灵 owner 残留边 {} 条", stale_edge_deleted);
            for batch in ghost_stmts.chunks(batch_size) {
                let mut stmts = batch.to_vec();
                flush(&mut stmts, dry_run).await?;
            }
        }
    }

    // 5) 固化可信分界（dry-run 不写）
    if !dry_run {
        crate::versioned_db::pe_owner_meta::upsert_maintained_since(
            dbnum,
            latest_sesno as u32,
            crate::versioned_db::pe_owner_meta::META_SOURCE_REBUILD_CLI,
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!("pe_owner_version_meta 写入失败（边已重建，可重跑本命令补写）: {e}")
        })?;
    }

    let summary = serde_json::json!({
        "dbnum": dbnum,
        "dry_run": dry_run,
        "enumeration": "pe_cursor_paging",
        "nodes_processed": nodes_processed,
        "owners_with_children": owners_with_children,
        "owners_skipped": owners_skipped,
        "owners_rewritten": owners_rewritten,
        "edges_inserted": edges_inserted,
        "ghost_edges_deleted": stale_edge_deleted,
        "maintained_since_sesno": if dry_run { serde_json::Value::Null } else { serde_json::json!(latest_sesno) },
        "meta_source": if dry_run { serde_json::Value::Null } else { serde_json::json!("rebuild_cli") },
        "duration_ms": started.elapsed().as_millis() as u64,
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "rebuild-pe-owner dbnum={dbnum} dry_run={dry_run} nodes={nodes_processed} skipped={owners_skipped} rewritten={owners_rewritten} edges_inserted={edges_inserted} ghost_deleted={stale_edge_deleted} maintained_since_sesno={} elapsed={}ms",
            if dry_run {
                "-".to_string()
            } else {
                latest_sesno.to_string()
            },
            started.elapsed().as_millis()
        );
    }
    Ok(())
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
    // cursor 分页（id > last ORDER BY id）：无序 START/LIMIT 页序不稳定会漏读/重读
    let mut last_key: Option<String> = None;

    loop {
        let page_sql = match &last_key {
            Some(key) => format!(
                "SELECT VALUE id FROM pe WHERE dbnum = {dbnum} AND id > {key} ORDER BY id LIMIT {batch};"
            ),
            None => {
                format!("SELECT VALUE id FROM pe WHERE dbnum = {dbnum} ORDER BY id LIMIT {batch};")
            }
        };
        let ids: Vec<RefnoEnum> = project_primary_db().query_take(&page_sql, 0).await?;
        if ids.is_empty() {
            break;
        }
        last_key = ids.last().map(|r| r.to_pe_key());
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
                    update_sqls.push(format!(
                        "UPDATE {} SET cata_hash = '{h}';",
                        row.p.to_pe_key()
                    ));
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
