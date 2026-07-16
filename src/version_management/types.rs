use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelVersionDuckLakeConfig {
    pub metadata_path: PathBuf,
    pub data_path: PathBuf,
    pub catalog_name: String,
}

impl ModelVersionDuckLakeConfig {
    pub fn new(metadata_path: PathBuf, data_path: PathBuf) -> Self {
        Self {
            metadata_path,
            data_path,
            catalog_name: "model_version_lake".to_string(),
        }
    }

    pub fn for_project_output_dir(project_output_dir: &Path, project_name: &str) -> Self {
        const MAX_PROJECTED_DUCKLAKE_PATH_CHARS: usize = 240;

        let model_versions_root = project_output_dir.join("model_versions");
        let default = Self::new(
            model_versions_root.join("metadata.ducklake"),
            model_versions_root.join("data"),
        );
        if projected_ducklake_data_file_chars(&default.data_path)
            <= MAX_PROJECTED_DUCKLAKE_PATH_CHARS
        {
            return default;
        }

        let short_root = PathBuf::from("output")
            .join(project_name)
            .join("model_versions_ducklake");
        Self::new(
            short_root.join("metadata.ducklake"),
            short_root.join("data"),
        )
    }
}

fn projected_ducklake_data_file_chars(data_path: &Path) -> usize {
    let projected = data_path
        .join("model_version")
        .join("component_snapshots")
        .join("ducklake-00000000-0000-0000-0000-000000000000.parquet");
    absolute_lexical_path_for_len(&projected)
        .to_string_lossy()
        .chars()
        .count()
}

fn absolute_lexical_path_for_len(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseFile {
    pub logical_name: String,
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub rows: Option<u64>,
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelPackageManifest {
    pub dbnum: u32,
    pub generated_at: Option<String>,
    pub package_dir: PathBuf,
    pub manifest_json: Value,
    pub rows_by_table: BTreeMap<String, u64>,
    pub files: Vec<ModelReleaseFile>,
    pub total_bytes: u64,
    pub package_hash: String,
}

impl ModelPackageManifest {
    pub fn row_count(&self, table: &str) -> Option<u64> {
        self.rows_by_table.get(table).copied()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseRegisterRequest {
    pub project_name: String,
    pub release_id: String,
    pub release_label: Option<String>,
    pub release_quality: Option<ModelReleaseQuality>,
    pub release_quality_reason: Option<String>,
    #[serde(default)]
    pub validation_flags: Vec<String>,
    pub spec_info_fallback_count: Option<u64>,
    pub branch_id: String,
    pub parent_release_id: Option<String>,
    pub derivation_type: String,
    pub dbnum: u32,
    /// specs/023：导出/登记对应的业务 sesno；用于同步写入 unit_versions_v2。
    #[serde(default)]
    pub export_sesno: Option<u32>,
    pub source_parquet_dir: PathBuf,
    pub release_root: PathBuf,
    pub ducklake: ModelVersionDuckLakeConfig,
    pub extra_metadata: Value,
    pub initial_status: ModelReleaseStatus,
    #[serde(default)]
    pub index_units: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReleasePublishRequest {
    pub project_name: String,
    pub release_id: String,
    pub release_label: Option<String>,
    pub release_quality: Option<ModelReleaseQuality>,
    pub release_quality_reason: Option<String>,
    #[serde(default)]
    pub validation_flags: Vec<String>,
    pub spec_info_fallback_count: Option<u64>,
    pub branch_id: String,
    pub parent_release_id: Option<String>,
    pub dbnum: u32,
    pub source_db_file: PathBuf,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub source_parquet_dir: PathBuf,
    pub current_parquet_dir: PathBuf,
    #[serde(default)]
    pub scene_tree_dir: Option<PathBuf>,
    #[serde(default)]
    pub require_scene_tree: bool,
    pub release_root: PathBuf,
    pub ducklake: ModelVersionDuckLakeConfig,
    pub extra_metadata: Value,
    pub mesh_root: Option<PathBuf>,
    pub mesh_base_url: Option<String>,
    pub materialize_assets: bool,
    pub index_units: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReleaseSafetyChecks {
    pub source_db_file: PathBuf,
    pub source_parquet_dir: PathBuf,
    pub current_parquet_dir: PathBuf,
    pub current_parquet_rejected: bool,
    pub instances_rows: u64,
    pub geo_instances_rows: u64,
    pub non_empty_model_package: bool,
    pub zero_model_package_guard_enabled: bool,
    pub replay_mode: String,
    pub generation_performed_by_command: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_tree: Option<ModelHistoryReplaySceneTreeEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReleasePublishResponse {
    pub release: ModelReleaseRegistration,
    pub safety_checks: ModelHistoryReleaseSafetyChecks,
    pub mesh_asset_index: Option<ModelReleaseMeshAssetIndexStats>,
    pub unit_index: Option<ModelUnitIndexStats>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelVersionCatalogMigrationReport {
    pub project_name: String,
    pub ducklake_metadata_path: PathBuf,
    pub ducklake_data_path: PathBuf,
    pub catalog_name: String,
    pub schema_name: String,
    pub schema_migration_count: u64,
    pub required_schema_migrations: Vec<String>,
    pub applied_schema_migrations: Vec<String>,
    pub missing_schema_migrations: Vec<String>,
    pub release_count: u64,
    pub required_tables: BTreeMap<String, bool>,
    pub required_release_columns: BTreeMap<String, bool>,
    pub missing_tables: Vec<String>,
    pub missing_release_columns: Vec<String>,
    pub release_quality_columns_present: bool,
    pub migrated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplayValidationRequest {
    pub project_name: String,
    pub dbnum: u32,
    pub source_db_file: PathBuf,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub source_parquet_dir: PathBuf,
    pub current_parquet_dir: PathBuf,
    pub scene_tree_dir: Option<PathBuf>,
    pub require_scene_tree: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplayPathChecks {
    pub sesno_range_valid: bool,
    pub source_db_file_exists: bool,
    pub source_db_file_is_file: bool,
    pub source_parquet_dir_exists: bool,
    pub source_parquet_dir_is_dir: bool,
    pub current_parquet_dir_exists: bool,
    pub source_parquet_differs_from_current: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplayPackageEvidence {
    pub manifest_loaded: bool,
    pub package_error: Option<String>,
    pub rows_by_table: BTreeMap<String, u64>,
    pub instances_rows: u64,
    pub geo_instances_rows: u64,
    pub transforms_rows: u64,
    pub aabb_rows: u64,
    pub missing_mesh_geo_hashes: Option<u64>,
    pub missing_mesh_owner_refnos: Option<u64>,
    pub raw_missing_mesh_geo_hashes: Option<u64>,
    pub raw_missing_mesh_owner_refnos: Option<u64>,
    pub render_missing_mesh_geo_hashes: Option<u64>,
    pub render_missing_mesh_owner_refnos: Option<u64>,
    pub quarantined_mesh_geo_hashes: Option<u64>,
    pub quarantined_mesh_owner_refnos: Option<u64>,
    pub mesh_validation_present: bool,
    pub quarantine_counts_consistent: bool,
    pub mesh_assets_complete: bool,
    pub non_empty_visual_package: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplaySceneTreeEvidence {
    pub scene_tree_dir: PathBuf,
    pub tree_file: PathBuf,
    pub db_meta_info_file: PathBuf,
    pub tree_file_exists: bool,
    pub db_meta_info_exists: bool,
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplayValidationResponse {
    pub project_name: String,
    pub dbnum: u32,
    pub source_db_file: PathBuf,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub source_parquet_dir: PathBuf,
    pub current_parquet_dir: PathBuf,
    pub classification: String,
    pub ready_for_publish: bool,
    pub recommended_action: String,
    pub path_checks: ModelHistoryReplayPathChecks,
    pub package: ModelHistoryReplayPackageEvidence,
    pub scene_tree: Option<ModelHistoryReplaySceneTreeEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSourceObservationQuiescence {
    pub requested_window_ms: u64,
    pub checks_performed: u32,
    pub stable: bool,
    pub started_at: String,
    pub confirmed_at: String,
    pub primary_sha256_before: String,
    pub primary_sha256_after: String,
    pub primary_bytes_before: u64,
    pub primary_bytes_after: u64,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSourceObservationFileEvidence {
    pub role: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub modified_at: Option<String>,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSourceObservationManifest {
    pub manifest_version: String,
    pub observation_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub requested_sesno: Option<String>,
    pub resolved_sesno: Option<u32>,
    pub observed_at: String,
    pub primary: ModelSourceObservationFileEvidence,
    pub dependencies: Vec<ModelSourceObservationFileEvidence>,
    pub quiescence: ModelSourceObservationQuiescence,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSourceObservationResponse {
    pub project_name: String,
    pub dbnum: u32,
    pub source_db_file: PathBuf,
    pub requested_sesno: Option<String>,
    pub resolved_sesno: Option<u32>,
    pub ready_for_increment: bool,
    pub status: String,
    pub observation_manifest_path: PathBuf,
    pub observation_manifest_hash: String,
    pub observation: ModelSourceObservationManifest,
    pub recommended_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMissingMeshRepairRequest {
    pub project_name: String,
    pub dbnum: u32,
    pub report_file: PathBuf,
    pub mesh_root: PathBuf,
    pub limit: Option<usize>,
    pub dry_run: bool,
    pub retry_bad: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMissingMeshRepairRow {
    pub geo_hash: String,
    pub before_exists: bool,
    pub after_exists: bool,
    pub inst_geo_found: bool,
    pub has_param: bool,
    pub was_bad: bool,
    pub attempted: bool,
    pub generated_now: bool,
    pub still_missing: bool,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMissingMeshRepairResponse {
    pub project_name: String,
    pub dbnum: u32,
    pub report_file: PathBuf,
    pub mesh_root: PathBuf,
    pub dry_run: bool,
    pub retry_bad: bool,
    pub degraded_fradius_fallback_enabled: bool,
    pub degraded_fradius_fallback_log: Option<PathBuf>,
    pub degraded_fradius_fallback_rows: usize,
    pub requested_hashes: usize,
    pub limited: bool,
    pub invalid_hashes: usize,
    pub skipped_existing: usize,
    pub missing_inst_geo: usize,
    pub param_missing: usize,
    pub non_renderable_inputs: usize,
    pub self_intersecting_inputs: usize,
    pub bad_skipped: usize,
    pub attempted_hashes: usize,
    pub generated_hashes: usize,
    pub still_missing_hashes: usize,
    pub rows: Vec<ModelMissingMeshRepairRow>,
    pub recommended_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSceneTreeArtifactRestoreRequest {
    pub project_name: String,
    pub dbnum: u32,
    pub source_scene_tree_dir: PathBuf,
    pub target_scene_tree_dir: PathBuf,
    pub overwrite_tree: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSceneTreeArtifactRestoreResponse {
    pub project_name: String,
    pub dbnum: u32,
    pub source_scene_tree_dir: PathBuf,
    pub target_scene_tree_dir: PathBuf,
    pub source_tree_file: PathBuf,
    pub target_tree_file: PathBuf,
    pub source_db_meta_info_file: PathBuf,
    pub target_db_meta_info_file: PathBuf,
    pub dry_run: bool,
    pub overwrite_tree: bool,
    pub source_tree_bytes: u64,
    pub source_tree_sha256: String,
    pub target_tree_sha256_before: Option<String>,
    pub target_tree_sha256_after: Option<String>,
    pub tree_would_copy: bool,
    pub tree_copied: bool,
    pub db_meta_would_write: bool,
    pub db_meta_written: bool,
    pub source_latest_sesno: Option<u64>,
    pub target_latest_sesno_before: Option<u64>,
    pub target_latest_sesno_after: Option<u64>,
    #[serde(default)]
    pub source_ref0s: Vec<u64>,
    #[serde(default)]
    pub target_ref0s_before: Vec<u64>,
    #[serde(default)]
    pub target_ref0s_after: Vec<u64>,
    #[serde(default)]
    pub added_ref0s: Vec<u64>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub recommended_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplayPrepareRequest {
    pub project_name: String,
    pub release_id: String,
    pub release_label: Option<String>,
    pub baseline_release_id: Option<String>,
    pub branch_id: String,
    pub parent_release_id: Option<String>,
    pub dbnum: u32,
    pub baseline_dbnums: Vec<u32>,
    pub source_db_file: PathBuf,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub base_config_arg: String,
    pub baseline_config_arg: Option<PathBuf>,
    pub replay_config_arg: PathBuf,
    pub replay_surreal_ns: Option<String>,
    pub replay_output_root: Option<PathBuf>,
    pub current_parquet_dir: PathBuf,
    pub baseline_source_confirmed_at_from_sesno: bool,
    pub force: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplayCommands {
    pub baseline_parse: String,
    pub baseline_generate: String,
    pub baseline_register: String,
    pub generate: String,
    pub publish: String,
    pub baseline_parse_argv: Vec<String>,
    pub baseline_generate_argv: Vec<String>,
    pub baseline_register_argv: Vec<String>,
    pub generate_argv: Vec<String>,
    pub publish_argv: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplaySafetyChecks {
    pub replay_namespace_differs_from_current: bool,
    pub replay_output_root_differs_from_current: bool,
    pub replay_project_output_differs_from_current: bool,
    pub replay_parquet_differs_from_current: bool,
    pub replay_config_differs_from_base_config: bool,
    pub generation_is_external_process: bool,
    pub materialize_assets_in_publish_command: bool,
    pub baseline_config_requests_save_db: bool,
    pub baseline_binary_supports_surreal_save: bool,
    pub baseline_parse_uses_current_file_state: bool,
    pub baseline_target_sesno_reconstruction_supported: bool,
    pub baseline_source_must_already_match_from_sesno: bool,
    pub baseline_source_confirmed_at_from_sesno: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHistoryReplayPrepareResponse {
    pub release_id: String,
    pub release_label: Option<String>,
    pub baseline_release_id: String,
    pub branch_id: String,
    pub parent_release_id: Option<String>,
    pub target_parent_release_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub baseline_dbnums: Vec<u32>,
    pub source_db_file: PathBuf,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub current_surreal_ns: String,
    pub replay_surreal_ns: String,
    pub current_output_root: PathBuf,
    pub current_project_output_dir: PathBuf,
    pub current_parquet_dir: PathBuf,
    pub replay_output_root: PathBuf,
    pub replay_project_output_dir: PathBuf,
    pub replay_parquet_dir: PathBuf,
    pub base_config_arg: String,
    pub base_config_path: PathBuf,
    pub baseline_config_arg: PathBuf,
    pub baseline_config_path: PathBuf,
    pub replay_config_arg: PathBuf,
    pub replay_config_path: PathBuf,
    pub written: bool,
    pub overwritten: bool,
    pub baseline_plan_warning: String,
    pub commands: ModelHistoryReplayCommands,
    pub safety_checks: ModelHistoryReplaySafetyChecks,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelPhysicalBaselineSnapshotRequest {
    pub project_name: String,
    pub snapshot_id: String,
    pub dbnum: u32,
    pub source_db_file: PathBuf,
    pub baseline_dbnums: Vec<u32>,
    pub base_config_arg: String,
    pub config_arg: Option<PathBuf>,
    pub snapshot_root: Option<PathBuf>,
    pub output_root: Option<PathBuf>,
    pub surreal_ns: Option<String>,
    pub copy_files: bool,
    pub force: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelPhysicalBaselineSnapshotCommands {
    pub parse: String,
    pub parse_argv: Vec<String>,
    pub generate_full_model: String,
    pub generate_full_model_argv: Vec<String>,
    pub prepare_history_replay_hint: String,
    pub prepare_history_replay_hint_argv: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelPhysicalBaselineSnapshotSafetyChecks {
    pub source_db_file_exists: bool,
    pub source_db_file_is_file: bool,
    pub source_db_file_matches_dbnum: bool,
    pub snapshot_project_differs_from_source: bool,
    pub snapshot_output_differs_from_current: bool,
    pub config_differs_from_base_config: bool,
    pub original_project_not_modified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelPhysicalBaselineStateManifest {
    pub manifest_version: String,
    pub snapshot_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub baseline_dbnums: Vec<u32>,
    pub source_db_file: PathBuf,
    pub source_db_sha256: String,
    pub replacement_db_file: PathBuf,
    pub replacement_db_sha256: String,
    pub source_db_type: String,
    pub source_db_session_page: u32,
    pub source_db_latest_sesno: u32,
    pub snapshot_root: PathBuf,
    pub snapshot_project_dir: PathBuf,
    pub snapshot_db_dir: PathBuf,
    pub output_root: PathBuf,
    pub config_path: PathBuf,
    pub surreal_ns: String,
    pub file_count: usize,
    pub hardlinked_count: usize,
    pub copied_count: usize,
    pub copy_mode: String,
    pub safety_checks: ModelPhysicalBaselineSnapshotSafetyChecks,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelBaselineStateValidationRequest {
    pub project_name: String,
    pub dbnum: Option<u32>,
    pub from_sesno: Option<u32>,
    pub baseline_state_manifest_path: PathBuf,
    pub baseline_state_manifest_hash: Option<String>,
    pub scene_tree_dir: Option<PathBuf>,
    pub require_scene_tree: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelBaselineStateValidationResponse {
    pub project_name: String,
    pub dbnum: u32,
    pub from_sesno: Option<u32>,
    pub ready: bool,
    pub manifest_version: String,
    pub snapshot_id: String,
    pub baseline_dbnums: Vec<u32>,
    pub baseline_state_manifest_path: PathBuf,
    pub baseline_state_manifest_hash: String,
    pub source_db_file: PathBuf,
    pub source_db_sha256: String,
    pub replacement_db_file: PathBuf,
    pub replacement_db_sha256: String,
    pub source_db_latest_sesno: u32,
    pub snapshot_root: PathBuf,
    pub config_path: PathBuf,
    pub output_root: PathBuf,
    pub surreal_ns: String,
    pub safety_checks: ModelPhysicalBaselineSnapshotSafetyChecks,
    pub scene_tree: ModelHistoryReplaySceneTreeEvidence,
    pub recommended_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelPhysicalBaselineSnapshotResponse {
    pub project_name: String,
    pub snapshot_id: String,
    pub dbnum: u32,
    pub baseline_dbnums: Vec<u32>,
    pub source_db_file: PathBuf,
    pub source_project_dir: PathBuf,
    pub source_db_dir: PathBuf,
    pub active_db_file: PathBuf,
    pub snapshot_root: PathBuf,
    pub snapshot_project_parent: PathBuf,
    pub snapshot_project_dir: PathBuf,
    pub snapshot_db_dir: PathBuf,
    pub replacement_db_file: PathBuf,
    pub source_db_latest_sesno: u32,
    pub base_config_arg: String,
    pub base_config_path: PathBuf,
    pub config_arg: PathBuf,
    pub config_path: PathBuf,
    pub output_root: PathBuf,
    pub surreal_ns: String,
    pub file_count: usize,
    pub hardlinked_count: usize,
    pub copied_count: usize,
    pub replaced_target: bool,
    pub overwritten: bool,
    pub copy_mode: String,
    pub written: bool,
    pub baseline_state_manifest_path: PathBuf,
    pub baseline_state_manifest_hash: String,
    pub commands: ModelPhysicalBaselineSnapshotCommands,
    pub safety_checks: ModelPhysicalBaselineSnapshotSafetyChecks,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseRecord {
    pub release_id: String,
    pub project_name: String,
    pub branch_id: String,
    pub release_lifecycle: ModelReleaseLifecycle,
    pub release_quality: ModelReleaseQuality,
    pub release_quality_reason: Option<String>,
    #[serde(default)]
    pub validation_flags: Vec<String>,
    pub spec_info_fallback_count: Option<u64>,
    /// Legacy lifecycle storage/readback field. New code should use
    /// `release_lifecycle` for workflow gates and `release_quality` for visual
    /// completeness/quarantine semantics.
    pub release_status: ModelReleaseStatus,
    pub release_label: Option<String>,
    pub dbnum: u32,
    pub source_package_dir: PathBuf,
    pub immutable_package_dir: PathBuf,
    pub package_hash: String,
    pub derivation_type: String,
    pub created_at: Option<String>,
    pub registered_at: String,
    pub rows_by_table: BTreeMap<String, u64>,
    pub source_manifest_path: Option<PathBuf>,
    pub source_manifest_hash: Option<String>,
    pub baseline_state_manifest_path: Option<PathBuf>,
    pub baseline_state_manifest_hash: Option<String>,
    pub generation_job_id: Option<String>,
    pub asset_manifest_path: Option<PathBuf>,
    pub asset_manifest_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelReleaseStatus {
    Staged,
    Validating,
    AssetsMaterialized,
    Indexed,
    Published,
    Failed,
    Degraded,
    Quarantined,
    PatchOnly,
}

impl ModelReleaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Validating => "validating",
            Self::AssetsMaterialized => "assets_materialized",
            Self::Indexed => "indexed",
            Self::Published => "published",
            Self::Failed => "failed",
            Self::Degraded => "degraded",
            Self::Quarantined => "quarantined",
            Self::PatchOnly => "patch_only",
        }
    }

    pub fn from_storage(value: Option<String>) -> Self {
        match value
            .as_deref()
            .unwrap_or("published")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "staged" => Self::Staged,
            "validating" => Self::Validating,
            "assets_materialized" => Self::AssetsMaterialized,
            "indexed" => Self::Indexed,
            "published" => Self::Published,
            "failed" => Self::Failed,
            "degraded" => Self::Degraded,
            "quarantined" => Self::Quarantined,
            "patch_only" => Self::PatchOnly,
            _ => Self::Failed,
        }
    }

    pub fn lifecycle(&self) -> ModelReleaseLifecycle {
        match self {
            Self::Staged => ModelReleaseLifecycle::Staged,
            Self::Validating => ModelReleaseLifecycle::Validating,
            Self::AssetsMaterialized => ModelReleaseLifecycle::AssetsMaterialized,
            Self::Indexed => ModelReleaseLifecycle::Indexed,
            Self::Published | Self::Degraded | Self::Quarantined | Self::PatchOnly => {
                ModelReleaseLifecycle::Published
            }
            Self::Failed => ModelReleaseLifecycle::Failed,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelReleaseLifecycle {
    Staged,
    Validating,
    AssetsMaterialized,
    Indexed,
    Published,
    Failed,
}

impl ModelReleaseLifecycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Validating => "validating",
            Self::AssetsMaterialized => "assets_materialized",
            Self::Indexed => "indexed",
            Self::Published => "published",
            Self::Failed => "failed",
        }
    }

    pub fn from_storage(value: Option<String>, legacy_status: &ModelReleaseStatus) -> Self {
        match value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("staged") => Self::Staged,
            Some("validating") => Self::Validating,
            Some("assets_materialized") => Self::AssetsMaterialized,
            Some("indexed") => Self::Indexed,
            Some("published") => Self::Published,
            Some("failed") => Self::Failed,
            _ => legacy_status.lifecycle(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelReleaseQuality {
    CompleteVisual,
    QuarantinedVisual,
    DegradedVisual,
    PatchOnly,
    NonVisual,
}

impl ModelReleaseQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CompleteVisual => "complete_visual",
            Self::QuarantinedVisual => "quarantined_visual",
            Self::DegradedVisual => "degraded_visual",
            Self::PatchOnly => "patch_only",
            Self::NonVisual => "non_visual",
        }
    }

    pub fn from_storage_or_infer(
        value: Option<String>,
        legacy_status: &ModelReleaseStatus,
        release_id: &str,
        release_label: Option<&str>,
        derivation_type: &str,
        instances_rows: Option<u64>,
        geo_instances_rows: Option<u64>,
    ) -> Self {
        match value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("complete_visual") => return Self::CompleteVisual,
            Some("quarantined_visual") | Some("quarantined") => {
                return Self::QuarantinedVisual;
            }
            Some("degraded_visual") | Some("degraded") => return Self::DegradedVisual,
            Some("patch_only") => return Self::PatchOnly,
            Some("non_visual") => return Self::NonVisual,
            _ => {}
        }

        match legacy_status {
            ModelReleaseStatus::Quarantined => return Self::QuarantinedVisual,
            ModelReleaseStatus::Degraded => return Self::DegradedVisual,
            ModelReleaseStatus::PatchOnly => return Self::PatchOnly,
            _ => {}
        }

        let marker = format!(
            "{} {} {}",
            release_id,
            release_label.unwrap_or_default(),
            derivation_type
        )
        .to_ascii_lowercase();
        if marker.contains("quarantine") || marker.contains("quarantined") {
            Self::QuarantinedVisual
        } else if marker.contains("patch_only") || marker.contains("patch-only") {
            Self::PatchOnly
        } else if marker.contains("degraded")
            || marker.contains("partial")
            || marker.contains("smoke")
        {
            Self::DegradedVisual
        } else if instances_rows.unwrap_or_default() == 0
            || geo_instances_rows.unwrap_or_default() == 0
        {
            Self::NonVisual
        } else {
            Self::CompleteVisual
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelReleaseRegistrationStatus {
    Created,
    AlreadyExists,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseRegistration {
    pub status: ModelReleaseRegistrationStatus,
    pub release: ModelReleaseRecord,
    pub files: Vec<ModelReleaseFile>,
    pub parent_release_id: Option<String>,
    pub ducklake_metadata_path: PathBuf,
    pub ducklake_data_path: PathBuf,
    pub component_index: Option<ModelComponentSnapshotStats>,
    pub unit_index: Option<ModelUnitIndexStats>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseListResponse {
    pub project_name: Option<String>,
    pub releases: Vec<ModelReleaseRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseStatusEvent {
    pub release_id: String,
    pub release_status: ModelReleaseStatus,
    pub reason: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseEventsResponse {
    pub release: ModelReleaseRecord,
    pub events: Vec<ModelReleaseStatusEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseReconcileReport {
    pub release: ModelReleaseRecord,
    pub previous_status: ModelReleaseStatus,
    pub previous_lifecycle: ModelReleaseLifecycle,
    pub current_status: ModelReleaseStatus,
    pub current_lifecycle: ModelReleaseLifecycle,
    pub publishable: bool,
    pub applied: bool,
    pub action_taken: String,
    pub recommended_action: String,
    pub package_dir_exists: bool,
    pub package_manifest_exists: bool,
    pub release_sidecar_path: PathBuf,
    pub release_sidecar_exists: bool,
    pub release_sidecar_hash: Option<String>,
    pub missing_required_files: Vec<String>,
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    pub component_index: Option<ModelComponentSnapshotStats>,
    pub mesh_asset_index: Option<ModelReleaseMeshAssetIndexStats>,
    pub unit_index: Option<ModelUnitIndexStats>,
    pub events: Vec<ModelReleaseStatusEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelComponentSnapshotStats {
    pub release_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub hash_version: String,
    pub component_count: u64,
    pub distinct_component_hashes: u64,
    pub indexed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelComponentDiffSummary {
    pub added: u64,
    pub deleted: u64,
    pub changed: u64,
    pub unchanged: u64,
    pub total_old: u64,
    pub total_new: u64,
    pub emitted: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelComponentDiffRow {
    pub change_type: String,
    pub component_key: String,
    pub dbnum: u32,
    pub refno_str: Option<String>,
    pub refno_u64: Option<u64>,
    pub noun: Option<String>,
    pub old_component_hash: Option<String>,
    pub new_component_hash: Option<String>,
    pub old_owner_refno_str: Option<String>,
    pub new_owner_refno_str: Option<String>,
    pub old_cata_hash: Option<String>,
    pub new_cata_hash: Option<String>,
    pub old_trans_hash: Option<String>,
    pub new_trans_hash: Option<String>,
    pub old_aabb_hash: Option<String>,
    pub new_aabb_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelComponentDiffResponse {
    pub from_release_id: String,
    pub to_release_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub from_index: ModelComponentSnapshotStats,
    pub to_index: ModelComponentSnapshotStats,
    pub summary: ModelComponentDiffSummary,
    pub rows: Vec<ModelComponentDiffRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseReadinessEvidence {
    pub release_id: String,
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ModelReleaseRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelReleaseLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<ModelReleaseQuality>,
    #[serde(default)]
    pub validation_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_state_manifest_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_state_manifest_hash: Option<String>,
    pub spec_info_manifest_evidence_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_info_manifest_fallback_count: Option<u64>,
    pub published: bool,
    pub complete_visual: bool,
    pub component_index_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_index: Option<ModelComponentSnapshotStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_index_current_count: Option<u64>,
    pub mesh_assets_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_asset_index: Option<ModelReleaseMeshAssetIndexStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_local_asset_violation_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_index: Option<ModelUnitIndexStats>,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub recommended_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleasePairReadinessResponse {
    pub from_release_id: String,
    pub to_release_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dbnum: Option<u32>,
    pub classification: String,
    pub production_ready: bool,
    pub production_comparison_allowed: bool,
    pub both_releases_exist: bool,
    pub same_project: bool,
    pub same_dbnum: bool,
    pub both_published: bool,
    pub both_complete_visual: bool,
    pub component_indexes_ready: bool,
    pub mesh_assets_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<ModelComponentDiffSummary>,
    pub from: ModelReleaseReadinessEvidence,
    pub to: ModelReleaseReadinessEvidence,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub recommended_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseSceneAabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseSceneGeometry {
    pub geo_index: u32,
    pub geo_hash: String,
    pub geo_trans_hash: Option<String>,
    pub geo_matrix: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_asset: Option<ModelReleaseSceneMeshAssetEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseSceneMeshAssetEvidence {
    pub geo_hash: String,
    pub builtin: bool,
    pub exists: bool,
    pub mesh_relative_path: Option<String>,
    pub mesh_absolute_path: Option<PathBuf>,
    pub mesh_url: Option<String>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub glb_readable: Option<bool>,
    pub glb_validation_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseSceneComponent {
    pub component_key: String,
    pub refno_str: String,
    pub refno_u64: u64,
    pub noun: String,
    pub owner_refno_str: Option<String>,
    pub owner_refno_u64: Option<u64>,
    pub owner_noun: Option<String>,
    pub cata_hash: Option<String>,
    pub trans_hash: Option<String>,
    pub aabb_hash: Option<String>,
    pub spec_value: i64,
    pub has_neg: bool,
    pub component_hash: Option<String>,
    pub instance_matrix: Option<Vec<f64>>,
    pub aabb: Option<ModelReleaseSceneAabb>,
    pub geometries: Vec<ModelReleaseSceneGeometry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseSceneResponse {
    pub release: ModelReleaseRecord,
    pub row_counts: BTreeMap<String, u64>,
    pub component_count: usize,
    pub geometry_count: usize,
    pub total_components: u64,
    pub offset: usize,
    pub limit: usize,
    pub next_offset: Option<usize>,
    pub has_more: bool,
    pub truncated: bool,
    pub components: Vec<ModelReleaseSceneComponent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseMeshAsset {
    pub release_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub lod_tag: String,
    pub geo_hash: String,
    pub builtin: bool,
    pub exists: bool,
    pub mesh_relative_path: Option<String>,
    pub mesh_absolute_path: Option<PathBuf>,
    pub mesh_url: Option<String>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub glb_readable: Option<bool>,
    pub glb_validation_error: Option<String>,
    pub indexed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseMeshAssetIndexStats {
    pub release_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub lod_tag: String,
    pub geo_hash_count: u64,
    pub present_count: u64,
    pub missing_count: u64,
    pub builtin_count: u64,
    pub total_bytes: u64,
    pub glb_checked_count: Option<u64>,
    pub glb_readable_count: Option<u64>,
    pub glb_unreadable_count: Option<u64>,
    pub asset_index_hash: String,
    pub manifest_path: PathBuf,
    pub indexed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseMeshAssetIndexResponse {
    pub stats: ModelReleaseMeshAssetIndexStats,
    pub assets: Vec<ModelReleaseMeshAsset>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelUnitIndexStats {
    pub release_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub hash_version: String,
    pub rule_set_hash: String,
    pub unit_count: u64,
    pub member_count: u64,
    pub unresolved_member_count: u64,
    pub indexed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelUnitDiffSummary {
    pub added: u64,
    pub deleted: u64,
    pub changed: u64,
    pub unchanged: u64,
    pub total_old: u64,
    pub total_new: u64,
    pub emitted: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelUnitDiffRow {
    pub change_type: String,
    pub unit_key: String,
    pub unit_noun: String,
    pub unit_refno_str: Option<String>,
    pub unit_refno_u64: Option<u64>,
    pub old_unit_version_id: Option<String>,
    pub new_unit_version_id: Option<String>,
    pub old_aggregate_hash: Option<String>,
    pub new_aggregate_hash: Option<String>,
    pub old_member_count: Option<u64>,
    pub new_member_count: Option<u64>,
    pub old_unresolved_member_count: Option<u64>,
    pub new_unresolved_member_count: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelUnitDiffResponse {
    pub from_release_id: String,
    pub to_release_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub from_index: ModelUnitIndexStats,
    pub to_index: ModelUnitIndexStats,
    pub summary: ModelUnitDiffSummary,
    pub rows: Vec<ModelUnitDiffRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelComponentUnitImpactSummary {
    pub component_changes: u64,
    pub impacted_units: u64,
    pub emitted: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelComponentUnitImpactRow {
    pub impact_kind: String,
    pub rule_id: String,
    pub component_key: String,
    pub dbnum: u32,
    pub refno_str: Option<String>,
    pub refno_u64: Option<u64>,
    pub noun: Option<String>,
    pub change_type: String,
    pub unit_key: String,
    pub unit_noun: String,
    pub unit_refno_str: Option<String>,
    pub unit_refno_u64: Option<u64>,
    pub old_unit_version_id: Option<String>,
    pub new_unit_version_id: Option<String>,
    pub old_aggregate_hash: Option<String>,
    pub new_aggregate_hash: Option<String>,
    pub old_component_hash: Option<String>,
    pub new_component_hash: Option<String>,
    pub old_membership_kind: Option<String>,
    pub new_membership_kind: Option<String>,
    pub dependency_path_json: String,
    pub evidence_json: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelComponentUnitImpactResponse {
    pub from_release_id: String,
    pub to_release_id: String,
    pub project_name: String,
    pub dbnum: u32,
    pub from_unit_index: ModelUnitIndexStats,
    pub to_unit_index: ModelUnitIndexStats,
    pub component_diff_summary: ModelComponentDiffSummary,
    pub summary: ModelComponentUnitImpactSummary,
    pub rows: Vec<ModelComponentUnitImpactRow>,
}

/// specs/023：交付单元版本主键 `(dbnum, unit_refno_u64, sesno)`。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnitVersionV2Record {
    pub dbnum: u32,
    pub unit_refno_u64: u64,
    pub sesno: u32,
    pub project_name: String,
    pub unit_refno_str: Option<String>,
    pub unit_noun: Option<String>,
    pub unit_key: Option<String>,
    pub aggregate_hash: String,
    pub hash_version: String,
    pub rule_set_hash: Option<String>,
    pub member_count: u64,
    pub unresolved_member_count: u64,
    pub member_signature: Option<String>,
    pub package_relpath: Option<String>,
    pub status: Option<String>,
    pub label: Option<String>,
    /// 过渡期只读兼容；新写入应保持 None。
    pub legacy_release_id: Option<String>,
    pub indexed_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpsertUnitVersionV2Request {
    pub dbnum: u32,
    pub unit_refno_u64: u64,
    pub sesno: u32,
    pub project_name: String,
    pub unit_refno_str: Option<String>,
    pub unit_noun: Option<String>,
    pub unit_key: Option<String>,
    pub aggregate_hash: String,
    pub hash_version: String,
    pub rule_set_hash: Option<String>,
    pub member_count: u64,
    pub unresolved_member_count: u64,
    pub member_signature: Option<String>,
    pub package_relpath: Option<String>,
    pub status: Option<String>,
    pub label: Option<String>,
    pub legacy_release_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum UpsertUnitVersionV2Outcome {
    Inserted { record: UnitVersionV2Record },
    Unchanged { record: UnitVersionV2Record },
}

/// specs/023：单元成员行；`sesno` 为所属单元版本号（= max(member_sesno)）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnitMembershipV2Record {
    pub dbnum: u32,
    pub unit_refno_u64: u64,
    pub sesno: u32,
    pub member_refno_u64: u64,
    pub project_name: String,
    pub unit_refno_str: Option<String>,
    pub unit_noun: Option<String>,
    pub unit_key: Option<String>,
    pub member_refno_str: Option<String>,
    pub member_noun: Option<String>,
    pub member_sesno: u32,
    pub component_hash: Option<String>,
    pub membership_kind: Option<String>,
    pub path_confidence: Option<f64>,
    pub unresolved_reason: Option<String>,
    pub membership_hash: Option<String>,
    pub hash_version: Option<String>,
    pub legacy_release_id: Option<String>,
    pub indexed_at: String,
}

/// 单元 sesno = 成员组件 sesno 的最大值（specs/023 ADR）。
pub fn unit_sesno_from_member_sesnos(member_sesnos: impl IntoIterator<Item = u32>) -> Option<u32> {
    member_sesnos.into_iter().max()
}

/// specs/023：当 CLI 未传 `--release-id` 时，用 sesno 生成**遗留批次元数据**别名。
/// 这不是版本身份真相源；真相键仍是 `(dbnum, refno, sesno)`。
pub fn legacy_batch_id_for_sesno(dbnum: u32, sesno: u32) -> String {
    format!("db{dbnum}-s{sesno}")
}

/// 解析 `legacy_batch_id_for_sesno` 生成的别名：`db{dbnum}-s{sesno}`。
/// 用于 C3 dual-read：v2 未命中时从遗留 `unit_versions.release_id` 回退。
pub fn parse_legacy_batch_id(release_id: &str) -> Option<(u32, u32)> {
    let rest = release_id.trim().strip_prefix("db")?;
    let (dbnum_str, sesno_str) = rest.split_once("-s")?;
    let dbnum = dbnum_str.parse::<u32>().ok()?;
    let sesno = sesno_str.parse::<u32>().ok()?;
    Some((dbnum, sesno))
}

/// specs/023 E2：挂在 `(dbnum, refno, sesno)` 上的单元状态事件（非 release_id）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnitVersionStatusEventV2 {
    pub dbnum: u32,
    pub unit_refno_u64: u64,
    pub sesno: u32,
    pub status: String,
    pub reason: Option<String>,
    pub created_at: String,
}

/// `model-version unit-v2-smoke` 输出（specs/023 B5）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnitVersionV2SmokeReport {
    pub ok: bool,
    pub work_dir: PathBuf,
    pub derived_sesno: u32,
    pub expected_sesno: u32,
    pub first_outcome: String,
    pub second_outcome: String,
    pub conflict_rejected: bool,
    pub listed_count: usize,
}

/// specs/023 C2：按 sesno 对比单元版本。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnitVersionV2DiffRow {
    pub change_type: String,
    pub unit_refno_u64: u64,
    pub unit_refno_str: Option<String>,
    pub unit_noun: Option<String>,
    pub unit_key: Option<String>,
    pub from_sesno: Option<u32>,
    pub to_sesno: Option<u32>,
    pub old_aggregate_hash: Option<String>,
    pub new_aggregate_hash: Option<String>,
    pub old_member_count: Option<u64>,
    pub new_member_count: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnitVersionV2DiffResponse {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub unit_refno_u64: Option<u64>,
    pub summary: ModelUnitDiffSummary,
    pub rows: Vec<UnitVersionV2DiffRow>,
}
