# SigMap Query Context
Generated: 2026-06-17T10:20:07.951Z

## .worktrees\pe-transform-backends\src\web_server\sqlite_spatial_api.rs
```
pub struct SqliteSpatialQueryParams
pub struct SpatialQueryResult
pub struct SpatialQueryResultItem
pub struct AabbDto
pub struct Vec3Dto
pub struct SpatialStatsResult
pub async fn api_sqlite_spatial_query(Query(params) → Json<SpatialQueryResult>
pub async fn api_sqlite_spatial_stats() → Json<SpatialStatsResult>
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

## src\web_server\mod.rs
```
pub struct AppState
pub struct TaskManager
pub struct ConfigManager
pub struct TaskQuery
pub struct CreateTaskRequest
pub struct UpdateConfigRequest
impl AppState
pub fn new() → Self
impl ConfigManager
pub fn add_template(&mut self, name: &str, config: DatabaseConfig)
pub async fn start_web_server(port: u16) → anyhow::Result<()>
pub async fn start_web_server_with_config(port: u16, config_file: Option<&str>,) → anyhow::Result<()>
```

## .worktrees\pe-transform-backends\docs\plans\2026-04-28-aveva-plant-sample-deployment-test-plan.md
```
h1 AvevaPlantSample 站点部署 + DESI 解析端到端测试与修复（2026-04-28）
h2 0. 背景
h2 1. 根因定位
h3 1.1 关键证据
h3 1.2 配置文件已确认正确
h3 1.3 候选根因
h3 1.4 反向佐证
h2 2. 方案选择
h3 2.1 候选方案对比
h3 2.2 修复要点（rs-core/src/options.rs）
h2 3. 执行计划
h3 Step 1 — 制定计划文件 *(本文件)*
h3 Step 2 — 实施 rs-core 最小修复
h3 Step 3 — 重新编译 web_server 并热重启
h3 Step 4 — 通过浏览器 admin UI 重新触发 AvevaPlantSample 解析
h3 Step 5 — 通过浏览器点击「启动」按钮启动子站点
h3 Step 6 — 在浏览器打开 plant3d-web 前端
h3 Step 7 — 汇报 + 进度落档
h2 4. 风险与回退
h2 5. 不做（Out of Scope）
```
