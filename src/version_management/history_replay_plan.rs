use crate::options::DbOptionExt;
use crate::version_management::release_package::validate_release_id_for_path;
use crate::version_management::types::{
    ModelHistoryReplayCommands, ModelHistoryReplayPrepareRequest,
    ModelHistoryReplayPrepareResponse, ModelHistoryReplaySafetyChecks,
};
use anyhow::Context;
use std::fs;
use std::path::{Component, Path, PathBuf};
use toml::Value as TomlValue;

const DERIVED_PATH_KEYS: &[&str] = &[
    "model_cache_dir",
    "transform_parquet_dir",
    "transform_ducklake_metadata",
    "transform_ducklake_data_path",
];

pub fn prepare_history_replay(
    db_option_ext: &DbOptionExt,
    request: ModelHistoryReplayPrepareRequest,
) -> anyhow::Result<ModelHistoryReplayPrepareResponse> {
    validate_prepare_request(&request)?;
    let baseline_binary_supports_surreal_save = cfg!(feature = "surreal-save");
    if !baseline_binary_supports_surreal_save {
        anyhow::bail!(
            "prepare-history-replay requires a binary built with feature `surreal-save`; \
             otherwise baseline_parse cannot honor save_db=true and the replay namespace would \
             not contain a complete database state"
        );
    }
    if !request.baseline_source_confirmed_at_from_sesno {
        anyhow::bail!(
            "prepare-history-replay requires explicit baseline source confirmation: \
             baseline_parse uses the source file's visible/current full-sync state and does not \
             reconstruct from_sesno={} from pdms-io history. Use \
             --baseline-source-confirmed-at-from-sesno only when {} is already an isolated \
             physical baseline for that session, for example from prepare-physical-baseline-snapshot; \
             otherwise use a physical baseline snapshot, restore a published baseline package, or \
             implement a proven target-sesno hydrate provider.",
            request.from_sesno,
            request.source_db_file.display()
        );
    }

    let current_surreal_ns = db_option_ext.inner.surreal_ns.to_string();
    let replay_surreal_ns = request.replay_surreal_ns.clone().unwrap_or_else(|| {
        format!(
            "{}_history_{}",
            current_surreal_ns,
            sanitize_namespace_fragment(&request.release_id)
        )
    });
    if replay_surreal_ns.trim().is_empty() {
        anyhow::bail!("replay_surreal_ns is empty");
    }
    if replay_surreal_ns == current_surreal_ns {
        anyhow::bail!(
            "replay_surreal_ns must differ from current SurrealDB namespace: {}",
            current_surreal_ns
        );
    }

    let current_output_root = db_option_ext.get_output_root();
    let current_project_output_dir =
        project_output_dir_for(&current_output_root, &request.project_name);
    let baseline_release_id = request
        .baseline_release_id
        .clone()
        .unwrap_or_else(|| format!("{}-baseline-{}", request.release_id, request.from_sesno));
    validate_release_id_for_path(&baseline_release_id)?;
    if baseline_release_id == request.release_id {
        anyhow::bail!(
            "baseline release id must differ from target release id: {}",
            baseline_release_id
        );
    }
    let baseline_dbnums = normalize_baseline_dbnums(request.dbnum, &request.baseline_dbnums);
    let replay_output_root = request.replay_output_root.clone().unwrap_or_else(|| {
        current_project_output_dir
            .join("model_versions")
            .join("replay_work")
            .join(&request.release_id)
            .join("output")
    });
    let replay_project_output_dir =
        project_output_dir_for(&replay_output_root, &request.project_name);
    let replay_parquet_dir = replay_project_output_dir
        .join("parquet")
        .join(request.dbnum.to_string());

    if paths_equivalent(&replay_output_root, &current_output_root)? {
        anyhow::bail!(
            "replay output_root must differ from current output_root: {}",
            replay_output_root.display()
        );
    }
    if paths_equivalent(&replay_output_root, &current_project_output_dir)? {
        anyhow::bail!(
            "replay output_root must not be the current project output dir: {}",
            replay_output_root.display()
        );
    }
    if paths_equivalent(&replay_parquet_dir, &request.current_parquet_dir)? {
        anyhow::bail!(
            "replay Parquet directory must differ from current Parquet directory: {}",
            replay_parquet_dir.display()
        );
    }

    let base_config_arg = strip_toml_extension(&request.base_config_arg);
    let base_config_path = config_toml_path(&base_config_arg);
    if !base_config_path.exists() {
        anyhow::bail!(
            "base DbOption TOML does not exist: {}",
            base_config_path.display()
        );
    }
    if !base_config_path.is_file() {
        anyhow::bail!(
            "base DbOption path is not a file: {}",
            base_config_path.display()
        );
    }

    let replay_config_arg = strip_toml_extension_path(&request.replay_config_arg);
    let replay_config_path = config_toml_path_for_path(&replay_config_arg);
    let baseline_config_arg = request
        .baseline_config_arg
        .as_deref()
        .map(strip_toml_extension_path)
        .unwrap_or_else(|| config_arg_with_suffix(&replay_config_arg, "baseline"));
    let baseline_config_path = config_toml_path_for_path(&baseline_config_arg);
    if paths_equivalent(&replay_config_path, &base_config_path)? {
        anyhow::bail!(
            "replay config path must differ from base config path: {}",
            replay_config_path.display()
        );
    }
    if paths_equivalent(&baseline_config_path, &base_config_path)? {
        anyhow::bail!(
            "baseline config path must differ from base config path: {}",
            baseline_config_path.display()
        );
    }
    if paths_equivalent(&baseline_config_path, &replay_config_path)? {
        anyhow::bail!(
            "baseline config path must differ from replay config path: {}",
            baseline_config_path.display()
        );
    }
    let overwritten = replay_config_path.exists();
    let baseline_overwritten = baseline_config_path.exists();
    if (overwritten || baseline_overwritten) && !request.force {
        anyhow::bail!(
            "history replay config already exists: replay={} baseline={}. Pass --force to overwrite after verifying the paths.",
            replay_config_path.display(),
            baseline_config_path.display()
        );
    }

    let mut toml_value = read_base_config(&base_config_path)?;
    apply_generation_replay_overrides(
        &mut toml_value,
        &request,
        &replay_surreal_ns,
        &replay_output_root,
    )?;
    write_toml_atomic(&replay_config_path, &toml_value)?;

    let mut baseline_toml_value = read_base_config(&base_config_path)?;
    apply_baseline_parse_overrides(
        &mut baseline_toml_value,
        &request,
        &baseline_dbnums,
        &replay_surreal_ns,
        &replay_output_root,
    )?;
    write_toml_atomic(&baseline_config_path, &baseline_toml_value)?;

    let commands = build_commands(
        &base_config_arg,
        &baseline_config_arg,
        &replay_config_arg,
        &request,
        &baseline_release_id,
        &baseline_dbnums,
        &replay_parquet_dir,
    );
    let safety_checks = ModelHistoryReplaySafetyChecks {
        replay_namespace_differs_from_current: replay_surreal_ns != current_surreal_ns,
        replay_output_root_differs_from_current: !paths_equivalent(
            &replay_output_root,
            &current_output_root,
        )?,
        replay_project_output_differs_from_current: !paths_equivalent(
            &replay_project_output_dir,
            &current_project_output_dir,
        )?,
        replay_parquet_differs_from_current: !paths_equivalent(
            &replay_parquet_dir,
            &request.current_parquet_dir,
        )?,
        replay_config_differs_from_base_config: !paths_equivalent(
            &replay_config_path,
            &base_config_path,
        )?,
        generation_is_external_process: true,
        materialize_assets_in_publish_command: commands
            .publish_argv
            .iter()
            .any(|arg| arg == "--materialize-assets"),
        baseline_config_requests_save_db: baseline_config_requests_save_db(&baseline_toml_value),
        baseline_binary_supports_surreal_save,
        baseline_parse_uses_current_file_state: true,
        baseline_target_sesno_reconstruction_supported: false,
        baseline_source_must_already_match_from_sesno: true,
        baseline_source_confirmed_at_from_sesno: request.baseline_source_confirmed_at_from_sesno,
    };
    let baseline_plan_warning = format!(
        "baseline_parse currently runs full-sync against the source file's visible/current state; it does not reconstruct from_sesno={} from pdms-io history. Use this baseline step only when the source DB file already represents that baseline, or implement target-sesno hydrate before publishing it.",
        request.from_sesno
    );

    Ok(ModelHistoryReplayPrepareResponse {
        release_id: request.release_id,
        release_label: request.release_label,
        baseline_release_id: baseline_release_id.clone(),
        branch_id: request.branch_id,
        parent_release_id: request.parent_release_id,
        target_parent_release_id: baseline_release_id,
        project_name: request.project_name,
        dbnum: request.dbnum,
        baseline_dbnums,
        source_db_file: request.source_db_file,
        from_sesno: request.from_sesno,
        to_sesno: request.to_sesno,
        current_surreal_ns,
        replay_surreal_ns,
        current_output_root,
        current_project_output_dir,
        current_parquet_dir: request.current_parquet_dir,
        replay_output_root,
        replay_project_output_dir,
        replay_parquet_dir,
        base_config_arg,
        base_config_path,
        baseline_config_arg,
        baseline_config_path,
        replay_config_arg,
        replay_config_path,
        written: true,
        overwritten: overwritten || baseline_overwritten,
        baseline_plan_warning,
        commands,
        safety_checks,
    })
}

fn baseline_config_requests_save_db(toml_value: &TomlValue) -> bool {
    toml_value
        .as_table()
        .and_then(|table| table.get("save_db"))
        .and_then(TomlValue::as_bool)
        .unwrap_or(false)
}

fn validate_prepare_request(request: &ModelHistoryReplayPrepareRequest) -> anyhow::Result<()> {
    validate_release_id_for_path(&request.release_id)?;
    if request.from_sesno >= request.to_sesno {
        anyhow::bail!(
            "invalid sesno range for historical replay: from_sesno={} must be less than to_sesno={}",
            request.from_sesno,
            request.to_sesno
        );
    }
    if !request.source_db_file.exists() {
        anyhow::bail!(
            "source DB file for historical replay does not exist: {}",
            request.source_db_file.display()
        );
    }
    if !request.source_db_file.is_file() {
        anyhow::bail!(
            "source DB path for historical replay is not a file: {}",
            request.source_db_file.display()
        );
    }
    Ok(())
}

fn read_base_config(path: &Path) -> anyhow::Result<TomlValue> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read base DbOption TOML failed: {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("parse base DbOption TOML failed: {}", path.display()))
}

fn apply_generation_replay_overrides(
    toml_value: &mut TomlValue,
    request: &ModelHistoryReplayPrepareRequest,
    replay_surreal_ns: &str,
    replay_output_root: &Path,
) -> anyhow::Result<()> {
    let Some(table) = toml_value.as_table_mut() else {
        anyhow::bail!("base DbOption TOML root must be a table");
    };

    table.insert(
        "project_name".to_string(),
        TomlValue::String(request.project_name.clone()),
    );
    table.insert(
        "surreal_ns".to_string(),
        TomlValue::String(replay_surreal_ns.to_string()),
    );
    table.insert(
        "output_root".to_string(),
        TomlValue::String(path_to_config_string(replay_output_root)),
    );
    table.insert(
        "manual_db_nums".to_string(),
        TomlValue::Array(vec![TomlValue::Integer(i64::from(request.dbnum))]),
    );
    table.insert(
        "included_projects".to_string(),
        TomlValue::Array(vec![TomlValue::String(request.project_name.clone())]),
    );
    table.insert(
        "export_parquet_after_gen".to_string(),
        TomlValue::Boolean(true),
    );
    table.insert("total_sync".to_string(), TomlValue::Boolean(false));
    table.insert("incr_sync".to_string(), TomlValue::Boolean(false));
    table.insert("sync_history".to_string(), TomlValue::Boolean(false));
    table.insert("only_sync_sys".to_string(), TomlValue::Boolean(false));
    table.insert("gen_tree_only".to_string(), TomlValue::Boolean(false));
    table.insert("save_db".to_string(), TomlValue::Boolean(true));
    table.insert("gen_model".to_string(), TomlValue::Boolean(true));
    table.insert("gen_mesh".to_string(), TomlValue::Boolean(true));
    table.insert(
        "index_tree_debug_limit_per_target_type".to_string(),
        TomlValue::Integer(0),
    );

    for key in DERIVED_PATH_KEYS {
        table.remove(*key);
    }

    Ok(())
}

fn apply_baseline_parse_overrides(
    toml_value: &mut TomlValue,
    request: &ModelHistoryReplayPrepareRequest,
    baseline_dbnums: &[u32],
    replay_surreal_ns: &str,
    replay_output_root: &Path,
) -> anyhow::Result<()> {
    let Some(table) = toml_value.as_table_mut() else {
        anyhow::bail!("base DbOption TOML root must be a table");
    };

    table.insert(
        "project_name".to_string(),
        TomlValue::String(request.project_name.clone()),
    );
    table.insert(
        "included_projects".to_string(),
        TomlValue::Array(vec![TomlValue::String(request.project_name.clone())]),
    );
    table.insert(
        "surreal_ns".to_string(),
        TomlValue::String(replay_surreal_ns.to_string()),
    );
    table.insert(
        "output_root".to_string(),
        TomlValue::String(path_to_config_string(replay_output_root)),
    );
    table.insert(
        "manual_db_nums".to_string(),
        TomlValue::Array(
            baseline_dbnums
                .iter()
                .map(|dbnum| TomlValue::Integer(i64::from(*dbnum)))
                .collect(),
        ),
    );
    table.remove("included_db_files");
    table.insert("total_sync".to_string(), TomlValue::Boolean(true));
    table.insert("incr_sync".to_string(), TomlValue::Boolean(false));
    table.insert("sync_history".to_string(), TomlValue::Boolean(false));
    table.insert("only_sync_sys".to_string(), TomlValue::Boolean(false));
    table.insert("gen_tree_only".to_string(), TomlValue::Boolean(false));
    table.insert("save_db".to_string(), TomlValue::Boolean(true));
    table.insert("gen_model".to_string(), TomlValue::Boolean(false));
    table.insert("gen_mesh".to_string(), TomlValue::Boolean(false));
    table.insert(
        "export_parquet_after_gen".to_string(),
        TomlValue::Boolean(false),
    );
    table.insert(
        "index_tree_debug_limit_per_target_type".to_string(),
        TomlValue::Integer(0),
    );

    for key in DERIVED_PATH_KEYS {
        table.remove(*key);
    }

    Ok(())
}

fn write_toml_atomic(path: &Path, toml_value: &TomlValue) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("replay config path has no parent: {}", path.display()))?;
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create replay config dir failed: {}", parent.display()))?;
    }

    let content = toml::to_string_pretty(toml_value).context("serialize replay DbOption TOML")?;
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, content)
        .with_context(|| format!("write temp replay config failed: {}", tmp_path.display()))?;
    replace_file_with_backup(path, &tmp_path)?;
    Ok(())
}

fn replace_file_with_backup(path: &Path, tmp_path: &Path) -> anyhow::Result<()> {
    let backup_path = path.with_extension("toml.bak");
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .with_context(|| format!("remove stale backup failed: {}", backup_path.display()))?;
    }

    let had_existing = path.exists();
    if had_existing {
        fs::rename(path, &backup_path).with_context(|| {
            format!(
                "backup old replay config failed: {} -> {}",
                path.display(),
                backup_path.display()
            )
        })?;
    }

    match fs::rename(tmp_path, path) {
        Ok(()) => {
            if had_existing && backup_path.exists() {
                fs::remove_file(&backup_path).with_context(|| {
                    format!(
                        "remove replay config backup failed: {}",
                        backup_path.display()
                    )
                })?;
            }
            Ok(())
        }
        Err(err) => {
            if had_existing && backup_path.exists() {
                let _ = fs::rename(&backup_path, path);
            }
            Err(anyhow::anyhow!(
                "atomic rename replay config failed: {} -> {}: {}",
                tmp_path.display(),
                path.display(),
                err
            ))
        }
    }
}

fn build_commands(
    base_config_arg: &str,
    baseline_config_arg: &Path,
    replay_config_arg: &Path,
    request: &ModelHistoryReplayPrepareRequest,
    baseline_release_id: &str,
    baseline_dbnums: &[u32],
    replay_parquet_dir: &Path,
) -> ModelHistoryReplayCommands {
    let baseline_config_arg = path_to_command_string(baseline_config_arg);
    let replay_config_arg = path_to_command_string(replay_config_arg);
    let source_db_file = path_to_command_string(&request.source_db_file);
    let replay_parquet_dir = path_to_command_string(replay_parquet_dir);

    let baseline_parse_argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        baseline_config_arg,
    ];

    let baseline_generate_argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        replay_config_arg.clone(),
    ];

    let baseline_metadata = serde_json::json!({
        "history_baseline": {
            "source": "model-version prepare-history-replay",
            "requested_baseline_sesno": request.from_sesno,
            "target_release_id": request.release_id,
            "source_db_file": &request.source_db_file,
            "dbnum": request.dbnum,
            "baseline_dbnums": baseline_dbnums,
            "staged_parquet_dir": &replay_parquet_dir,
            "note": "baseline_parse currently uses the source file's visible/current full-sync state; publish this baseline only when that state matches requested_baseline_sesno"
        }
    })
    .to_string();
    let mut baseline_register_argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        base_config_arg.to_string(),
        "model-version".to_string(),
        "register".to_string(),
        "--release-id".to_string(),
        baseline_release_id.to_string(),
        "--branch-id".to_string(),
        request.branch_id.clone(),
        "--derivation-type".to_string(),
        "historical-baseline".to_string(),
        "--dbnum".to_string(),
        request.dbnum.to_string(),
        "--parquet-dir".to_string(),
        replay_parquet_dir.clone(),
        "--metadata-json".to_string(),
        baseline_metadata,
        "--json".to_string(),
    ];
    if let Some(parent) = request.parent_release_id.as_deref() {
        baseline_register_argv.push("--parent-release-id".to_string());
        baseline_register_argv.push(parent.to_string());
    }

    let generate_argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        replay_config_arg,
        "incremental-sesno".to_string(),
        "--file".to_string(),
        source_db_file.clone(),
        "--from-sesno".to_string(),
        request.from_sesno.to_string(),
        "--to-sesno".to_string(),
        request.to_sesno.to_string(),
        "--generate-model".to_string(),
        "--json".to_string(),
    ];

    let mut publish_argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        base_config_arg.to_string(),
        "model-version".to_string(),
        "publish-history".to_string(),
        "--release-id".to_string(),
        request.release_id.clone(),
        "--branch-id".to_string(),
        request.branch_id.clone(),
        "--dbnum".to_string(),
        request.dbnum.to_string(),
        "--source-db-file".to_string(),
        source_db_file,
        "--from-sesno".to_string(),
        request.from_sesno.to_string(),
        "--to-sesno".to_string(),
        request.to_sesno.to_string(),
        "--parquet-dir".to_string(),
        replay_parquet_dir,
        "--materialize-assets".to_string(),
        "--index-units".to_string(),
    ];
    if let Some(label) = request.release_label.as_deref() {
        publish_argv.push("--release-label".to_string());
        publish_argv.push(label.to_string());
    }
    publish_argv.push("--parent-release-id".to_string());
    publish_argv.push(baseline_release_id.to_string());
    publish_argv.push("--json".to_string());

    ModelHistoryReplayCommands {
        baseline_parse: command_to_shell_string(&baseline_parse_argv),
        baseline_generate: command_to_shell_string(&baseline_generate_argv),
        baseline_register: command_to_shell_string(&baseline_register_argv),
        generate: command_to_shell_string(&generate_argv),
        publish: command_to_shell_string(&publish_argv),
        baseline_parse_argv,
        baseline_generate_argv,
        baseline_register_argv,
        generate_argv,
        publish_argv,
    }
}

fn project_output_dir_for(output_root: &Path, project_name: &str) -> PathBuf {
    output_root.join(project_name)
}

fn normalize_baseline_dbnums(target_dbnum: u32, requested: &[u32]) -> Vec<u32> {
    let mut dbnums = requested
        .iter()
        .copied()
        .filter(|dbnum| *dbnum > 0)
        .collect::<Vec<_>>();
    if !dbnums.contains(&target_dbnum) {
        dbnums.push(target_dbnum);
    }
    dbnums.sort_unstable();
    dbnums.dedup();
    dbnums
}

fn config_toml_path(config_arg: &str) -> PathBuf {
    config_toml_path_for_path(Path::new(config_arg))
}

fn config_toml_path_for_path(config_arg: &Path) -> PathBuf {
    if config_arg.extension().and_then(|value| value.to_str()) == Some("toml") {
        config_arg.to_path_buf()
    } else {
        let mut value = config_arg.as_os_str().to_os_string();
        value.push(".toml");
        PathBuf::from(value)
    }
}

fn strip_toml_extension(value: &str) -> String {
    strip_toml_extension_path(Path::new(value))
        .to_string_lossy()
        .to_string()
}

fn strip_toml_extension_path(value: &Path) -> PathBuf {
    if value.extension().and_then(|ext| ext.to_str()) == Some("toml") {
        value.with_extension("")
    } else {
        value.to_path_buf()
    }
}

fn config_arg_with_suffix(value: &Path, suffix: &str) -> PathBuf {
    let mut out = value.to_path_buf();
    let file_name = out
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("history-replay");
    out.set_file_name(format!("{file_name}-{suffix}"));
    out
}

fn sanitize_namespace_fragment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_' | '.') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "release".to_string()
    } else {
        out.to_string()
    }
}

fn path_to_config_string(path: &Path) -> String {
    path_to_command_string(path).replace('\\', "/")
}

fn path_to_command_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn command_to_shell_string(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if shell_safe_arg(arg) {
                arg.to_string()
            } else {
                shell_quote(arg)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_safe_arg(arg: &str) -> bool {
    !arg.is_empty()
        && arg.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '\\' | ':' | '=')
        })
}

fn paths_equivalent(left: &Path, right: &Path) -> anyhow::Result<bool> {
    let left = absolute_lexical(left)?;
    let right = absolute_lexical(right)?;
    Ok(left
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy()))
}

fn absolute_lexical(path: &Path) -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    Ok(normalize_lexical(&absolute))
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
