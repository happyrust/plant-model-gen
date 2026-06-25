use crate::version_management::baseline_state::{
    BaselineStateExpectation, optional_baseline_state_evidence_from_metadata,
    required_baseline_state_evidence_from_metadata, validate_baseline_state_evidence,
};
use crate::version_management::ducklake_store::ModelVersionDuckLakeStore;
use crate::version_management::history_replay_validation::{
    ensure_history_replay_publishable, validate_history_replay_package,
};
use crate::version_management::release_package::{
    default_release_package_dir, materialize_release_package,
};
use crate::version_management::types::{
    ModelComponentDiffResponse, ModelComponentSnapshotStats, ModelComponentUnitImpactResponse,
    ModelHistoryReleasePublishRequest, ModelHistoryReleasePublishResponse,
    ModelHistoryReleaseSafetyChecks, ModelHistoryReplayValidationRequest,
    ModelReleaseEventsResponse, ModelReleaseListResponse, ModelReleaseMeshAssetIndexResponse,
    ModelReleaseMeshAssetIndexStats, ModelReleasePairReadinessResponse, ModelReleaseQuality,
    ModelReleaseReconcileReport, ModelReleaseRecord, ModelReleaseRegisterRequest,
    ModelReleaseRegistration, ModelReleaseRegistrationStatus, ModelReleaseSceneResponse,
    ModelReleaseStatus, ModelUnitDiffResponse, ModelUnitIndexStats,
    ModelVersionCatalogMigrationReport, ModelVersionDuckLakeConfig,
};
use anyhow::Context;
use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub fn publish_history_model_release(
    request: ModelHistoryReleasePublishRequest,
) -> anyhow::Result<ModelHistoryReleasePublishResponse> {
    validate_history_publish_request(&request)?;
    let validation = validate_history_replay_package(ModelHistoryReplayValidationRequest {
        project_name: request.project_name.clone(),
        dbnum: request.dbnum,
        source_db_file: request.source_db_file.clone(),
        from_sesno: request.from_sesno,
        to_sesno: request.to_sesno,
        source_parquet_dir: request.source_parquet_dir.clone(),
        current_parquet_dir: request.current_parquet_dir.clone(),
        scene_tree_dir: request.scene_tree_dir.clone(),
        require_scene_tree: request.require_scene_tree,
    })?;
    ensure_history_replay_publishable(&validation)?;
    let instances_rows = validation.package.instances_rows;
    let geo_instances_rows = validation.package.geo_instances_rows;
    ensure_visual_publish_materializes_assets(&request, geo_instances_rows)?;
    let baseline_state = required_baseline_state_evidence_from_metadata(
        &request.extra_metadata,
        BaselineStateExpectation {
            project_name: &request.project_name,
            dbnum: request.dbnum,
            from_sesno: Some(request.from_sesno),
        },
    )?;

    let generation_job_id = metadata_string_candidates(
        &request.extra_metadata,
        &[&["generation_job_id"], &["job_id"]],
    )
    .unwrap_or_else(|| {
        format!(
            "external-publish-history:{}:{}-{}",
            request.release_id, request.from_sesno, request.to_sesno
        )
    });
    let history_metadata = serde_json::json!({
        "history_publish": {
            "source": "model-version publish-history",
            "replay_mode": "isolated-staged-parquet",
            "generation_performed_by_command": false,
            "generation_job_id": generation_job_id,
            "source_db_file": request.source_db_file,
            "from_sesno": request.from_sesno,
            "to_sesno": request.to_sesno,
            "dbnum": request.dbnum,
            "source_parquet_dir": request.source_parquet_dir,
            "current_parquet_dir": request.current_parquet_dir,
            "scene_tree": validation.scene_tree.clone(),
            "scene_tree_required": request.require_scene_tree,
            "instances_rows": instances_rows,
            "geo_instances_rows": geo_instances_rows,
            "non_empty_model_package": validation.package.non_empty_visual_package,
            "baseline_state_manifest_path": baseline_state.manifest_path.to_string_lossy().to_string(),
            "baseline_state_manifest_hash": baseline_state.manifest_hash.clone(),
            "baseline_state_id": baseline_state.manifest.snapshot_id.clone(),
            "baseline_source_db_latest_sesno": baseline_state.manifest.source_db_latest_sesno,
            "baseline_replacement_db_file": baseline_state.manifest.replacement_db_file.to_string_lossy().to_string(),
            "baseline_replacement_db_sha256": baseline_state.manifest.replacement_db_sha256.clone(),
            "zero_model_package_guard_enabled": true,
            "validation_classification": validation.classification,
            "safety_note": "source Parquet must come from an isolated replay/output root; current Parquet is rejected",
            "user_metadata": request.extra_metadata,
        }
    });
    let mut validation_flags = merge_validation_flags(
        request.validation_flags.clone(),
        validation_flags_from_metadata(&request.extra_metadata, false),
    );
    if validation.classification == "quarantined_visual_release_candidate" {
        push_unique_flag(&mut validation_flags, "mesh_missing_rows_quarantined");
    }
    let spec_info_fallback_count = request.spec_info_fallback_count.or_else(|| {
        metadata_u64_candidates(&request.extra_metadata, &[&["spec_info_fallback_count"]])
    });
    if spec_info_fallback_count.unwrap_or(0) > 0 {
        push_unique_flag(&mut validation_flags, "spec_info_fallback");
    }
    let release_quality = request.release_quality.clone().or_else(|| {
        if validation.classification == "quarantined_visual_release_candidate" {
            Some(ModelReleaseQuality::QuarantinedVisual)
        } else {
            None
        }
    });
    let release_quality_reason = request.release_quality_reason.clone().or_else(|| {
        metadata_string_candidates(
            &request.extra_metadata,
            &[&["release_quality_reason"], &["quality_reason"]],
        )
    });
    let release_quality_reason = release_quality_reason.or_else(|| {
        if validation.classification == "quarantined_visual_release_candidate" {
            Some(validation.recommended_action.clone())
        } else {
            None
        }
    });

    let registration = register_model_release(ModelReleaseRegisterRequest {
        project_name: request.project_name.clone(),
        release_id: request.release_id.clone(),
        release_label: request.release_label.clone(),
        release_quality,
        release_quality_reason,
        validation_flags,
        spec_info_fallback_count,
        branch_id: request.branch_id.clone(),
        parent_release_id: request.parent_release_id.clone(),
        derivation_type: "incremental-sesno-isolated".to_string(),
        dbnum: request.dbnum,
        source_parquet_dir: request.source_parquet_dir.clone(),
        release_root: request.release_root.clone(),
        ducklake: request.ducklake.clone(),
        extra_metadata: history_metadata,
        initial_status: ModelReleaseStatus::Staged,
    })?;

    update_release_status(
        request.ducklake.clone(),
        &request.release_id,
        ModelReleaseStatus::Validating,
        "history replay package validated",
    )?;

    let publish_steps = (|| -> anyhow::Result<(
        Option<ModelReleaseMeshAssetIndexStats>,
        Option<ModelUnitIndexStats>,
    )> {
        let mesh_asset_index = if request.materialize_assets {
            let mesh_root = request.mesh_root.as_deref().ok_or_else(|| {
                anyhow::anyhow!("mesh_root is required when materialize_assets=true")
            })?;
            let stats = index_model_release_mesh_assets(
                request.ducklake.clone(),
                &request.release_id,
                mesh_root,
                request.mesh_base_url.as_deref(),
                true,
            )?;
            ensure_mesh_asset_index_publishable(&request.release_id, &stats)?;
            update_release_status(
                request.ducklake.clone(),
                &request.release_id,
                ModelReleaseStatus::AssetsMaterialized,
                "release mesh assets materialized and indexed",
            )?;
            Some(stats)
        } else {
            None
        };

        let unit_index = if request.index_units {
            let stats = index_model_release_units(request.ducklake.clone(), &request.release_id)?;
            update_release_status(
                request.ducklake.clone(),
                &request.release_id,
                ModelReleaseStatus::Indexed,
                "release unit index completed",
            )?;
            Some(stats)
        } else {
            None
        };

        Ok((mesh_asset_index, unit_index))
    })();

    let (mesh_asset_index, unit_index) = match publish_steps {
        Ok(value) => value,
        Err(error) => {
            if let Err(status_error) = update_release_status(
                request.ducklake.clone(),
                &request.release_id,
                ModelReleaseStatus::Failed,
                &error.to_string(),
            ) {
                return Err(error).context(format!(
                    "also failed to mark release '{}' failed: {status_error}",
                    request.release_id
                ));
            }
            return Err(error);
        }
    };

    update_release_status(
        request.ducklake.clone(),
        &request.release_id,
        ModelReleaseStatus::Published,
        "history release published",
    )?;

    let mut registration = registration;
    registration.release = ModelVersionDuckLakeStore::open_readonly(request.ducklake.clone())?
        .get_release(&request.release_id)?;
    write_release_sidecar(&registration.release)?;

    Ok(ModelHistoryReleasePublishResponse {
        release: registration,
        safety_checks: ModelHistoryReleaseSafetyChecks {
            source_db_file: request.source_db_file,
            source_parquet_dir: request.source_parquet_dir,
            current_parquet_dir: request.current_parquet_dir,
            current_parquet_rejected: true,
            instances_rows,
            geo_instances_rows,
            non_empty_model_package: validation.package.non_empty_visual_package,
            zero_model_package_guard_enabled: true,
            replay_mode: "isolated-staged-parquet".to_string(),
            generation_performed_by_command: false,
            scene_tree: validation.scene_tree,
        },
        mesh_asset_index,
        unit_index,
    })
}

pub fn register_model_release(
    request: ModelReleaseRegisterRequest,
) -> anyhow::Result<ModelReleaseRegistration> {
    validate_release_register_request(&request)?;
    let baseline_state = optional_baseline_state_evidence_from_metadata(&request.extra_metadata)?;
    if let Some(evidence) = baseline_state.as_ref() {
        validate_baseline_state_evidence(
            evidence,
            BaselineStateExpectation {
                project_name: &request.project_name,
                dbnum: request.dbnum,
                from_sesno: None,
            },
        )?;
    }
    let (baseline_state_manifest_path, baseline_state_manifest_hash) = baseline_state
        .as_ref()
        .map(|evidence| {
            (
                Some(evidence.manifest_path.clone()),
                Some(evidence.manifest_hash.clone()),
            )
        })
        .unwrap_or((None, None));
    ensure_release_package_path_boundaries(
        &request.source_parquet_dir,
        None,
        &request.release_root,
        &request.release_id,
        request.dbnum,
    )?;
    let package = materialize_release_package(
        &request.source_parquet_dir,
        &request.release_root,
        &request.release_id,
        request.dbnum,
    )?;
    let registered_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let source_manifest_path = package.package_dir.join("manifest.json");
    let source_manifest_hash = crate::version_management::hashing::sha256_file(
        &source_manifest_path,
    )
    .with_context(|| {
        format!(
            "hash release package manifest failed: {}",
            source_manifest_path.display()
        )
    })?;
    let generation_job_id = release_generation_job_id(&request);
    let target_status = request.initial_status.clone();
    let insert_status = if target_status == ModelReleaseStatus::Published {
        ModelReleaseStatus::Staged
    } else {
        target_status.clone()
    };
    let spec_info_fallback_count = request
        .spec_info_fallback_count
        .or_else(|| {
            metadata_u64_candidates(
                &request.extra_metadata,
                &[
                    &["spec_info_fallback_count"],
                    &[
                        "history_publish",
                        "user_metadata",
                        "spec_info_fallback_count",
                    ],
                ],
            )
        })
        .or_else(|| {
            metadata_u64_candidates(
                &package.manifest_json,
                &[
                    &["spec_info_fallback_count"],
                    &["spec_info_validation", "fallback_count"],
                ],
            )
        });
    let mut validation_flags = merge_validation_flags(
        request.validation_flags.clone(),
        validation_flags_from_metadata(&request.extra_metadata, true),
    );
    if spec_info_fallback_count.unwrap_or(0) > 0 {
        push_unique_flag(&mut validation_flags, "spec_info_fallback");
    }
    let release = ModelReleaseRecord {
        release_id: request.release_id.clone(),
        project_name: request.project_name.clone(),
        branch_id: request.branch_id.clone(),
        release_lifecycle: insert_status.lifecycle(),
        release_quality: request.release_quality.clone().unwrap_or_else(|| {
            ModelReleaseQuality::from_storage_or_infer(
                metadata_string_candidates(
                    &request.extra_metadata,
                    &[
                        &["release_quality"],
                        &["quality"],
                        &["history_publish", "user_metadata", "release_quality"],
                        &["history_publish", "user_metadata", "quality"],
                    ],
                ),
                &insert_status,
                &request.release_id,
                request.release_label.as_deref(),
                &request.derivation_type,
                package.rows_by_table.get("instances").copied(),
                package.rows_by_table.get("geo_instances").copied(),
            )
        }),
        release_quality_reason: request.release_quality_reason.clone().or_else(|| {
            metadata_string_candidates(
                &request.extra_metadata,
                &[
                    &["release_quality_reason"],
                    &["quality_reason"],
                    &["history_publish", "user_metadata", "release_quality_reason"],
                    &["history_publish", "user_metadata", "quality_reason"],
                ],
            )
        }),
        validation_flags,
        spec_info_fallback_count,
        release_status: insert_status,
        release_label: request.release_label.clone(),
        dbnum: request.dbnum,
        source_package_dir: request.source_parquet_dir.clone(),
        immutable_package_dir: package.package_dir.clone(),
        package_hash: package.package_hash.clone(),
        derivation_type: request.derivation_type.clone(),
        created_at: package.generated_at.clone(),
        registered_at,
        rows_by_table: package.rows_by_table.clone(),
        source_manifest_path: Some(source_manifest_path),
        source_manifest_hash: Some(source_manifest_hash),
        baseline_state_manifest_path,
        baseline_state_manifest_hash,
        generation_job_id,
        asset_manifest_path: None,
        asset_manifest_hash: None,
    };

    let store = ModelVersionDuckLakeStore::open_writer(request.ducklake.clone())?;
    let mut registration = store.register_release(
        &release,
        &package.files,
        request.parent_release_id.as_deref(),
        &package.manifest_json,
        &request.extra_metadata,
    )?;
    let component_index = match store.ensure_release_components_indexed(&registration.release) {
        Ok(stats) => stats,
        Err(error) => {
            if registration.status == ModelReleaseRegistrationStatus::Created {
                if store
                    .update_release_status(
                        &registration.release.release_id,
                        ModelReleaseStatus::Failed,
                        Some(&error.to_string()),
                    )
                    .is_ok()
                    && let Ok(failed_release) = store.get_release(&registration.release.release_id)
                {
                    let _ = write_release_sidecar(&failed_release);
                }
            }
            return Err(error);
        }
    };
    registration.component_index = Some(component_index);
    if registration.status == ModelReleaseRegistrationStatus::Created
        && registration.release.release_status != target_status
    {
        store.update_release_status(
            &registration.release.release_id,
            target_status.clone(),
            Some("release registration completed"),
        )?;
        registration.release = store.get_release(&registration.release.release_id)?;
    }
    write_release_sidecar(&registration.release)?;
    Ok(registration)
}

pub fn annotate_model_release(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
    release_quality: Option<ModelReleaseQuality>,
    release_quality_reason: Option<&str>,
    validation_flags: &[String],
    spec_info_fallback_count: Option<u64>,
) -> anyhow::Result<ModelReleaseRecord> {
    let store = ModelVersionDuckLakeStore::open_writer(ducklake)?;
    let release = store.annotate_release_quality(
        release_id,
        release_quality,
        release_quality_reason,
        validation_flags,
        spec_info_fallback_count,
    )?;
    write_release_sidecar(&release)?;
    Ok(release)
}

pub fn migrate_model_version_catalog(
    project_name: &str,
    ducklake: ModelVersionDuckLakeConfig,
) -> anyhow::Result<ModelVersionCatalogMigrationReport> {
    let store = ModelVersionDuckLakeStore::open_writer(ducklake)?;
    store.catalog_migration_report(project_name)
}

fn release_generation_job_id(request: &ModelReleaseRegisterRequest) -> Option<String> {
    metadata_string_candidates(
        &request.extra_metadata,
        &[
            &["generation_job_id"],
            &["job_id"],
            &["history_publish", "generation_job_id"],
            &["history_publish", "user_metadata", "generation_job_id"],
            &["history_publish", "user_metadata", "job_id"],
            &["history_baseline", "generation_job_id"],
            &["history_baseline", "job_id"],
        ],
    )
}

fn validate_release_id(release_id: &str) -> anyhow::Result<()> {
    validate_path_safe_identifier("release_id", release_id, 128)
}

fn validate_release_register_request(request: &ModelReleaseRegisterRequest) -> anyhow::Result<()> {
    validate_release_id(&request.release_id)?;
    validate_path_safe_identifier("project_name", &request.project_name, 128)?;
    validate_path_safe_identifier("branch_id", &request.branch_id, 128)?;
    if let Some(parent_release_id) = request.parent_release_id.as_deref() {
        validate_path_safe_identifier("parent_release_id", parent_release_id, 128)?;
        if parent_release_id == request.release_id {
            anyhow::bail!(
                "parent_release_id cannot equal release_id '{}'",
                request.release_id
            );
        }
    }
    Ok(())
}

fn validate_path_safe_identifier(label: &str, raw: &str, max_len: usize) -> anyhow::Result<()> {
    let value = raw.trim();
    if value.is_empty() {
        anyhow::bail!("{label} cannot be empty");
    }
    if value.len() > max_len {
        anyhow::bail!(
            "{label} is too long: {} bytes, max {}",
            value.len(),
            max_len
        );
    }
    if value == "." || value == ".." {
        anyhow::bail!("{label} cannot be '.' or '..'");
    }
    if value != raw {
        anyhow::bail!("{label} cannot contain leading or trailing whitespace");
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        anyhow::bail!(
            "{label} '{}' contains unsafe characters; use only ASCII letters, digits, '.', '_' and '-'",
            raw
        );
    }
    Ok(())
}

fn metadata_string_candidates(metadata: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| metadata_string_at(metadata, path))
}

fn metadata_string_vec_candidates(metadata: &Value, paths: &[&[&str]]) -> Vec<String> {
    paths
        .iter()
        .find_map(|path| metadata_string_vec_at(metadata, path))
        .unwrap_or_default()
}

fn validation_flags_from_metadata(metadata: &Value, include_history_user: bool) -> Vec<String> {
    let mut flags = if include_history_user {
        metadata_string_vec_candidates(
            metadata,
            &[
                &["validation_flags"],
                &["flags"],
                &["history_publish", "user_metadata", "validation_flags"],
                &["history_publish", "user_metadata", "flags"],
            ],
        )
    } else {
        metadata_string_vec_candidates(metadata, &[&["validation_flags"], &["flags"]])
    };
    for flag in missing_mesh_repair_flags_from_metadata(metadata) {
        push_unique_flag(&mut flags, &flag);
    }
    flags
}

fn missing_mesh_repair_flags_from_metadata(metadata: &Value) -> Vec<String> {
    let mut flags = Vec::new();
    let paths: &[&[&str]] = &[
        &["missing_mesh_repair"],
        &["mesh_repair"],
        &["repair_missing_meshes"],
        &["history_publish", "user_metadata", "missing_mesh_repair"],
        &["history_publish", "user_metadata", "mesh_repair"],
        &["history_publish", "user_metadata", "repair_missing_meshes"],
    ];
    for path in paths {
        let mut value = metadata;
        for segment in *path {
            let Some(next) = value.get(*segment) else {
                value = &Value::Null;
                break;
            };
            value = next;
        }
        if value.is_null() {
            continue;
        }
        if metadata_u64_at(value, &["still_missing_hashes"]).unwrap_or(0) > 0 {
            push_unique_flag(&mut flags, "mesh_missing_rows_quarantined");
        }
        if metadata_u64_at(value, &["degraded_fradius_fallback_rows"]).unwrap_or(0) > 0 {
            push_unique_flag(&mut flags, "degraded_geometry_fallback");
        }
        if metadata_u64_at(value, &["self_intersecting_inputs"]).unwrap_or(0) > 0 {
            push_unique_flag(&mut flags, "self_intersecting_input");
        }
        if metadata_u64_at(value, &["non_renderable_inputs"]).unwrap_or(0) > 0 {
            push_unique_flag(&mut flags, "non_renderable_input");
        }
        if metadata_u64_at(value, &["missing_inst_geo"]).unwrap_or(0) > 0 {
            push_unique_flag(&mut flags, "missing_inst_geo");
        }
    }
    flags
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

fn metadata_string_vec_at(metadata: &Value, path: &[&str]) -> Option<Vec<String>> {
    let mut value = metadata;
    for segment in path {
        value = value.get(*segment)?;
    }
    if let Some(values) = value.as_array() {
        return Some(
            values
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        );
    }
    value.as_str().map(|raw| {
        raw.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn metadata_u64_candidates(metadata: &Value, paths: &[&[&str]]) -> Option<u64> {
    paths
        .iter()
        .find_map(|path| metadata_u64_at(metadata, path))
}

fn metadata_u64_at(metadata: &Value, path: &[&str]) -> Option<u64> {
    let mut value = metadata;
    for segment in path {
        value = value.get(*segment)?;
    }
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

fn merge_validation_flags(mut explicit: Vec<String>, inferred: Vec<String>) -> Vec<String> {
    for flag in inferred {
        push_unique_flag(&mut explicit, &flag);
    }
    explicit
}

fn push_unique_flag(flags: &mut Vec<String>, flag: &str) {
    let value = flag.trim();
    if value.is_empty() {
        return;
    }
    if !flags.iter().any(|existing| existing == value) {
        flags.push(value.to_string());
    }
}

pub(crate) fn write_release_sidecar(release: &ModelReleaseRecord) -> anyhow::Result<PathBuf> {
    let release_dir = release
        .immutable_package_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "release package path has no release root: {}",
                release.immutable_package_dir.display()
            )
        })?;
    fs::create_dir_all(release_dir).with_context(|| {
        format!(
            "create release sidecar dir failed: {}",
            release_dir.display()
        )
    })?;
    let path = release_dir.join("release.json");
    let body = serde_json::json!({
        "schema_version": "model_release_sidecar:v1",
        "release_id": &release.release_id,
        "project_name": &release.project_name,
        "branch_id": &release.branch_id,
        "dbnum": release.dbnum,
        "release_lifecycle": &release.release_lifecycle,
        "release_quality": &release.release_quality,
        "release_quality_reason": &release.release_quality_reason,
        "validation_flags": &release.validation_flags,
        "spec_info_fallback_count": release.spec_info_fallback_count,
        "release_status": &release.release_status,
        "release_label": &release.release_label,
        "derivation_type": &release.derivation_type,
        "generation_job_id": &release.generation_job_id,
        "registered_at": &release.registered_at,
        "created_at": &release.created_at,
        "immutable_package_dir": release.immutable_package_dir.to_string_lossy().to_string(),
        "source_package_dir": release.source_package_dir.to_string_lossy().to_string(),
        "package_hash": &release.package_hash,
        "rows_by_table": &release.rows_by_table,
        "source_manifest_path": release.source_manifest_path.as_ref().map(|path| path.to_string_lossy().to_string()),
        "source_manifest_hash": &release.source_manifest_hash,
        "baseline_state_manifest_path": release.baseline_state_manifest_path.as_ref().map(|path| path.to_string_lossy().to_string()),
        "baseline_state_manifest_hash": &release.baseline_state_manifest_hash,
        "asset_manifest_path": release.asset_manifest_path.as_ref().map(|path| path.to_string_lossy().to_string()),
        "asset_manifest_hash": &release.asset_manifest_hash,
    });
    let bytes = serde_json::to_vec_pretty(&body)?;
    fs::write(&path, bytes)
        .with_context(|| format!("write release sidecar failed: {}", path.display()))?;
    Ok(path)
}

fn update_release_status(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
    status: ModelReleaseStatus,
    reason: &str,
) -> anyhow::Result<()> {
    let store = ModelVersionDuckLakeStore::open_writer(ducklake)?;
    store.update_release_status(release_id, status, Some(reason))?;
    let release = store.get_release(release_id)?;
    write_release_sidecar(&release)?;
    Ok(())
}

fn validate_history_publish_request(
    request: &ModelHistoryReleasePublishRequest,
) -> anyhow::Result<()> {
    validate_release_id(&request.release_id)?;
    validate_path_safe_identifier("project_name", &request.project_name, 128)?;
    validate_path_safe_identifier("branch_id", &request.branch_id, 128)?;
    if let Some(parent_release_id) = request.parent_release_id.as_deref() {
        validate_path_safe_identifier("parent_release_id", parent_release_id, 128)?;
        if parent_release_id == request.release_id {
            anyhow::bail!(
                "parent_release_id cannot equal release_id '{}'",
                request.release_id
            );
        }
    }
    if request.from_sesno >= request.to_sesno {
        anyhow::bail!(
            "invalid sesno range for historical release: from_sesno={} must be less than to_sesno={}",
            request.from_sesno,
            request.to_sesno
        );
    }
    if !request.source_db_file.exists() {
        anyhow::bail!(
            "source DB file for historical release does not exist: {}",
            request.source_db_file.display()
        );
    }
    if !request.source_db_file.is_file() {
        anyhow::bail!(
            "source DB path for historical release is not a file: {}",
            request.source_db_file.display()
        );
    }
    if !request.source_parquet_dir.exists() {
        anyhow::bail!(
            "historical release source Parquet directory does not exist: {}",
            request.source_parquet_dir.display()
        );
    }
    if !request.source_parquet_dir.is_dir() {
        anyhow::bail!(
            "historical release source Parquet path is not a directory: {}",
            request.source_parquet_dir.display()
        );
    }
    if paths_refer_to_same_existing_dir(&request.source_parquet_dir, &request.current_parquet_dir)?
    {
        anyhow::bail!(
            "refusing to publish historical release from current Parquet directory: {}. \
             Generate/export history into an isolated replay or staging output root first.",
            request.source_parquet_dir.display()
        );
    }
    ensure_release_package_path_boundaries(
        &request.source_parquet_dir,
        Some(&request.current_parquet_dir),
        &request.release_root,
        &request.release_id,
        request.dbnum,
    )?;
    Ok(())
}

fn ensure_visual_publish_materializes_assets(
    request: &ModelHistoryReleasePublishRequest,
    geo_instances_rows: u64,
) -> anyhow::Result<()> {
    if geo_instances_rows > 0 && !request.materialize_assets {
        anyhow::bail!(
            "visual historical release '{}' references {} geo_instances rows but materialize_assets=false; \
             rerun publish-history with --materialize-assets and a valid --mesh-root so the published release is self-contained",
            request.release_id,
            geo_instances_rows
        );
    }
    Ok(())
}

fn ensure_mesh_asset_index_publishable(
    release_id: &str,
    stats: &ModelReleaseMeshAssetIndexStats,
) -> anyhow::Result<()> {
    if stats.missing_count > 0 {
        anyhow::bail!(
            "release '{}' cannot be published because {} non-builtin mesh assets are missing after materialization; \
             repair/generate the missing GLB files or classify the affected geometry before publishing",
            release_id,
            stats.missing_count
        );
    }
    match stats.glb_unreadable_count {
        Some(count) if count > 0 => {
            anyhow::bail!(
                "release '{}' cannot be published because {} GLB mesh assets are unreadable after materialization; \
                 repair/regenerate the bad GLB files before publishing",
                release_id,
                count
            );
        }
        Some(_) => {}
        None => {
            anyhow::bail!(
                "release '{}' cannot be published because mesh asset GLB readability evidence is missing; \
                 rerun index-assets with the current build before publishing",
                release_id
            );
        }
    }
    Ok(())
}

fn paths_refer_to_same_existing_dir(left: &Path, right: &Path) -> anyhow::Result<bool> {
    if !left.exists() || !right.exists() {
        return Ok(false);
    }
    let left = left
        .canonicalize()
        .map_err(|err| anyhow::anyhow!("canonicalize {} failed: {}", left.display(), err))?;
    let right = right
        .canonicalize()
        .map_err(|err| anyhow::anyhow!("canonicalize {} failed: {}", right.display(), err))?;
    Ok(left == right)
}

fn ensure_release_package_path_boundaries(
    source_parquet_dir: &Path,
    current_parquet_dir: Option<&Path>,
    release_root: &Path,
    release_id: &str,
    dbnum: u32,
) -> anyhow::Result<()> {
    let destination = default_release_package_dir(release_root, release_id, dbnum);
    ensure_not_nested_release_package_path(source_parquet_dir, &destination, "source_parquet_dir")?;
    if let Some(current_parquet_dir) = current_parquet_dir {
        ensure_not_nested_release_package_path(
            current_parquet_dir,
            &destination,
            "current_parquet_dir",
        )?;
    }
    Ok(())
}

fn ensure_not_nested_release_package_path(
    package_input_dir: &Path,
    destination_dir: &Path,
    label: &str,
) -> anyhow::Result<()> {
    if paths_refer_to_same_existing_dir(package_input_dir, destination_dir)? {
        return Ok(());
    }
    let input = absolute_lexical_path(package_input_dir)?;
    let destination = absolute_lexical_path(destination_dir)?;
    if path_is_equal_or_nested(&destination, &input) {
        anyhow::bail!(
            "release package destination {} is inside {label} {}; choose an isolated release_root",
            destination_dir.display(),
            package_input_dir.display()
        );
    }
    if path_is_equal_or_nested(&input, &destination) {
        anyhow::bail!(
            "{label} {} is inside release package destination {}; choose an isolated source/current Parquet directory",
            package_input_dir.display(),
            destination_dir.display()
        );
    }
    Ok(())
}

fn absolute_lexical_path(path: &Path) -> anyhow::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve current working directory")?
            .join(path)
    };
    Ok(normalize_lexical_path(&absolute))
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn path_is_equal_or_nested(child: &Path, parent: &Path) -> bool {
    let child_components = path_components_for_compare(child);
    let parent_components = path_components_for_compare(parent);
    child_components.len() >= parent_components.len()
        && child_components
            .iter()
            .zip(parent_components.iter())
            .all(|(child, parent)| child == parent)
}

fn path_components_for_compare(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| {
            let value = component.as_os_str().to_string_lossy().replace('/', "\\");
            if cfg!(windows) {
                value.to_ascii_lowercase()
            } else {
                value
            }
        })
        .collect()
}

pub fn list_model_releases(
    ducklake: ModelVersionDuckLakeConfig,
    project_name: Option<&str>,
) -> anyhow::Result<ModelReleaseListResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    store.list_releases(project_name)
}

pub fn get_model_release_events(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
) -> anyhow::Result<ModelReleaseEventsResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    store.release_events(release_id)
}

pub fn reconcile_model_release(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
    publish_if_complete: bool,
    fail_if_unusable: bool,
) -> anyhow::Result<ModelReleaseReconcileReport> {
    let store = ModelVersionDuckLakeStore::open_writer(ducklake)?;
    let mut report = store.reconcile_release(release_id, publish_if_complete, fail_if_unusable)?;
    let initial_applied = report.applied;
    let initial_action = report.action_taken.clone();
    let repaired_source_manifest = store
        .repair_release_source_manifest_to_package(release_id)?
        .is_some();
    if repaired_source_manifest {
        report = store.reconcile_release(release_id, false, false)?;
    }
    let repaired_sidecar = !report.release_sidecar_exists;
    if report.applied || repaired_source_manifest || repaired_sidecar {
        let sidecar_path = write_release_sidecar(&report.release)?;
        if repaired_source_manifest || repaired_sidecar {
            let mut refreshed = store.reconcile_release(release_id, false, false)?;
            refreshed.applied = true;
            let mut actions = Vec::new();
            if initial_applied && initial_action != "none" {
                actions.push(initial_action);
            }
            if repaired_source_manifest {
                actions.push("source manifest evidence repaired".to_string());
            }
            if repaired_sidecar {
                actions.push("release sidecar written".to_string());
            }
            refreshed.action_taken = actions.join(", ");
            refreshed.recommended_action = if refreshed.problems.is_empty() {
                "release evidence was repaired; rerun readiness/state-machine for production gates"
                    .to_string()
            } else {
                "release evidence was repaired; resolve remaining reconcile problems".to_string()
            };
            refreshed.release_sidecar_path = sidecar_path;
            refreshed.release_sidecar_exists = true;
            refreshed.release_sidecar_hash = Some(crate::version_management::hashing::sha256_file(
                &refreshed.release_sidecar_path,
            )?);
            report = refreshed;
        }
    }
    Ok(report)
}

pub fn index_model_release_components(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
) -> anyhow::Result<ModelComponentSnapshotStats> {
    let store = ModelVersionDuckLakeStore::open_writer(ducklake)?;
    let release = store.get_release(release_id)?;
    store.index_release_components(&release)
}

pub fn diff_model_releases(
    ducklake: ModelVersionDuckLakeConfig,
    from_release_id: &str,
    to_release_id: &str,
    limit: usize,
    change_type_filter: Option<&str>,
) -> anyhow::Result<ModelComponentDiffResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    store.diff_releases(from_release_id, to_release_id, limit, change_type_filter)
}

pub fn validate_model_release_pair_readiness(
    ducklake: ModelVersionDuckLakeConfig,
    from_release_id: &str,
    to_release_id: &str,
) -> anyhow::Result<ModelReleasePairReadinessResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    store.compare_readiness(from_release_id, to_release_id)
}

pub fn index_model_release_units(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
) -> anyhow::Result<ModelUnitIndexStats> {
    let store = ModelVersionDuckLakeStore::open_writer(ducklake)?;
    let release = store.get_release(release_id)?;
    store.index_release_units(&release)
}

pub fn diff_model_release_units(
    ducklake: ModelVersionDuckLakeConfig,
    from_release_id: &str,
    to_release_id: &str,
    limit: usize,
    unit_noun_filter: Option<&str>,
) -> anyhow::Result<ModelUnitDiffResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    store.diff_units(from_release_id, to_release_id, limit, unit_noun_filter)
}

pub fn get_model_component_unit_impacts(
    ducklake: ModelVersionDuckLakeConfig,
    from_release_id: &str,
    to_release_id: &str,
    limit: usize,
    component_key_filter: Option<&str>,
) -> anyhow::Result<ModelComponentUnitImpactResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    store.component_unit_impacts(from_release_id, to_release_id, limit, component_key_filter)
}

pub fn index_model_release_mesh_assets(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
    mesh_root: &Path,
    mesh_base_url: Option<&str>,
    materialize: bool,
) -> anyhow::Result<ModelReleaseMeshAssetIndexStats> {
    let store = ModelVersionDuckLakeStore::open_writer(ducklake)?;
    let release = store.get_release(release_id)?;
    let stats = store.index_release_mesh_assets(&release, mesh_root, mesh_base_url, materialize)?;
    let release = store.get_release(release_id)?;
    write_release_sidecar(&release)?;
    Ok(stats)
}

pub fn get_model_release_mesh_assets(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
    limit: usize,
    missing_only: bool,
) -> anyhow::Result<ModelReleaseMeshAssetIndexResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    store.get_release_mesh_assets(release_id, limit, missing_only)
}

pub fn get_model_release_scene(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
    limit: usize,
    offset: usize,
    component_key: Option<&str>,
) -> anyhow::Result<ModelReleaseSceneResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    store.release_scene(release_id, limit, offset, component_key)
}
