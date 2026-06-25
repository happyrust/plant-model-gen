use crate::version_management::release_package::load_model_package;
use crate::version_management::types::{
    ModelHistoryReplayPackageEvidence, ModelHistoryReplayPathChecks,
    ModelHistoryReplaySceneTreeEvidence, ModelHistoryReplayValidationRequest,
    ModelHistoryReplayValidationResponse,
};
use anyhow::Context;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

const CLASS_COMPLETE: &str = "complete_visual_release_candidate";
const CLASS_PATCH_ONLY_EMPTY: &str = "patch_only_empty_baseline";
const CLASS_INVALID_PACKAGE: &str = "invalid_replay_package";
const CLASS_UNSAFE_CURRENT: &str = "unsafe_current_output";
const CLASS_MISSING_SOURCE: &str = "missing_source_artifacts";
const CLASS_MISSING_SCENE_TREE: &str = "missing_scene_tree_baseline";
const CLASS_MISSING_MESH_ASSETS: &str = "missing_mesh_assets";
const CLASS_QUARANTINED_MESH_ASSETS: &str = "quarantined_visual_release_candidate";

pub fn validate_history_replay_package(
    request: ModelHistoryReplayValidationRequest,
) -> anyhow::Result<ModelHistoryReplayValidationResponse> {
    let path_checks = build_path_checks(&request)?;
    let package = build_package_evidence(&request);
    let scene_tree = build_scene_tree_evidence(&request);
    let (classification, ready_for_publish, recommended_action) = classify_replay(
        &path_checks,
        &package,
        scene_tree.as_ref(),
        request.require_scene_tree,
    );

    Ok(ModelHistoryReplayValidationResponse {
        project_name: request.project_name,
        dbnum: request.dbnum,
        source_db_file: request.source_db_file,
        from_sesno: request.from_sesno,
        to_sesno: request.to_sesno,
        source_parquet_dir: request.source_parquet_dir,
        current_parquet_dir: request.current_parquet_dir,
        classification,
        ready_for_publish,
        recommended_action,
        path_checks,
        package,
        scene_tree,
    })
}

pub fn ensure_history_replay_publishable(
    response: &ModelHistoryReplayValidationResponse,
) -> anyhow::Result<()> {
    if response.ready_for_publish {
        return Ok(());
    }

    if response.classification == CLASS_PATCH_ONLY_EMPTY {
        anyhow::bail!(
            "refusing to publish historical release package with zero model rows: instances={} geo_instances={} package={}. \
             A sesno range replayed into an empty namespace is a patch, not a complete 3D release. \
             Build or restore a baseline state before applying the historical range, then publish the non-empty replay package.",
            response.package.instances_rows,
            response.package.geo_instances_rows,
            response.source_parquet_dir.display()
        );
    }

    if response.classification == CLASS_MISSING_MESH_ASSETS {
        anyhow::bail!(
            "refusing to publish historical release package with missing mesh assets: missing_geo_hashes={} missing_owner_refnos={} package={}. \
             Generate/materialize the missing GLB files or explicitly classify the affected geometry before publishing a visual release.",
            response.package.missing_mesh_geo_hashes.unwrap_or(0),
            response.package.missing_mesh_owner_refnos.unwrap_or(0),
            response.source_parquet_dir.display()
        );
    }

    anyhow::bail!(
        "historical replay package is not publishable: classification={} package={} action={}",
        response.classification,
        response.source_parquet_dir.display(),
        response.recommended_action
    );
}

fn build_path_checks(
    request: &ModelHistoryReplayValidationRequest,
) -> anyhow::Result<ModelHistoryReplayPathChecks> {
    let source_parquet_dir_exists = request.source_parquet_dir.exists();
    let current_parquet_dir_exists = request.current_parquet_dir.exists();
    Ok(ModelHistoryReplayPathChecks {
        sesno_range_valid: request.from_sesno < request.to_sesno,
        source_db_file_exists: request.source_db_file.exists(),
        source_db_file_is_file: request.source_db_file.is_file(),
        source_parquet_dir_exists,
        source_parquet_dir_is_dir: request.source_parquet_dir.is_dir(),
        current_parquet_dir_exists,
        source_parquet_differs_from_current: !paths_equivalent(
            &request.source_parquet_dir,
            &request.current_parquet_dir,
        )?,
    })
}

fn build_package_evidence(
    request: &ModelHistoryReplayValidationRequest,
) -> ModelHistoryReplayPackageEvidence {
    match load_model_package(&request.source_parquet_dir, request.dbnum) {
        Ok(package) => {
            let instances_rows = package.row_count("instances").unwrap_or(0);
            let geo_instances_rows = package.row_count("geo_instances").unwrap_or(0);
            let transforms_rows = package.row_count("transforms").unwrap_or(0);
            let aabb_rows = package.row_count("aabb").unwrap_or(0);
            let mesh_validation = package.manifest_json.get("mesh_validation");
            let mesh_validation_present = mesh_validation.is_some();
            let raw_missing_mesh_geo_hashes = mesh_validation
                .and_then(|value| value.get("raw_missing_geo_hashes"))
                .and_then(|value| value.as_u64())
                .or_else(|| {
                    mesh_validation
                        .and_then(|value| value.get("missing_geo_hashes"))
                        .and_then(|value| value.as_u64())
                });
            let raw_missing_mesh_owner_refnos = mesh_validation
                .and_then(|value| value.get("raw_missing_owner_refnos"))
                .and_then(|value| value.as_u64())
                .or_else(|| {
                    mesh_validation
                        .and_then(|value| value.get("missing_owner_refnos"))
                        .and_then(|value| value.as_u64())
                });
            let render_missing_mesh_geo_hashes = mesh_validation
                .and_then(|value| value.get("render_missing_geo_hashes"))
                .and_then(|value| value.as_u64())
                .or(raw_missing_mesh_geo_hashes);
            let render_missing_mesh_owner_refnos = mesh_validation
                .and_then(|value| value.get("render_missing_owner_refnos"))
                .and_then(|value| value.as_u64())
                .or(raw_missing_mesh_owner_refnos);
            let quarantined_mesh_geo_hashes = mesh_validation
                .and_then(|value| value.get("quarantined_geo_hashes"))
                .and_then(|value| value.as_u64());
            let quarantined_mesh_owner_refnos = mesh_validation
                .and_then(|value| value.get("quarantined_owner_refnos"))
                .and_then(|value| value.as_u64());
            let missing_mesh_geo_hashes = render_missing_mesh_geo_hashes;
            let missing_mesh_owner_refnos = render_missing_mesh_owner_refnos;
            let quarantine_counts_consistent = mesh_validation_present
                && missing_counts_conserved(
                    raw_missing_mesh_geo_hashes,
                    quarantined_mesh_geo_hashes,
                    render_missing_mesh_geo_hashes,
                )
                && missing_counts_conserved(
                    raw_missing_mesh_owner_refnos,
                    quarantined_mesh_owner_refnos,
                    render_missing_mesh_owner_refnos,
                );
            let mesh_assets_complete = mesh_validation_present
                && quarantine_counts_consistent
                && render_missing_mesh_geo_hashes.unwrap_or(u64::MAX) == 0
                && render_missing_mesh_owner_refnos.unwrap_or(u64::MAX) == 0;
            ModelHistoryReplayPackageEvidence {
                manifest_loaded: true,
                package_error: None,
                rows_by_table: package.rows_by_table,
                instances_rows,
                geo_instances_rows,
                transforms_rows,
                aabb_rows,
                missing_mesh_geo_hashes,
                missing_mesh_owner_refnos,
                raw_missing_mesh_geo_hashes,
                raw_missing_mesh_owner_refnos,
                render_missing_mesh_geo_hashes,
                render_missing_mesh_owner_refnos,
                quarantined_mesh_geo_hashes,
                quarantined_mesh_owner_refnos,
                mesh_validation_present,
                quarantine_counts_consistent,
                mesh_assets_complete,
                non_empty_visual_package: instances_rows > 0 && geo_instances_rows > 0,
            }
        }
        Err(error) => ModelHistoryReplayPackageEvidence {
            manifest_loaded: false,
            package_error: Some(error.to_string()),
            rows_by_table: BTreeMap::new(),
            instances_rows: 0,
            geo_instances_rows: 0,
            transforms_rows: 0,
            aabb_rows: 0,
            missing_mesh_geo_hashes: None,
            missing_mesh_owner_refnos: None,
            raw_missing_mesh_geo_hashes: None,
            raw_missing_mesh_owner_refnos: None,
            render_missing_mesh_geo_hashes: None,
            render_missing_mesh_owner_refnos: None,
            quarantined_mesh_geo_hashes: None,
            quarantined_mesh_owner_refnos: None,
            mesh_validation_present: false,
            quarantine_counts_consistent: false,
            mesh_assets_complete: false,
            non_empty_visual_package: false,
        },
    }
}

fn missing_counts_conserved(
    raw_missing: Option<u64>,
    quarantined: Option<u64>,
    render_missing: Option<u64>,
) -> bool {
    let Some(raw_missing) = raw_missing else {
        return false;
    };
    raw_missing
        == quarantined
            .unwrap_or(0)
            .saturating_add(render_missing.unwrap_or(0))
}

fn build_scene_tree_evidence(
    request: &ModelHistoryReplayValidationRequest,
) -> Option<ModelHistoryReplaySceneTreeEvidence> {
    let scene_tree_dir = request
        .scene_tree_dir
        .clone()
        .or_else(|| infer_project_output_dir_from_parquet_dir(&request.source_parquet_dir))
        .map(|project_output_dir| {
            if project_output_dir
                .file_name()
                .and_then(|value| value.to_str())
                == Some("scene_tree")
            {
                project_output_dir
            } else {
                project_output_dir.join("scene_tree")
            }
        })?;
    let tree_file = scene_tree_dir.join(format!("{}.tree", request.dbnum));
    let db_meta_info_file = scene_tree_dir.join("db_meta_info.json");
    Some(ModelHistoryReplaySceneTreeEvidence {
        tree_file_exists: tree_file.is_file(),
        db_meta_info_exists: db_meta_info_file.is_file(),
        scene_tree_dir,
        tree_file,
        db_meta_info_file,
        required: request.require_scene_tree,
    })
}

fn classify_replay(
    path_checks: &ModelHistoryReplayPathChecks,
    package: &ModelHistoryReplayPackageEvidence,
    scene_tree: Option<&ModelHistoryReplaySceneTreeEvidence>,
    require_scene_tree: bool,
) -> (String, bool, String) {
    if !path_checks.sesno_range_valid
        || !path_checks.source_db_file_exists
        || !path_checks.source_db_file_is_file
        || !path_checks.source_parquet_dir_exists
        || !path_checks.source_parquet_dir_is_dir
    {
        return (
            CLASS_MISSING_SOURCE.to_string(),
            false,
            "Fix the source DB file, sesno range, and replay Parquet directory before publishing."
                .to_string(),
        );
    }

    if !path_checks.source_parquet_differs_from_current {
        return (
            CLASS_UNSAFE_CURRENT.to_string(),
            false,
            "Generate/export history into an isolated replay output root; do not publish the current mutable Parquet directory.".to_string(),
        );
    }

    if !package.manifest_loaded {
        return (
            CLASS_INVALID_PACKAGE.to_string(),
            false,
            package.package_error.clone().unwrap_or_else(|| {
                "Fix the replay package manifest and required Parquet files.".to_string()
            }),
        );
    }

    if !package.non_empty_visual_package {
        return (
            CLASS_PATCH_ONLY_EMPTY.to_string(),
            false,
            "Build or restore a complete baseline state before applying the historical range; this package has no visual model rows.".to_string(),
        );
    }

    if !package.mesh_validation_present {
        return (
            CLASS_INVALID_PACKAGE.to_string(),
            false,
            "Replay package manifest is missing mesh_validation; visual releases must prove raw/render/quarantine mesh counts before publishing.".to_string(),
        );
    }

    if !package.quarantine_counts_consistent {
        return (
            CLASS_INVALID_PACKAGE.to_string(),
            false,
            format!(
                "Replay package mesh_validation counts are inconsistent: raw_missing_geo_hashes={} quarantined_geo_hashes={} render_missing_geo_hashes={} raw_missing_owner_refnos={} quarantined_owner_refnos={} render_missing_owner_refnos={}.",
                package.raw_missing_mesh_geo_hashes.unwrap_or(0),
                package.quarantined_mesh_geo_hashes.unwrap_or(0),
                package.render_missing_mesh_geo_hashes.unwrap_or(0),
                package.raw_missing_mesh_owner_refnos.unwrap_or(0),
                package.quarantined_mesh_owner_refnos.unwrap_or(0),
                package.render_missing_mesh_owner_refnos.unwrap_or(0)
            ),
        );
    }

    if !package.mesh_assets_complete {
        return (
            CLASS_MISSING_MESH_ASSETS.to_string(),
            false,
            format!(
                "Generate, materialize, or quarantine all render-required GLB mesh assets before publishing; render_missing_geo_hashes={} render_missing_owner_refnos={}.",
                package.missing_mesh_geo_hashes.unwrap_or(0),
                package.missing_mesh_owner_refnos.unwrap_or(0)
            ),
        );
    }

    if require_scene_tree {
        let scene_tree_ready = scene_tree
            .map(|evidence| evidence.tree_file_exists && evidence.db_meta_info_exists)
            .unwrap_or(false);
        if !scene_tree_ready {
            return (
                CLASS_MISSING_SCENE_TREE.to_string(),
                false,
                "Build or restore scene_tree artifacts for the replay workspace, or rerun without --require-scene-tree only after verifying the visual package is sufficient.".to_string(),
            );
        }
    }

    if package.quarantined_mesh_geo_hashes.unwrap_or(0) > 0
        || package.quarantined_mesh_owner_refnos.unwrap_or(0) > 0
    {
        return (
            CLASS_QUARANTINED_MESH_ASSETS.to_string(),
            true,
            format!(
                "Replay package is renderable after quarantining missing mesh rows: raw_missing_geo_hashes={} quarantined_geo_hashes={} raw_missing_owner_refnos={} quarantined_owner_refnos={} dropped geometry is excluded from the visual package.",
                package.raw_missing_mesh_geo_hashes.unwrap_or(0),
                package.quarantined_mesh_geo_hashes.unwrap_or(0),
                package.raw_missing_mesh_owner_refnos.unwrap_or(0),
                package.quarantined_mesh_owner_refnos.unwrap_or(0)
            ),
        );
    }

    (
        CLASS_COMPLETE.to_string(),
        true,
        "Replay package is a non-empty visual release candidate; publish-history may proceed."
            .to_string(),
    )
}

fn infer_project_output_dir_from_parquet_dir(parquet_dir: &Path) -> Option<PathBuf> {
    let dbnum_dir = parquet_dir.file_name()?;
    if dbnum_dir.to_string_lossy().is_empty() {
        return None;
    }
    let parquet_parent = parquet_dir.parent()?;
    if parquet_parent.file_name().and_then(|value| value.to_str()) != Some("parquet") {
        return None;
    }
    parquet_parent.parent().map(Path::to_path_buf)
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
