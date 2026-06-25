use crate::version_management::hashing;
use crate::version_management::types::{ModelPackageManifest, ModelReleaseFile};
use anyhow::{Context, anyhow};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

const REQUIRED_TABLES: &[&str] = &[
    "instances",
    "geo_instances",
    "transforms",
    "aabb",
    "tubings",
    "ptsets",
    "primitive_keypoints",
];
const MAX_RELEASE_STORAGE_DIR_CHARS: usize = 24;
const RELEASE_STORAGE_PREFIX_CHARS: usize = 6;
const RELEASE_STORAGE_HASH_CHARS: usize = 16;

pub fn validate_release_id_for_path(release_id: &str) -> anyhow::Result<()> {
    if release_id.trim().is_empty() {
        anyhow::bail!("release_id is empty");
    }
    let valid = release_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid {
        anyhow::bail!(
            "release_id '{}' is not path-safe; use only ASCII letters, digits, '.', '_' and '-'",
            release_id
        );
    }
    Ok(())
}

pub fn default_release_package_dir(release_root: &Path, release_id: &str, dbnum: u32) -> PathBuf {
    release_root
        .join(release_storage_dir_name(release_id))
        .join("parquet")
        .join(dbnum.to_string())
}

fn release_storage_dir_name(release_id: &str) -> String {
    if release_id.chars().count() <= MAX_RELEASE_STORAGE_DIR_CHARS {
        return release_id.to_string();
    }

    let hash = hashing::sha256_bytes(release_id.as_bytes());
    let mut prefix = release_id
        .chars()
        .take(RELEASE_STORAGE_PREFIX_CHARS)
        .collect::<String>();
    prefix = prefix.trim_matches(['.', '-', '_']).to_string();
    let hash_prefix = &hash[..RELEASE_STORAGE_HASH_CHARS];
    if prefix.is_empty() {
        format!("r-{hash_prefix}")
    } else {
        format!("{prefix}-{hash_prefix}")
    }
}

pub fn load_model_package(
    package_dir: impl AsRef<Path>,
    expected_dbnum: u32,
) -> anyhow::Result<ModelPackageManifest> {
    let package_dir = package_dir.as_ref();
    if !package_dir.exists() {
        anyhow::bail!(
            "Parquet package directory does not exist: {}",
            package_dir.display()
        );
    }
    if !package_dir.is_dir() {
        anyhow::bail!(
            "Parquet package path is not a directory: {}",
            package_dir.display()
        );
    }

    let manifest_path = package_dir.join("manifest.json");
    if !manifest_path.exists() {
        anyhow::bail!(
            "Parquet package is missing manifest.json: {}",
            manifest_path.display()
        );
    }

    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("read manifest failed: {}", manifest_path.display()))?;
    let manifest_json: Value = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("parse manifest JSON failed: {}", manifest_path.display()))?;

    let dbnum = manifest_json
        .get("dbnum")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| anyhow!("manifest missing numeric dbnum"))? as u32;
    if dbnum != expected_dbnum {
        anyhow::bail!(
            "manifest dbnum mismatch: expected {}, got {} in {}",
            expected_dbnum,
            dbnum,
            manifest_path.display()
        );
    }

    let tables = manifest_json
        .get("tables")
        .and_then(|value| value.as_object())
        .ok_or_else(|| anyhow!("manifest missing tables object"))?;

    let mut files = Vec::new();
    let mut rows_by_table = BTreeMap::new();
    let mut seen_paths = HashSet::new();
    push_file_entry(
        &mut files,
        &mut seen_paths,
        package_dir,
        "manifest",
        "manifest.json",
        None,
        true,
    )?;

    for table in REQUIRED_TABLES {
        let entry = tables
            .get(*table)
            .ok_or_else(|| anyhow!("manifest missing required table '{}'", table))?;
        let file_name = entry
            .get("file")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("manifest table '{}' missing file", table))?;
        let rows = entry
            .get("rows")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| anyhow!("manifest table '{}' missing numeric rows", table))?;
        rows_by_table.insert((*table).to_string(), rows);
        push_file_entry(
            &mut files,
            &mut seen_paths,
            package_dir,
            table,
            file_name,
            Some(rows),
            true,
        )?;
    }

    if let Some(report_file) = manifest_json
        .get("mesh_validation")
        .and_then(|value| value.get("report_file"))
        .and_then(|value| value.as_str())
    {
        let report_rel = safe_relative_path(report_file)?;
        if package_dir.join(&report_rel).exists() {
            push_file_entry(
                &mut files,
                &mut seen_paths,
                package_dir,
                "mesh_validation_report",
                report_file,
                None,
                false,
            )?;
        }
    }

    let total_bytes = files.iter().map(|file| file.bytes).sum();
    let package_hash = hashing::package_hash(&files)?;
    let generated_at = manifest_json
        .get("generated_at")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

    Ok(ModelPackageManifest {
        dbnum,
        generated_at,
        package_dir: package_dir.to_path_buf(),
        manifest_json,
        rows_by_table,
        files,
        total_bytes,
        package_hash,
    })
}

pub fn materialize_release_package(
    source_dir: impl AsRef<Path>,
    release_root: impl AsRef<Path>,
    release_id: &str,
    dbnum: u32,
) -> anyhow::Result<ModelPackageManifest> {
    validate_release_id_for_path(release_id)?;
    let source_dir = source_dir.as_ref();
    let release_root = release_root.as_ref();
    let source_manifest = load_model_package(source_dir, dbnum)?;
    let dest_dir = default_release_package_dir(release_root, release_id, dbnum);

    if paths_equal_if_existing(source_dir, &dest_dir)? {
        return Ok(source_manifest);
    }

    if dest_dir.exists() {
        let existing = load_model_package(&dest_dir, dbnum)?;
        if existing.package_hash != source_manifest.package_hash {
            anyhow::bail!(
                "release package already exists with different content: {}",
                dest_dir.display()
            );
        }
        return Ok(existing);
    }

    let parent = dest_dir.parent().ok_or_else(|| {
        anyhow!(
            "release package destination has no parent: {}",
            dest_dir.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create release package parent failed: {}", parent.display()))?;

    let tmp_dir = parent.join(format!(
        ".{}.tmp.{}",
        dest_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("package"),
        std::process::id()
    ));
    if tmp_dir.exists() {
        fs::remove_dir_all(&tmp_dir).with_context(|| {
            format!(
                "remove stale temp package dir failed: {}",
                tmp_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&tmp_dir)
        .with_context(|| format!("create temp package dir failed: {}", tmp_dir.display()))?;

    let copy_result = copy_package_files(&source_manifest, &tmp_dir)
        .and_then(|_| load_model_package(&tmp_dir, dbnum))
        .and_then(|tmp_manifest| {
            if tmp_manifest.package_hash != source_manifest.package_hash {
                anyhow::bail!(
                    "copied package hash mismatch: source={} copied={}",
                    source_manifest.package_hash,
                    tmp_manifest.package_hash
                );
            }
            fs::rename(&tmp_dir, &dest_dir).with_context(|| {
                format!(
                    "move temp package {} to {} failed",
                    tmp_dir.display(),
                    dest_dir.display()
                )
            })?;
            load_model_package(&dest_dir, dbnum)
        });

    if copy_result.is_err() && tmp_dir.exists() {
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    copy_result
}

fn copy_package_files(
    source_manifest: &ModelPackageManifest,
    tmp_dir: &Path,
) -> anyhow::Result<()> {
    for file in &source_manifest.files {
        let rel = safe_relative_path(&file.relative_path)?;
        let src = source_manifest.package_dir.join(&rel);
        let dst = tmp_dir.join(&rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create package subdir failed: {}", parent.display()))?;
        }
        fs::copy(&src, &dst).with_context(|| {
            format!(
                "copy package file {} to {} failed",
                src.display(),
                dst.display()
            )
        })?;
    }
    Ok(())
}

fn push_file_entry(
    files: &mut Vec<ModelReleaseFile>,
    seen_paths: &mut HashSet<String>,
    package_dir: &Path,
    logical_name: &str,
    relative_path: &str,
    rows: Option<u64>,
    required: bool,
) -> anyhow::Result<()> {
    let rel_path = safe_relative_path(relative_path)?;
    let rel_string = rel_path.to_string_lossy().replace('\\', "/");
    if !seen_paths.insert(rel_string.clone()) {
        return Ok(());
    }
    let absolute_path = package_dir.join(&rel_path);
    if !absolute_path.exists() {
        if required {
            anyhow::bail!(
                "required package file '{}' is missing: {}",
                logical_name,
                absolute_path.display()
            );
        }
        return Ok(());
    }
    if !absolute_path.is_file() {
        anyhow::bail!(
            "package entry '{}' is not a file: {}",
            logical_name,
            absolute_path.display()
        );
    }
    let metadata = fs::metadata(&absolute_path)
        .with_context(|| format!("read file metadata failed: {}", absolute_path.display()))?;
    let sha256 = hashing::sha256_file(&absolute_path)?;
    validate_expected_parquet_rows(&absolute_path, logical_name, rows)?;
    files.push(ModelReleaseFile {
        logical_name: logical_name.to_string(),
        relative_path: rel_string,
        absolute_path,
        bytes: metadata.len(),
        sha256,
        rows,
        required,
    });
    Ok(())
}

fn safe_relative_path(raw: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(raw);
    if path.as_os_str().is_empty() || path.is_absolute() {
        anyhow::bail!("unsafe package relative path '{}'", raw);
    }
    for component in path.components() {
        if matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        ) {
            anyhow::bail!("unsafe package relative path '{}'", raw);
        }
    }
    Ok(path.to_path_buf())
}

fn paths_equal_if_existing(a: &Path, b: &Path) -> anyhow::Result<bool> {
    if !a.exists() || !b.exists() {
        return Ok(false);
    }
    let a = a
        .canonicalize()
        .with_context(|| format!("canonicalize path failed: {}", a.display()))?;
    let b = b
        .canonicalize()
        .with_context(|| format!("canonicalize path failed: {}", b.display()))?;
    Ok(a == b)
}

#[cfg(feature = "parquet-export")]
fn validate_expected_parquet_rows(
    path: &Path,
    logical_name: &str,
    expected_rows: Option<u64>,
) -> anyhow::Result<()> {
    let Some(expected_rows) = expected_rows else {
        return Ok(());
    };
    if path.extension().and_then(|value| value.to_str()) != Some("parquet") {
        return Ok(());
    }

    use parquet::file::reader::{FileReader, SerializedFileReader};
    let file = fs::File::open(path).with_context(|| {
        format!(
            "open parquet for row-count validation failed: {}",
            path.display()
        )
    })?;
    let reader = SerializedFileReader::new(file)
        .with_context(|| format!("read parquet metadata failed: {}", path.display()))?;
    let actual_rows =
        u64::try_from(reader.metadata().file_metadata().num_rows()).with_context(|| {
            format!(
                "parquet row count is negative or out of range for {}",
                path.display()
            )
        })?;
    if actual_rows != expected_rows {
        anyhow::bail!(
            "manifest row count mismatch for '{}': manifest={} actual={} file={}",
            logical_name,
            expected_rows,
            actual_rows,
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(feature = "parquet-export"))]
fn validate_expected_parquet_rows(
    _path: &Path,
    _logical_name: &str,
    _expected_rows: Option<u64>,
) -> anyhow::Result<()> {
    Ok(())
}
