use crate::options::DbOptionExt;
use crate::version_management::release_package::validate_release_id_for_path;
use crate::version_management::types::{
    ModelPhysicalBaselineSnapshotCommands, ModelPhysicalBaselineSnapshotRequest,
    ModelPhysicalBaselineSnapshotResponse, ModelPhysicalBaselineSnapshotSafetyChecks,
    ModelPhysicalBaselineStateManifest,
};
use anyhow::Context;
use parse_pdms_db::parse::parse_file_basic_info;
use pdms_io::PdmsIO;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use toml::Value as TomlValue;

const DERIVED_PATH_KEYS: &[&str] = &[
    "model_cache_dir",
    "transform_parquet_dir",
    "transform_ducklake_metadata",
    "transform_ducklake_data_path",
];

pub fn prepare_physical_baseline_snapshot(
    db_option_ext: &DbOptionExt,
    request: ModelPhysicalBaselineSnapshotRequest,
) -> anyhow::Result<ModelPhysicalBaselineSnapshotResponse> {
    validate_release_id_for_path(&request.snapshot_id)?;
    validate_source_db_file(&request)?;

    let source_project_dir = db_option_ext
        .inner
        .get_project_path(&request.project_name)
        .ok_or_else(|| anyhow::anyhow!("project path not found: {}", request.project_name))?;
    if !source_project_dir.is_dir() {
        anyhow::bail!(
            "source project directory is missing or not a directory: {}",
            source_project_dir.display()
        );
    }

    let source_db_dir = find_primary_db_dir(&source_project_dir)?;
    let active_db_file = find_db_file_by_dbnum(&source_db_dir, request.dbnum)?;
    let source_db_info = read_db_basic_info(&request.source_db_file)?;
    let source_db_latest_sesno =
        read_db_latest_sesno(&request.project_name, &request.source_db_file)?;
    if source_db_info.dbnum != request.dbnum {
        anyhow::bail!(
            "source DB file dbnum mismatch: expected {}, got {} ({})",
            request.dbnum,
            source_db_info.dbnum,
            request.source_db_file.display()
        );
    }

    let current_output_root = db_option_ext.get_output_root();
    let current_project_output_dir = current_output_root.join(&request.project_name);
    let snapshot_root = request.snapshot_root.clone().unwrap_or_else(|| {
        current_project_output_dir
            .join("model_versions")
            .join("physical_baselines")
            .join(&request.snapshot_id)
    });
    let snapshot_project_parent = snapshot_root.join("project_path");
    let snapshot_project_dir = snapshot_project_parent.join(&request.project_name);
    let snapshot_db_dir = snapshot_project_dir.join(
        source_db_dir
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("source db dir has no file name"))?,
    );
    let replacement_db_file = snapshot_db_dir.join(
        active_db_file
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("active db file has no file name"))?,
    );
    let output_root = request
        .output_root
        .clone()
        .unwrap_or_else(|| snapshot_root.join("output"));
    let surreal_ns = request
        .surreal_ns
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "{}_baseline_{}",
                db_option_ext.inner.surreal_ns,
                sanitize_namespace_fragment(&request.snapshot_id)
            )
        });
    if surreal_ns.eq_ignore_ascii_case(&db_option_ext.inner.surreal_ns) {
        anyhow::bail!(
            "snapshot surreal_ns must differ from current surreal_ns: {}",
            surreal_ns
        );
    }

    if paths_equivalent(&snapshot_project_dir, &source_project_dir)? {
        anyhow::bail!(
            "snapshot project directory must differ from source project directory: {}",
            snapshot_project_dir.display()
        );
    }
    if paths_equivalent(&output_root, &current_output_root)? {
        anyhow::bail!(
            "snapshot output_root must differ from current output_root: {}",
            output_root.display()
        );
    }

    let base_config_arg = strip_toml_extension(&request.base_config_arg);
    let base_config_path = config_toml_path(Path::new(&base_config_arg));
    if !base_config_path.is_file() {
        anyhow::bail!(
            "base DbOption TOML does not exist or is not a file: {}",
            base_config_path.display()
        );
    }
    let config_arg = request
        .config_arg
        .as_deref()
        .map(strip_toml_extension_path)
        .unwrap_or_else(|| snapshot_root.join("DbOption-physical-baseline"));
    let config_path = config_toml_path(&config_arg);
    if paths_equivalent(&config_path, &base_config_path)? {
        anyhow::bail!(
            "snapshot config path must differ from base config path: {}",
            config_path.display()
        );
    }

    let snapshot_existed = snapshot_project_dir.exists();
    let config_existed = config_path.exists();
    if (snapshot_existed || config_existed) && !request.force {
        anyhow::bail!(
            "physical baseline snapshot already exists: project={} config={}. Pass --force to overwrite files inside the snapshot directory.",
            snapshot_project_dir.display(),
            config_path.display()
        );
    }

    fs::create_dir_all(&snapshot_db_dir).with_context(|| {
        format!(
            "create snapshot db directory failed: {}",
            snapshot_db_dir.display()
        )
    })?;
    let copy_stats = materialize_db_dir(
        &source_db_dir,
        &snapshot_db_dir,
        &active_db_file,
        &request.source_db_file,
        request.copy_files,
        request.force,
    )?;

    let baseline_dbnums = normalize_baseline_dbnums(request.dbnum, &request.baseline_dbnums);
    let mut toml_value = read_base_config(&base_config_path)?;
    apply_snapshot_config_overrides(
        &mut toml_value,
        &request.project_name,
        &snapshot_project_parent,
        &output_root,
        &surreal_ns,
        &baseline_dbnums,
    )?;
    write_toml_atomic(&config_path, &toml_value)?;
    let copy_mode = if request.copy_files {
        "copy".to_string()
    } else {
        "hardlink_with_copy_fallback".to_string()
    };
    let safety_checks = ModelPhysicalBaselineSnapshotSafetyChecks {
        source_db_file_exists: true,
        source_db_file_is_file: true,
        source_db_file_matches_dbnum: true,
        snapshot_project_differs_from_source: true,
        snapshot_output_differs_from_current: true,
        config_differs_from_base_config: true,
        original_project_not_modified: true,
    };
    let baseline_state_manifest = ModelPhysicalBaselineStateManifest {
        manifest_version: "physical_baseline_state_manifest:v1".to_string(),
        snapshot_id: request.snapshot_id.clone(),
        project_name: request.project_name.clone(),
        dbnum: request.dbnum,
        baseline_dbnums: baseline_dbnums.clone(),
        source_db_file: request.source_db_file.clone(),
        source_db_sha256: crate::version_management::hashing::sha256_file(&request.source_db_file)
            .with_context(|| {
                format!(
                    "hash physical baseline source DB failed: {}",
                    request.source_db_file.display()
                )
            })?,
        replacement_db_file: replacement_db_file.clone(),
        replacement_db_sha256: crate::version_management::hashing::sha256_file(
            &replacement_db_file,
        )
        .with_context(|| {
            format!(
                "hash physical baseline replacement DB failed: {}",
                replacement_db_file.display()
            )
        })?,
        source_db_type: source_db_info.db_type.clone(),
        source_db_session_page: source_db_info.ses_pgno,
        source_db_latest_sesno,
        snapshot_root: snapshot_root.clone(),
        snapshot_project_dir: snapshot_project_dir.clone(),
        snapshot_db_dir: snapshot_db_dir.clone(),
        output_root: output_root.clone(),
        config_path: config_path.clone(),
        surreal_ns: surreal_ns.clone(),
        file_count: copy_stats.file_count,
        hardlinked_count: copy_stats.hardlinked_count,
        copied_count: copy_stats.copied_count,
        copy_mode: copy_mode.clone(),
        safety_checks: safety_checks.clone(),
    };
    let baseline_state_manifest_path = snapshot_root.join("baseline_state_manifest.json");
    write_json_atomic(&baseline_state_manifest_path, &baseline_state_manifest)?;
    let baseline_state_manifest_hash =
        crate::version_management::hashing::sha256_file(&baseline_state_manifest_path)
            .with_context(|| {
                format!(
                    "hash baseline state manifest failed: {}",
                    baseline_state_manifest_path.display()
                )
            })?;

    let parse_argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        path_to_command_string(&config_arg),
    ];
    let generate_full_model_argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        path_to_command_string(&config_arg),
        "--regen-model".to_string(),
        "--dbnum".to_string(),
        request.dbnum.to_string(),
        "--export-parquet-after-gen".to_string(),
    ];
    let prepare_history_replay_hint_argv = vec![
        "aios-database".to_string(),
        "-c".to_string(),
        path_to_command_string(&config_arg),
        "model-version".to_string(),
        "prepare-history-replay".to_string(),
        "--base-config".to_string(),
        path_to_command_string(&config_arg),
        "--dbnum".to_string(),
        request.dbnum.to_string(),
        "--source-db-file".to_string(),
        path_to_command_string(&replacement_db_file),
        "--from-sesno".to_string(),
        "<baseline-sesno>".to_string(),
        "--to-sesno".to_string(),
        "<target-sesno>".to_string(),
        "--baseline-source-confirmed-at-from-sesno".to_string(),
        "--json".to_string(),
    ];
    let commands = ModelPhysicalBaselineSnapshotCommands {
        parse: command_to_shell_string(&parse_argv),
        parse_argv,
        generate_full_model: command_to_shell_string(&generate_full_model_argv),
        generate_full_model_argv,
        prepare_history_replay_hint: command_to_shell_string(&prepare_history_replay_hint_argv),
        prepare_history_replay_hint_argv,
    };

    Ok(ModelPhysicalBaselineSnapshotResponse {
        project_name: request.project_name,
        snapshot_id: request.snapshot_id,
        dbnum: request.dbnum,
        baseline_dbnums,
        source_db_file: request.source_db_file,
        source_project_dir,
        source_db_dir,
        active_db_file,
        snapshot_root,
        snapshot_project_parent,
        snapshot_project_dir,
        snapshot_db_dir,
        replacement_db_file,
        source_db_latest_sesno,
        base_config_arg,
        base_config_path,
        config_arg,
        config_path,
        output_root,
        surreal_ns,
        file_count: copy_stats.file_count,
        hardlinked_count: copy_stats.hardlinked_count,
        copied_count: copy_stats.copied_count,
        replaced_target: true,
        overwritten: snapshot_existed || config_existed,
        copy_mode,
        written: true,
        baseline_state_manifest_path,
        baseline_state_manifest_hash,
        commands,
        safety_checks,
    })
}

fn validate_source_db_file(request: &ModelPhysicalBaselineSnapshotRequest) -> anyhow::Result<()> {
    if !request.source_db_file.exists() {
        anyhow::bail!(
            "source DB file for physical baseline does not exist: {}",
            request.source_db_file.display()
        );
    }
    if !request.source_db_file.is_file() {
        anyhow::bail!(
            "source DB path for physical baseline is not a file: {}",
            request.source_db_file.display()
        );
    }
    Ok(())
}

fn find_primary_db_dir(project_dir: &Path) -> anyhow::Result<PathBuf> {
    let mut candidates = fs::read_dir(project_dir)
        .with_context(|| format!("read project dir failed: {}", project_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.ends_with("000"))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "no primary *000 database directory found under {}",
            project_dir.display()
        )
    })
}

fn find_db_file_by_dbnum(db_dir: &Path, dbnum: u32) -> anyhow::Result<PathBuf> {
    let mut matches = Vec::new();
    for entry in
        fs::read_dir(db_dir).with_context(|| format!("read db dir failed: {}", db_dir.display()))?
    {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Ok(info) = read_db_basic_info(&path) else {
            continue;
        };
        if info.dbnum == dbnum {
            matches.push(path);
        }
    }
    matches.sort();
    matches
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("dbnum {} not found under {}", dbnum, db_dir.display()))
}

struct CopyStats {
    file_count: usize,
    hardlinked_count: usize,
    copied_count: usize,
}

fn materialize_db_dir(
    source_db_dir: &Path,
    snapshot_db_dir: &Path,
    active_db_file: &Path,
    replacement_source: &Path,
    copy_files: bool,
    force: bool,
) -> anyhow::Result<CopyStats> {
    let mut stats = CopyStats {
        file_count: 0,
        hardlinked_count: 0,
        copied_count: 0,
    };
    for entry in fs::read_dir(source_db_dir)
        .with_context(|| format!("read source db dir failed: {}", source_db_dir.display()))?
    {
        let source = entry?.path();
        if !source.is_file() {
            continue;
        }
        let destination = snapshot_db_dir.join(
            source
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("source file has no name: {}", source.display()))?,
        );
        let source_to_materialize = if paths_equivalent(&source, active_db_file)? {
            replacement_source
        } else {
            &source
        };
        let hardlinked =
            materialize_one_file(source_to_materialize, &destination, copy_files, force)?;
        stats.file_count += 1;
        if hardlinked {
            stats.hardlinked_count += 1;
        } else {
            stats.copied_count += 1;
        }
    }
    Ok(stats)
}

fn materialize_one_file(
    source: &Path,
    destination: &Path,
    copy_files: bool,
    force: bool,
) -> anyhow::Result<bool> {
    if destination.exists() {
        if !force {
            anyhow::bail!(
                "snapshot file already exists: {}. Pass --force to overwrite.",
                destination.display()
            );
        }
        fs::remove_file(destination).with_context(|| {
            format!(
                "remove existing snapshot file failed: {}",
                destination.display()
            )
        })?;
    }
    if copy_files {
        fs::copy(source, destination).with_context(|| {
            format!(
                "copy snapshot file failed: {} -> {}",
                source.display(),
                destination.display()
            )
        })?;
        return Ok(false);
    }
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(true),
        Err(_) => {
            fs::copy(source, destination).with_context(|| {
                format!(
                    "copy fallback after hardlink failed: {} -> {}",
                    source.display(),
                    destination.display()
                )
            })?;
            Ok(false)
        }
    }
}

fn read_db_basic_info(path: &Path) -> anyhow::Result<parse_pdms_db::parse::DbBasicInfo> {
    let mut file =
        fs::File::open(path).with_context(|| format!("open DB file failed: {}", path.display()))?;
    let mut buf = [0u8; 60];
    file.read_exact(&mut buf)
        .with_context(|| format!("read DB header failed: {}", path.display()))?;
    Ok(parse_file_basic_info(&buf))
}

fn read_db_latest_sesno(project_name: &str, path: &Path) -> anyhow::Result<u32> {
    let mut io = PdmsIO::new(project_name, path, false);
    io.open()
        .with_context(|| format!("open source DB for latest sesno failed: {}", path.display()))?;
    io.get_latest_sesno()
        .with_context(|| format!("read source DB latest sesno failed: {}", path.display()))
}

fn read_base_config(path: &Path) -> anyhow::Result<TomlValue> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read base DbOption TOML failed: {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("parse base DbOption TOML failed: {}", path.display()))
}

fn apply_snapshot_config_overrides(
    toml_value: &mut TomlValue,
    project_name: &str,
    snapshot_project_parent: &Path,
    output_root: &Path,
    surreal_ns: &str,
    baseline_dbnums: &[u32],
) -> anyhow::Result<()> {
    let Some(table) = toml_value.as_table_mut() else {
        anyhow::bail!("base DbOption TOML root must be a table");
    };
    table.insert(
        "project_path".to_string(),
        TomlValue::String(path_to_config_string(snapshot_project_parent)),
    );
    table.insert(
        "project_name".to_string(),
        TomlValue::String(project_name.to_string()),
    );
    table.insert(
        "included_projects".to_string(),
        TomlValue::Array(vec![TomlValue::String(project_name.to_string())]),
    );
    table.insert(
        "surreal_ns".to_string(),
        TomlValue::String(surreal_ns.to_string()),
    );
    table.insert(
        "output_root".to_string(),
        TomlValue::String(path_to_config_string(output_root)),
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
        .ok_or_else(|| anyhow::anyhow!("config path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create config dir failed: {}", parent.display()))?;
    let content =
        toml::to_string_pretty(toml_value).context("serialize physical baseline DbOption TOML")?;
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, content)
        .with_context(|| format!("write temp config failed: {}", tmp_path.display()))?;
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("remove old config failed: {}", path.display()))?;
    }
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "atomic rename config failed: {} -> {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "baseline state manifest path has no parent: {}",
            path.display()
        )
    })?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "create baseline state manifest dir failed: {}",
            parent.display()
        )
    })?;
    let content = serde_json::to_vec_pretty(value).context("serialize baseline state manifest")?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content).with_context(|| {
        format!(
            "write temp baseline state manifest failed: {}",
            tmp_path.display()
        )
    })?;
    if path.exists() {
        fs::remove_file(path).with_context(|| {
            format!(
                "remove old baseline state manifest failed: {}",
                path.display()
            )
        })?;
    }
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "atomic rename baseline state manifest failed: {} -> {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(())
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
        "baseline".to_string()
    } else {
        out.to_string()
    }
}

fn config_toml_path(config_arg: &Path) -> PathBuf {
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
