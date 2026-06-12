# SigMap Query Context
Generated: 2026-06-12T02:57:05.671Z

## .worktrees\pe-transform-backends\src\web_server\managed_project_sites.rs
```
pub struct RuntimeUpdate
pub struct StopSiteResult
pub struct TailLogResponse
pub fn ensure_schema() → Result<()>
pub fn get_site(site_id: &str) → Result<Option<ManagedProjec...
pub fn list_sites() → Result<Vec<ManagedProjectSi...
pub fn create_site(req: CreateManagedSiteRequest) → Result<ManagedProjectSite>
pub fn preview_parse_plan(req: PreviewManagedSiteParsePlanRequest) → Result<ManagedSiteParsePlan>
pub fn update_site(site_id: &str, req: UpdateManagedSiteRequest) → Result<ManagedProjectSite>
pub fn update_runtime(site_id: &str, update: RuntimeUpdate) → Result<()>
pub fn resource_summary() → Result<AdminResourceSummary>
pub async fn start_site(site_id: String) → Result<()>
pub async fn parse_site(site_id: String) → Result<()>
pub async fn restart_site(site_id: &str) → Result<()>
pub async fn stop_site(site_id: &str) → Result<StopSiteResult>
pub fn delete_site(site_id: &str) → Result<bool>
pub fn runtime_status(site_id: &str) → Result<ManagedSiteRuntimeSt...
pub fn tail_log(site_id: &str, kind: &str, limit: usize) → Result<TailLogResponse>
pub fn full_log_path(site_id: &str, kind: &str) → Result<PathBuf>
pub fn logs(site_id: &str) → Result<ManagedSiteLogsRespo...
```

## src\web_server\wizard_handlers.rs
```
pub struct DatabaseFileInfo
pub struct DatabaseFileScanRequest
pub struct DatabaseFileScanResult
pub struct BrowseDirectoryRequest
pub struct DirectoryEntry
pub struct BrowseDirectoryResponse
pub async fn scan_directory(State(_state) → Result<Json<DirectoryScanRe...
pub async fn list_projects(State(_state) → Result<Json<Vec<ProjectInfo...
pub async fn create_wizard_task(State(state) → Result<Json<TaskInfo>, (Sta...
pub async fn get_wizard_templates(State(_state) → Result<Json<Vec<TaskTemplat...
pub async fn scan_database_files(State(_state) → Result<Json<DatabaseFileSca...
pub fn open_deployment_sites_sqlite() → Result<rusqlite::Connection...
pub fn persist_task_progress_to_sqlite(task: &TaskInfo) → Result<(), Box<dyn std::err...
pub async fn browse_directory(Query(request) → Result<Json<BrowseDirectory...
pub fn delete_deployment_site_from_sqlite(site_id: &str) → Result<(), Box<dyn std::err...
pub fn load_wizard_config_by_task_id(task_id: &str) → Option<DataParsingWizardCon...
pub fn restore_tasks_from_sqlite() → Vec<TaskInfo>
pub fn load_deployment_sites_from_sqlite() → Result<Vec<serde_json::Valu...
pub fn load_deployment_site_by_id_from_sqlite(site_id: &str,) → Result<Option<serde_json::V...
pub fn update_deployment_site_health(site_id: &str, status: &str, timestamp: &str,) → Result<(), Box<dyn std::err...
```

## .worktrees\model-persistence-trait\src\cli_modes.rs
```
pub struct ExportConfig
pub struct CanonicalParquetValidationReport
pub struct RoomComputeCliConfig
pub struct SpatialQueryVerifyResultItem
pub struct SpatialQueryVerifySnapshot
impl ExportConfig
impl ExportConfig
pub fn new(refnos_str: Vec<String>) → Self
pub fn with_output_path(mut self, output_path: Option<String>) → Self
pub fn with_filter_nouns(mut self, filter_nouns: Option<Vec<String>>) → Self
pub fn with_include_descendants(mut self, include_descendants: bool) → Self
pub fn with_unit_conversion(mut self, source_unit: &str, target_unit: &str) → Self
pub fn with_verbose(mut self, verbose: bool) → Self
pub fn with_regenerate_plant_mesh(mut self, regenerate_plant_mesh: bool) → Self
pub fn with_run_all_dbnos(mut self, run_all_dbnos: bool) → Self
impl RoomVerifySummary
impl ScopedEnvVar
impl ScopedEnvVar
impl RoomComputeCliReport
pub fn validate_canonical_parquet_writer_mode(output_dir: &Path, project_name: &str, dbnum: u32, batch_id: u64,) → Result<CanonicalParquetVali...
```

## src\web_server\handlers.rs
```
pub struct CreateBatchTaskRequest
pub struct CreateTaskTemplateRequest
pub struct SshOptions
pub struct SurrealControlRequest
pub struct SurrealTestRequest
pub struct SqliteSpatialQuery
pub struct DeploymentSiteBrowseQuery
pub struct GetInstancesRequest
pub struct ModelDataResponse
pub struct SurrealStatusQuery
pub struct TraySupportsDetectRequest
pub struct SctnTestRequest
pub struct DatabaseConnectionStatus
pub struct DatabaseConnectionConfig
pub struct StartupScript
pub struct DbConnCheckQuery
pub struct StartDatabaseRequest
pub struct ExportRequest
pub struct ExportResponse
pub struct ExportStatusResponse
```

## .worktrees\model-persistence-trait\src\fast_model\gen_model\transform_cache.rs
```
pub struct TransformCacheManager
impl TransformCacheManager
pub fn new() → Self
pub fn get_world_transform(&self, dbnum: u32, refno: RefnoEnum) → Option<Transform>
pub fn get_local_transform(&self, dbnum: u32, refno: RefnoEnum) → Option<Transform>
pub fn remove(&self, dbnum: u32, refno: RefnoEnum)
pub fn insert_world_transform(&self, dbnum: u32, refno: RefnoEnum, world: Transform)
pub fn insert_local_transform(&self, dbnum: u32, refno: RefnoEnum, local: Transform)
pub fn is_dbnum_loaded(&self, dbnum: u32) → bool
pub fn load_dbnum_snapshot(&self, dbnum: u32, snapshot: LoadedTransformDbnum)
pub fn init_global_transform_cache()
pub fn prime_global_transform_cache_from_pe_entries(entries: &[PeTransformEntry]) → usize
pub fn clear_global_transform_cache() → usize
pub fn clear_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) → usize
pub fn pin_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) → usize
pub fn release_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) → usize
pub async fn get_world_transform_cache_first(db_option: Option<&DbOptionExt>, refno: RefnoEnum,) → anyhow::Result<Option<Trans...
pub async fn get_local_transform_cache_first(db_option: Option<&DbOptionExt>, refno: RefnoEnum,) → anyhow::Result<Option<Trans...
pub async fn get_world_transforms_cache_only_batch(db_option: &DbOptionExt, refnos: &[RefnoEnum],) → anyhow::Result<HashMap<Refn...
pub async fn get_local_transforms_cache_only_batch(db_option: &DbOptionExt, refnos: &[RefnoEnum],) → anyhow::Result<HashMap<Refn...
```
