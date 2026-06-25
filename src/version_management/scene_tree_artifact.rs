use crate::version_management::hashing::sha256_file;
use crate::version_management::types::{
    ModelSceneTreeArtifactRestoreRequest, ModelSceneTreeArtifactRestoreResponse,
};
use anyhow::Context;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn restore_scene_tree_artifact(
    request: ModelSceneTreeArtifactRestoreRequest,
) -> anyhow::Result<ModelSceneTreeArtifactRestoreResponse> {
    if request.project_name.trim().is_empty() {
        anyhow::bail!("project_name is required");
    }
    if request.dbnum == 0 {
        anyhow::bail!("dbnum must be non-zero");
    }

    let source_tree_file = request
        .source_scene_tree_dir
        .join(format!("{}.tree", request.dbnum));
    let target_tree_file = request
        .target_scene_tree_dir
        .join(format!("{}.tree", request.dbnum));
    let source_db_meta_info_file = request.source_scene_tree_dir.join("db_meta_info.json");
    let target_db_meta_info_file = request.target_scene_tree_dir.join("db_meta_info.json");

    let source_tree_meta = fs::metadata(&source_tree_file).with_context(|| {
        format!(
            "source scene_tree file is missing or unreadable: {}",
            source_tree_file.display()
        )
    })?;
    if !source_tree_meta.is_file() {
        anyhow::bail!(
            "source scene_tree path is not a file: {}",
            source_tree_file.display()
        );
    }
    let source_tree_sha256 = sha256_file(&source_tree_file)?;
    let target_tree_sha256_before = if target_tree_file.is_file() {
        Some(sha256_file(&target_tree_file)?)
    } else {
        None
    };

    let source_meta = read_json_file(&source_db_meta_info_file)?;
    let mut target_meta = if target_db_meta_info_file.is_file() {
        read_json_file(&target_db_meta_info_file)?
    } else {
        serde_json::json!({
            "db_files": {},
            "ref0_to_dbnum": {},
            "updated_at": null,
            "version": source_meta.get("version").cloned().unwrap_or(Value::Null),
        })
    };

    let dbnum_key = request.dbnum.to_string();
    let source_db_file = source_meta
        .get("db_files")
        .and_then(Value::as_object)
        .and_then(|db_files| db_files.get(&dbnum_key))
        .cloned()
        .with_context(|| {
            format!(
                "source db_meta_info.json does not contain db_files.{}",
                dbnum_key
            )
        })?;
    let source_ref0s = ref0s_from_db_file(&source_db_file);
    if source_ref0s.is_empty() {
        anyhow::bail!(
            "source db_meta_info.json db_files.{} has no ref0s; refusing to restore ambiguous scene_tree evidence",
            dbnum_key
        );
    }
    let source_latest_sesno = source_db_file.get("latest_sesno").and_then(Value::as_u64);

    ensure_meta_objects(&mut target_meta)?;
    let target_db_file_before = target_meta
        .get("db_files")
        .and_then(Value::as_object)
        .and_then(|db_files| db_files.get(&dbnum_key))
        .cloned();
    let target_ref0s_before = target_db_file_before
        .as_ref()
        .map(ref0s_from_db_file)
        .unwrap_or_default();
    let target_latest_sesno_before = target_db_file_before
        .as_ref()
        .and_then(|value| value.get("latest_sesno"))
        .and_then(Value::as_u64);

    let mut warnings = Vec::new();
    if let (Some(source), Some(target)) = (source_latest_sesno, target_latest_sesno_before)
        && source != target
    {
        warnings.push(format!(
            "source latest_sesno {} differs from target latest_sesno {}; restored tree must be treated as evidence for the source state",
            source, target
        ));
    }

    validate_ref0_mapping_conflicts(&target_meta, &source_ref0s, request.dbnum)?;

    let target_ref0s_after = union_ref0s(&target_ref0s_before, &source_ref0s);
    let added_ref0s = target_ref0s_after
        .iter()
        .copied()
        .filter(|ref0| !target_ref0s_before.contains(ref0))
        .collect::<Vec<_>>();
    merge_db_file_entry(
        &mut target_meta,
        &dbnum_key,
        source_db_file,
        &target_ref0s_after,
    )?;
    merge_ref0_to_dbnum(&mut target_meta, &source_ref0s, request.dbnum)?;

    let db_meta_would_write = !target_db_meta_info_file.is_file()
        || !added_ref0s.is_empty()
        || target_db_file_before.is_none();
    let tree_would_copy = match &target_tree_sha256_before {
        Some(existing_hash) if existing_hash.eq_ignore_ascii_case(&source_tree_sha256) => false,
        Some(_) if !request.overwrite_tree => {
            anyhow::bail!(
                "target scene_tree file already exists with a different hash; pass --overwrite-tree to replace it: {}",
                target_tree_file.display()
            );
        }
        Some(_) => true,
        None => true,
    };

    let mut tree_copied = false;
    let mut db_meta_written = false;
    if !request.dry_run {
        fs::create_dir_all(&request.target_scene_tree_dir).with_context(|| {
            format!(
                "create target scene_tree directory failed: {}",
                request.target_scene_tree_dir.display()
            )
        })?;
        if tree_would_copy {
            copy_file_atomic(&source_tree_file, &target_tree_file)?;
            tree_copied = true;
        }
        if db_meta_would_write {
            write_json_atomic(&target_db_meta_info_file, &target_meta)?;
            db_meta_written = true;
        }
    }

    let target_tree_sha256_after = if request.dry_run {
        target_tree_sha256_before.clone()
    } else if target_tree_file.is_file() {
        Some(sha256_file(&target_tree_file)?)
    } else {
        None
    };
    let target_latest_sesno_after = target_meta
        .get("db_files")
        .and_then(Value::as_object)
        .and_then(|db_files| db_files.get(&dbnum_key))
        .and_then(|value| value.get("latest_sesno"))
        .and_then(Value::as_u64);
    let recommended_action = if request.dry_run {
        "Dry run only. Re-run without --dry-run to restore the scene_tree artifact.".to_string()
    } else if tree_copied || db_meta_written {
        "Scene tree artifact restored; rerun release validation with --require-scene-tree."
            .to_string()
    } else {
        "Scene tree artifact was already present and db_meta_info needed no changes.".to_string()
    };

    Ok(ModelSceneTreeArtifactRestoreResponse {
        project_name: request.project_name,
        dbnum: request.dbnum,
        source_scene_tree_dir: request.source_scene_tree_dir,
        target_scene_tree_dir: request.target_scene_tree_dir,
        source_tree_file,
        target_tree_file,
        source_db_meta_info_file,
        target_db_meta_info_file,
        dry_run: request.dry_run,
        overwrite_tree: request.overwrite_tree,
        source_tree_bytes: source_tree_meta.len(),
        source_tree_sha256,
        target_tree_sha256_before,
        target_tree_sha256_after,
        tree_would_copy,
        tree_copied,
        db_meta_would_write,
        db_meta_written,
        source_latest_sesno,
        target_latest_sesno_before,
        target_latest_sesno_after,
        source_ref0s,
        target_ref0s_before,
        target_ref0s_after,
        added_ref0s,
        warnings,
        recommended_action,
    })
}

fn read_json_file(path: &Path) -> anyhow::Result<Value> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read JSON file failed: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("parse JSON file failed: {}", path.display()))
}

fn ensure_meta_objects(meta: &mut Value) -> anyhow::Result<()> {
    if !meta.is_object() {
        anyhow::bail!("db_meta_info root must be a JSON object");
    }
    let root = meta.as_object_mut().expect("checked object");
    if !root.contains_key("db_files") {
        root.insert("db_files".to_string(), Value::Object(Map::new()));
    }
    if !root.contains_key("ref0_to_dbnum") {
        root.insert("ref0_to_dbnum".to_string(), Value::Object(Map::new()));
    }
    if !root.get("db_files").map(Value::is_object).unwrap_or(false) {
        anyhow::bail!("db_meta_info.db_files must be a JSON object");
    }
    if !root
        .get("ref0_to_dbnum")
        .map(Value::is_object)
        .unwrap_or(false)
    {
        anyhow::bail!("db_meta_info.ref0_to_dbnum must be a JSON object");
    }
    Ok(())
}

fn ref0s_from_db_file(db_file: &Value) -> Vec<u64> {
    let mut values = db_file
        .get("ref0s")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
        .unwrap_or_default();
    values.sort_unstable();
    values.dedup();
    values
}

fn union_ref0s(left: &[u64], right: &[u64]) -> Vec<u64> {
    let mut set = BTreeSet::new();
    set.extend(left.iter().copied());
    set.extend(right.iter().copied());
    set.into_iter().collect()
}

fn validate_ref0_mapping_conflicts(
    target_meta: &Value,
    source_ref0s: &[u64],
    dbnum: u32,
) -> anyhow::Result<()> {
    let Some(map) = target_meta.get("ref0_to_dbnum").and_then(Value::as_object) else {
        return Ok(());
    };
    for ref0 in source_ref0s {
        let key = ref0.to_string();
        if let Some(existing) = map.get(&key).and_then(Value::as_u64)
            && existing != dbnum as u64
        {
            anyhow::bail!(
                "target db_meta_info ref0_to_dbnum conflict: ref0 {} maps to {}, source maps to {}",
                ref0,
                existing,
                dbnum
            );
        }
    }
    Ok(())
}

fn merge_db_file_entry(
    target_meta: &mut Value,
    dbnum_key: &str,
    source_db_file: Value,
    target_ref0s_after: &[u64],
) -> anyhow::Result<()> {
    let db_files = target_meta
        .get_mut("db_files")
        .and_then(Value::as_object_mut)
        .context("db_meta_info.db_files must be a JSON object")?;
    let entry = db_files
        .entry(dbnum_key.to_string())
        .or_insert(source_db_file);
    if !entry.is_object() {
        anyhow::bail!("db_meta_info.db_files.{} must be a JSON object", dbnum_key);
    }
    entry["ref0s"] = Value::Array(
        target_ref0s_after
            .iter()
            .copied()
            .map(|value| Value::Number(value.into()))
            .collect(),
    );
    Ok(())
}

fn merge_ref0_to_dbnum(meta: &mut Value, source_ref0s: &[u64], dbnum: u32) -> anyhow::Result<()> {
    let map = meta
        .get_mut("ref0_to_dbnum")
        .and_then(Value::as_object_mut)
        .context("db_meta_info.ref0_to_dbnum must be a JSON object")?;
    for ref0 in source_ref0s {
        map.insert(ref0.to_string(), Value::Number((dbnum as u64).into()));
    }
    Ok(())
}

fn copy_file_atomic(source: &Path, target: &Path) -> anyhow::Result<()> {
    let parent = target
        .parent()
        .with_context(|| format!("target path has no parent: {}", target.display()))?;
    let temp = temp_path(target, "tmp");
    fs::copy(source, &temp).with_context(|| {
        format!(
            "copy scene_tree file failed: {} -> {}",
            source.display(),
            temp.display()
        )
    })?;
    replace_file_with_backup(&temp, target, "scene_tree file")?;
    if !parent.is_dir() {
        anyhow::bail!(
            "target scene_tree parent disappeared during restore: {}",
            parent.display()
        );
    }
    Ok(())
}

fn write_json_atomic(path: &Path, value: &Value) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("JSON output path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create JSON output dir failed: {}", parent.display()))?;
    let temp = temp_path(path, "tmp");
    let content = serde_json::to_vec_pretty(value).context("serialize db_meta_info JSON")?;
    fs::write(&temp, content)
        .with_context(|| format!("write temp JSON file failed: {}", temp.display()))?;
    replace_file_with_backup(&temp, path, "db_meta_info JSON")?;
    Ok(())
}

fn replace_file_with_backup(temp: &Path, target: &Path, label: &str) -> anyhow::Result<()> {
    if !temp.is_file() {
        anyhow::bail!(
            "temp {} does not exist before replace: {}",
            label,
            temp.display()
        );
    }
    let backup = temp_path(target, "bak");
    if backup.exists() {
        fs::remove_file(&backup)
            .with_context(|| format!("remove stale backup failed: {}", backup.display()))?;
    }

    let had_target = target.exists();
    if had_target {
        fs::rename(target, &backup).with_context(|| {
            format!(
                "move existing {} to backup failed: {} -> {}",
                label,
                target.display(),
                backup.display()
            )
        })?;
    }

    match fs::rename(temp, target) {
        Ok(()) => {
            if had_target && backup.exists() {
                let _ = fs::remove_file(&backup);
            }
            Ok(())
        }
        Err(rename_error) => {
            let mut restore_error = None;
            if had_target
                && backup.exists()
                && let Err(error) = fs::rename(&backup, target)
            {
                restore_error = Some(error);
            }
            if let Some(error) = restore_error {
                anyhow::bail!(
                    "replace {} failed: {} -> {}; additionally failed to restore backup {} -> {}: {}; original rename error: {}",
                    label,
                    temp.display(),
                    target.display(),
                    backup.display(),
                    target.display(),
                    error,
                    rename_error
                );
            }
            anyhow::bail!(
                "replace {} failed: {} -> {}: {}",
                label,
                temp.display(),
                target.display(),
                rename_error
            );
        }
    }
}

fn temp_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("scene_tree_artifact");
    path.with_file_name(format!(".{}.{}.{}", file_name, std::process::id(), suffix))
}
