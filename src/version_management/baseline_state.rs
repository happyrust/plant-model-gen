use crate::version_management::hashing::sha256_file;
use crate::version_management::types::{
    ModelBaselineStateValidationRequest, ModelBaselineStateValidationResponse,
    ModelHistoryReplaySceneTreeEvidence, ModelPhysicalBaselineStateManifest,
};
use anyhow::Context;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct BaselineStateEvidence {
    pub manifest_path: PathBuf,
    pub manifest_hash: String,
    pub manifest: ModelPhysicalBaselineStateManifest,
}

#[derive(Clone, Debug)]
pub struct BaselineStateExpectation<'a> {
    pub project_name: &'a str,
    pub dbnum: u32,
    pub from_sesno: Option<u32>,
}

pub fn validate_baseline_state_request(
    request: ModelBaselineStateValidationRequest,
) -> anyhow::Result<ModelBaselineStateValidationResponse> {
    let evidence = load_and_verify_baseline_manifest(
        &request.baseline_state_manifest_path,
        request.baseline_state_manifest_hash.as_deref(),
    )?;
    let expected_dbnum = request.dbnum.unwrap_or(evidence.manifest.dbnum);
    validate_baseline_state_evidence(
        &evidence,
        BaselineStateExpectation {
            project_name: &request.project_name,
            dbnum: expected_dbnum,
            from_sesno: request.from_sesno,
        },
    )?;

    let scene_tree = build_baseline_scene_tree_evidence(
        &evidence.manifest,
        expected_dbnum,
        request.scene_tree_dir.clone(),
        request.require_scene_tree,
    );
    let scene_tree_ready = scene_tree.tree_file_exists && scene_tree.db_meta_info_exists;
    if request.require_scene_tree && !scene_tree_ready {
        anyhow::bail!(
            "baseline_scene_tree_missing: --require-scene-tree enabled but baseline scene_tree is incomplete for dbnum {}; tree_file={} exists={} db_meta_info={} exists={}",
            expected_dbnum,
            scene_tree.tree_file.display(),
            scene_tree.tree_file_exists,
            scene_tree.db_meta_info_file.display(),
            scene_tree.db_meta_info_exists
        );
    }

    let manifest = evidence.manifest;
    let recommended_action = if scene_tree_ready {
        "baseline state is publishable; pass baseline_state_manifest_path and baseline_state_manifest_hash in --metadata-json before publish-history"
            .to_string()
    } else {
        "physical baseline snapshot is verifiable, but baseline scene_tree artifacts are incomplete; build/restore scene_tree before full visual replay or rerun with --require-scene-tree to fail closed"
            .to_string()
    };
    Ok(ModelBaselineStateValidationResponse {
        project_name: request.project_name,
        dbnum: manifest.dbnum,
        from_sesno: request.from_sesno,
        ready: true,
        manifest_version: manifest.manifest_version,
        snapshot_id: manifest.snapshot_id,
        baseline_dbnums: manifest.baseline_dbnums,
        baseline_state_manifest_path: evidence.manifest_path,
        baseline_state_manifest_hash: evidence.manifest_hash,
        source_db_file: manifest.source_db_file,
        source_db_sha256: manifest.source_db_sha256,
        replacement_db_file: manifest.replacement_db_file,
        replacement_db_sha256: manifest.replacement_db_sha256,
        source_db_latest_sesno: manifest.source_db_latest_sesno,
        snapshot_root: manifest.snapshot_root,
        config_path: manifest.config_path,
        output_root: manifest.output_root,
        surreal_ns: manifest.surreal_ns,
        safety_checks: manifest.safety_checks,
        scene_tree,
        recommended_action,
    })
}

fn build_baseline_scene_tree_evidence(
    manifest: &ModelPhysicalBaselineStateManifest,
    dbnum: u32,
    explicit_scene_tree_dir: Option<PathBuf>,
    required: bool,
) -> ModelHistoryReplaySceneTreeEvidence {
    let scene_tree_dir = explicit_scene_tree_dir.unwrap_or_else(|| {
        manifest
            .output_root
            .join(&manifest.project_name)
            .join("scene_tree")
    });
    let tree_file = scene_tree_dir.join(format!("{dbnum}.tree"));
    let db_meta_info_file = scene_tree_dir.join("db_meta_info.json");
    ModelHistoryReplaySceneTreeEvidence {
        tree_file_exists: tree_file.is_file(),
        db_meta_info_exists: db_meta_info_file.is_file(),
        scene_tree_dir,
        tree_file,
        db_meta_info_file,
        required,
    }
}

pub fn optional_baseline_state_evidence_from_metadata(
    metadata: &Value,
) -> anyhow::Result<Option<BaselineStateEvidence>> {
    let Some((path, hash)) = baseline_state_manifest_path_hash(metadata, false)? else {
        return Ok(None);
    };
    Ok(Some(load_and_verify_baseline_manifest(&path, Some(&hash))?))
}

pub fn required_baseline_state_evidence_from_metadata(
    metadata: &Value,
    expectation: BaselineStateExpectation<'_>,
) -> anyhow::Result<BaselineStateEvidence> {
    let Some((path, hash)) = baseline_state_manifest_path_hash(metadata, true)? else {
        anyhow::bail!(
            "baseline_missing: publish-history requires baseline_state_manifest_path and baseline_state_manifest_hash metadata; \
             prepare a physical baseline snapshot or restore a proven baseline release before publishing"
        );
    };
    let evidence = load_and_verify_baseline_manifest(&path, Some(&hash))?;
    validate_baseline_state_evidence(&evidence, expectation)?;
    Ok(evidence)
}

pub fn validate_baseline_state_evidence(
    evidence: &BaselineStateEvidence,
    expectation: BaselineStateExpectation<'_>,
) -> anyhow::Result<()> {
    let manifest = &evidence.manifest;
    if manifest.manifest_version != "physical_baseline_state_manifest:v1" {
        anyhow::bail!(
            "baseline_state_manifest has unsupported manifest_version '{}': {}",
            manifest.manifest_version,
            evidence.manifest_path.display()
        );
    }
    if manifest.project_name != expectation.project_name {
        anyhow::bail!(
            "baseline_state_manifest project mismatch: expected {}, got {} ({})",
            expectation.project_name,
            manifest.project_name,
            evidence.manifest_path.display()
        );
    }
    if manifest.dbnum != expectation.dbnum {
        anyhow::bail!(
            "baseline_state_manifest dbnum mismatch: expected {}, got {} ({})",
            expectation.dbnum,
            manifest.dbnum,
            evidence.manifest_path.display()
        );
    }
    if let Some(from_sesno) = expectation.from_sesno {
        if manifest.source_db_latest_sesno != from_sesno {
            anyhow::bail!(
                "baseline_state_manifest sesno mismatch: from_sesno={} requires baseline latest sesno {}, got {} ({})",
                from_sesno,
                from_sesno,
                manifest.source_db_latest_sesno,
                evidence.manifest_path.display()
            );
        }
    }
    if !manifest.replacement_db_file.is_file() {
        anyhow::bail!(
            "baseline replacement DB file is missing or not a file: {}",
            manifest.replacement_db_file.display()
        );
    }
    let replacement_hash = sha256_file(&manifest.replacement_db_file).with_context(|| {
        format!(
            "hash baseline replacement DB failed: {}",
            manifest.replacement_db_file.display()
        )
    })?;
    if !replacement_hash.eq_ignore_ascii_case(&manifest.replacement_db_sha256) {
        anyhow::bail!(
            "baseline replacement DB hash mismatch: manifest={} observed={} file={}",
            manifest.replacement_db_sha256,
            replacement_hash,
            manifest.replacement_db_file.display()
        );
    }
    validate_physical_snapshot_safety(manifest)?;
    Ok(())
}

fn load_and_verify_baseline_manifest(
    path: &Path,
    expected_hash: Option<&str>,
) -> anyhow::Result<BaselineStateEvidence> {
    if !path.is_file() {
        anyhow::bail!(
            "baseline state manifest path is not a file: {}",
            path.display()
        );
    }
    let actual_hash = sha256_file(path)
        .with_context(|| format!("hash baseline state manifest failed: {}", path.display()))?;
    if let Some(expected_hash) = expected_hash {
        if !actual_hash.eq_ignore_ascii_case(expected_hash) {
            anyhow::bail!(
                "baseline state manifest hash mismatch for {}: expected {}, got {}",
                path.display(),
                expected_hash,
                actual_hash
            );
        }
    }
    let manifest: ModelPhysicalBaselineStateManifest = serde_json::from_slice(
        &fs::read(path)
            .with_context(|| format!("read baseline state manifest failed: {}", path.display()))?,
    )
    .with_context(|| format!("parse baseline state manifest failed: {}", path.display()))?;
    Ok(BaselineStateEvidence {
        manifest_path: path.to_path_buf(),
        manifest_hash: actual_hash,
        manifest,
    })
}

fn baseline_state_manifest_path_hash(
    metadata: &Value,
    strict_pair: bool,
) -> anyhow::Result<Option<(PathBuf, String)>> {
    let path = metadata_string_candidates(
        metadata,
        &[
            &["baseline_state_manifest_path"],
            &["baseline_state_manifest"],
            &["history_publish", "baseline_state_manifest_path"],
            &["history_publish", "baseline_state_manifest"],
            &[
                "history_publish",
                "user_metadata",
                "baseline_state_manifest_path",
            ],
            &[
                "history_publish",
                "user_metadata",
                "baseline_state_manifest",
            ],
            &["history_baseline", "baseline_state_manifest_path"],
            &["history_baseline", "baseline_state_manifest"],
        ],
    )
    .map(PathBuf::from);
    let hash = metadata_string_candidates(
        metadata,
        &[
            &["baseline_state_manifest_hash"],
            &["history_publish", "baseline_state_manifest_hash"],
            &[
                "history_publish",
                "user_metadata",
                "baseline_state_manifest_hash",
            ],
            &["history_baseline", "baseline_state_manifest_hash"],
        ],
    );

    match (path, hash) {
        (Some(path), Some(hash)) => Ok(Some((path, hash))),
        (Some(path), None) if strict_pair => anyhow::bail!(
            "baseline_state_manifest_path '{}' was provided without baseline_state_manifest_hash; \
             publish-history requires a verifiable path+hash pair",
            path.display()
        ),
        (Some(path), None) => {
            let hash = sha256_file(&path).with_context(|| {
                format!("hash baseline state manifest failed: {}", path.display())
            })?;
            Ok(Some((path, hash)))
        }
        (None, Some(hash)) => anyhow::bail!(
            "baseline_state_manifest_hash '{}' was provided without a baseline manifest path; \
             publish a verifiable path+hash pair or omit both fields",
            hash
        ),
        (None, None) => Ok(None),
    }
}

fn validate_physical_snapshot_safety(
    manifest: &ModelPhysicalBaselineStateManifest,
) -> anyhow::Result<()> {
    let safety = &manifest.safety_checks;
    if !safety.source_db_file_exists
        || !safety.source_db_file_is_file
        || !safety.source_db_file_matches_dbnum
        || !safety.snapshot_project_differs_from_source
        || !safety.snapshot_output_differs_from_current
        || !safety.config_differs_from_base_config
        || !safety.original_project_not_modified
    {
        anyhow::bail!(
            "baseline_state_manifest safety checks are not publishable: exists={} is_file={} dbnum_match={} snapshot_project_differs={} output_differs={} config_differs={} original_not_modified={} snapshot={}",
            safety.source_db_file_exists,
            safety.source_db_file_is_file,
            safety.source_db_file_matches_dbnum,
            safety.snapshot_project_differs_from_source,
            safety.snapshot_output_differs_from_current,
            safety.config_differs_from_base_config,
            safety.original_project_not_modified,
            manifest.snapshot_id
        );
    }
    Ok(())
}

fn metadata_string_candidates(metadata: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| metadata_string_at(metadata, path))
}

fn metadata_string_at(metadata: &Value, path: &[&str]) -> Option<String> {
    let mut value = metadata;
    for segment in path {
        value = value.get(*segment)?;
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
