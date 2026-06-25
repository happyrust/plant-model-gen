use crate::version_management::hashing::sha256_file;
use crate::version_management::types::{
    ModelSourceObservationFileEvidence, ModelSourceObservationManifest,
    ModelSourceObservationQuiescence,
};
use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug)]
pub struct SourceObservationEvidence {
    pub manifest_path: PathBuf,
    pub manifest_hash: String,
    pub manifest: ModelSourceObservationManifest,
}

#[derive(Clone, Debug)]
pub struct SourceObservationBuildRequest {
    pub observation_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub primary_file: PathBuf,
    pub dependency_files: Vec<PathBuf>,
    pub requested_sesno: Option<String>,
    pub resolved_sesno: Option<u32>,
    pub quiescence_window_ms: u64,
}

pub fn build_source_observation_manifest(
    request: SourceObservationBuildRequest,
) -> anyhow::Result<ModelSourceObservationManifest> {
    validate_observation_id(&request.observation_id)?;
    let started_at = now_rfc3339();
    let primary_before = file_evidence("primary", &request.primary_file)?;
    if request.quiescence_window_ms > 0 {
        std::thread::sleep(Duration::from_millis(request.quiescence_window_ms));
    }
    let primary_after = file_evidence("primary", &request.primary_file)?;
    let confirmed_at = now_rfc3339();
    let stable = primary_before.bytes == primary_after.bytes
        && primary_before.sha256 == primary_after.sha256;

    let mut dependencies = Vec::with_capacity(request.dependency_files.len());
    for (index, path) in request.dependency_files.iter().enumerate() {
        dependencies.push(file_evidence(&format!("dependency:{index}"), path)?);
    }

    Ok(ModelSourceObservationManifest {
        manifest_version: "source_observation_manifest:v1".to_string(),
        observation_id: request.observation_id,
        project_name: request.project_name,
        dbnum: request.dbnum,
        requested_sesno: request.requested_sesno,
        resolved_sesno: request.resolved_sesno,
        observed_at: confirmed_at.clone(),
        primary: primary_after.clone(),
        dependencies,
        quiescence: ModelSourceObservationQuiescence {
            requested_window_ms: request.quiescence_window_ms,
            checks_performed: if request.quiescence_window_ms > 0 {
                2
            } else {
                1
            },
            stable,
            started_at,
            confirmed_at,
            primary_sha256_before: primary_before.sha256,
            primary_sha256_after: primary_after.sha256,
            primary_bytes_before: primary_before.bytes,
            primary_bytes_after: primary_after.bytes,
            note: if stable {
                "primary file evidence was stable for the requested observation window".to_string()
            } else {
                "primary file changed during the requested observation window".to_string()
            },
        },
    })
}

pub fn write_source_observation_manifest(
    path: &Path,
    manifest: &ModelSourceObservationManifest,
) -> anyhow::Result<String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("create source observation dir failed: {}", parent.display())
        })?;
    }
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(manifest)?).with_context(|| {
        format!(
            "write temporary source observation failed: {}",
            tmp.display()
        )
    })?;
    if path.exists() {
        fs::remove_file(path).with_context(|| {
            format!(
                "remove previous source observation failed: {}",
                path.display()
            )
        })?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("replace source observation failed: {}", path.display()))?;
    sha256_file(path).with_context(|| {
        format!(
            "hash source observation manifest failed: {}",
            path.display()
        )
    })
}

pub fn load_source_observation_manifest(
    path: &Path,
    expected_hash: Option<&str>,
) -> anyhow::Result<SourceObservationEvidence> {
    if !path.is_file() {
        anyhow::bail!(
            "source observation manifest is missing or not a file: {}",
            path.display()
        );
    }
    let manifest_hash = sha256_file(path).with_context(|| {
        format!(
            "hash source observation manifest failed: {}",
            path.display()
        )
    })?;
    if let Some(expected) = expected_hash
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !manifest_hash.eq_ignore_ascii_case(expected) {
            anyhow::bail!(
                "source observation manifest hash mismatch: expected {}, got {} ({})",
                expected,
                manifest_hash,
                path.display()
            );
        }
    }
    let content = fs::read_to_string(path).with_context(|| {
        format!(
            "read source observation manifest failed: {}",
            path.display()
        )
    })?;
    let manifest: ModelSourceObservationManifest =
        serde_json::from_str(&content).with_context(|| {
            format!(
                "parse source observation manifest failed: {}",
                path.display()
            )
        })?;
    if manifest.manifest_version != "source_observation_manifest:v1" {
        anyhow::bail!(
            "unsupported source observation manifest version: {} ({})",
            manifest.manifest_version,
            path.display()
        );
    }
    Ok(SourceObservationEvidence {
        manifest_path: path.to_path_buf(),
        manifest_hash,
        manifest,
    })
}

pub fn validate_source_observation_for_increment(
    evidence: &SourceObservationEvidence,
    project_name: &str,
    dbnum: u32,
    from_sesno: u32,
    to_sesno: Option<u32>,
) -> anyhow::Result<()> {
    let manifest = &evidence.manifest;
    if manifest.project_name != project_name {
        anyhow::bail!(
            "source observation project mismatch: expected {}, got {} ({})",
            project_name,
            manifest.project_name,
            evidence.manifest_path.display()
        );
    }
    if manifest.dbnum != dbnum {
        anyhow::bail!(
            "source observation dbnum mismatch: expected {}, got {} ({})",
            dbnum,
            manifest.dbnum,
            evidence.manifest_path.display()
        );
    }
    if !manifest.quiescence.stable {
        anyhow::bail!(
            "source observation is not stable: {} ({})",
            manifest.quiescence.note,
            evidence.manifest_path.display()
        );
    }
    if let Some(resolved_sesno) = manifest.resolved_sesno {
        if from_sesno > resolved_sesno {
            anyhow::bail!(
                "source observation resolved_sesno {} is older than from_sesno {} ({})",
                resolved_sesno,
                from_sesno,
                evidence.manifest_path.display()
            );
        }
        if let Some(to_sesno) = to_sesno {
            if to_sesno > resolved_sesno {
                anyhow::bail!(
                    "source observation resolved_sesno {} is older than to_sesno {} ({})",
                    resolved_sesno,
                    to_sesno,
                    evidence.manifest_path.display()
                );
            }
        }
    }
    Ok(())
}

pub fn verify_source_observation_primary_hash(
    evidence: &SourceObservationEvidence,
    stage: &str,
) -> anyhow::Result<String> {
    let primary = &evidence.manifest.primary;
    if !primary.path.is_file() {
        anyhow::bail!(
            "source observation primary file is missing during {}: {}",
            stage,
            primary.path.display()
        );
    }
    let current_hash = sha256_file(&primary.path).with_context(|| {
        format!(
            "hash source observation primary file failed during {}: {}",
            stage,
            primary.path.display()
        )
    })?;
    if !current_hash.eq_ignore_ascii_case(&primary.sha256) {
        anyhow::bail!(
            "source observation primary hash mismatch during {}: expected {}, got {} ({})",
            stage,
            primary.sha256,
            current_hash,
            primary.path.display()
        );
    }
    Ok(current_hash)
}

fn file_evidence(role: &str, path: &Path) -> anyhow::Result<ModelSourceObservationFileEvidence> {
    if !path.is_file() {
        anyhow::bail!(
            "source observation file is missing or not a file: {}",
            path.display()
        );
    }
    let metadata = fs::metadata(path).with_context(|| {
        format!(
            "read source observation metadata failed: {}",
            path.display()
        )
    })?;
    let modified_at = metadata.modified().ok().map(system_time_to_rfc3339);
    let sha256 = sha256_file(path)
        .with_context(|| format!("hash source observation file failed: {}", path.display()))?;
    Ok(ModelSourceObservationFileEvidence {
        role: role.to_string(),
        path: fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        bytes: metadata.len(),
        modified_at,
        sha256,
    })
}

fn validate_observation_id(observation_id: &str) -> anyhow::Result<()> {
    let trimmed = observation_id.trim();
    if trimmed.is_empty() {
        anyhow::bail!("observation_id must not be empty");
    }
    if trimmed.len() > 128 {
        anyhow::bail!("observation_id must be <= 128 characters");
    }
    if trimmed.contains("..")
        || trimmed.starts_with('.')
        || trimmed.ends_with('.')
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!(
            "observation_id must be path-safe ASCII using only letters, numbers, dash, underscore, or dot"
        );
    }
    Ok(())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn system_time_to_rfc3339(time: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Utc> = time.into();
    datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
