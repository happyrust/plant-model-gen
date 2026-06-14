export type ManagedSiteStatus =
  | 'Draft'
  | 'Parsed'
  | 'Starting'
  | 'Running'
  | 'Stopping'
  | 'Stopped'
  | 'Failed'

export type ManagedSiteParseStatus =
  | 'Pending'
  | 'Running'
  | 'Parsed'
  | 'Failed'

export type ProjectRole = 'design' | 'library'

/** 站点内的单个工程条目（多工程合并站点的最小单元） */
export interface SiteProject {
  path: string
  name: string
  role: ProjectRole
  is_primary: boolean
  sort_order: number
}

/** Phase 3 扫描 API 返回的候选工程 */
export interface ScannedProject extends SiteProject {
  dbnums: number[]
  db_types: string[]
}

export interface ScannedDbnumConflict {
  dbnum: number
  projects: string[]
}

export interface ScanProjectsResult {
  root: string
  projects: ScannedProject[]
  conflicts: ScannedDbnumConflict[]
  has_conflict: boolean
}

export type ManagedSiteRiskLevel = 'normal' | 'warning' | 'critical'
export type ManagedSiteParseHealthStatus = ManagedSiteRiskLevel | 'unknown'
export type ManagedSiteParsePlanMode = 'Full' | 'Bootstrap' | 'RebuildSystem' | 'Selective' | 'FastReparse'
export type ManagedSiteDbMode = 'file' | 'ws'


export type ManagedRemoteTargetOs = 'ubuntu22' | 'centos79' | 'windows'

export interface ManagedRemoteTarget {
  id: string
  name: string
  target_os: ManagedRemoteTargetOs
  host: string
  ssh_port: number
  ssh_user: string
  password_env: string
  ssh_password?: string | null
  remote_root: string
  remote_db_path: string
  remote_web_port: number
  remote_db_port: number
  public_base_url?: string | null
  surreal_bin: string
  remote_web_bin: string
  auto_prepare: boolean
  upload_web_server: boolean
  upload_surreal: boolean
  upload_resource: boolean
  upload_viewer: boolean
  open_firewall: boolean
  allowed_cidrs: string[]
  web_bind_host: string
  db_bind_host: string
  local_web_bin?: string | null
  local_surreal_bin?: string | null
  local_resource_dir?: string | null
  local_viewer_dir?: string | null
  created_at: string
  updated_at: string
}

export interface ManagedRemoteTargetRequest {
  id?: string | null
  name?: string | null
  target_os?: ManagedRemoteTargetOs | null
  host?: string | null
  ssh_port?: number | null
  ssh_user?: string | null
  password_env?: string | null
  ssh_password?: string | null
  remote_root?: string | null
  remote_db_path?: string | null
  remote_web_port?: number | null
  remote_db_port?: number | null
  public_base_url?: string | null
  surreal_bin?: string | null
  remote_web_bin?: string | null
  auto_prepare?: boolean | null
  upload_web_server?: boolean | null
  upload_surreal?: boolean | null
  upload_resource?: boolean | null
  upload_viewer?: boolean | null
  open_firewall?: boolean | null
  allowed_cidrs?: string[] | null
  web_bind_host?: string | null
  db_bind_host?: string | null
  local_web_bin?: string | null
  local_surreal_bin?: string | null
  local_resource_dir?: string | null
  local_viewer_dir?: string | null
}

export interface ManagedRemoteDeployRequest {
  target_id?: string | null
  target?: ManagedRemoteTargetRequest | null
}

export interface ManagedRemoteDeployStatus {
  site_id: string
  target_id: string
  deploy_id?: string | null
  deploy_task_id?: string | null
  deployment_mode?: string | null
  degraded: boolean
  status: string
  current_step: string
  remote_entry_url?: string | null
  remote_api_base_url?: string | null
  checked_at: string
  last_error?: string | null
  checks: ManagedSitePreflightCheck[]
}

export interface ManagedSiteParseHealth {
  status: ManagedSiteParseHealthStatus
  label: string
  detail: string | null
}

export interface ManagedSiteParsePlanEntry {
  file_name: string
  dbnum?: number | null
  db_type?: string | null
  source: string
  priority: number
}

export interface ManagedSiteParsePlan {
  mode: ManagedSiteParsePlanMode
  label: string
  detail: string
  includes_system_db_files: boolean
  included_db_files: string[]
  /** 「自动解析依赖库」根据 ref0→dbnum 依赖闭包额外纳入的目标文件子集（included_db_files 的子集）。仅预览返回。 */
  auto_related_db_files: string[]
  entries?: ManagedSiteParsePlanEntry[]
  warnings?: string[]
}

export interface ManagedProjectSite {
  site_id: string
  site_name?: string
  projects?: SiteProject[]
  project_name: string
  project_code: number
  project_path: string
  manual_db_nums: number[]
  generate_db_nums: number[]
  parse_db_types: string[]
  force_rebuild_system_db: boolean
  auto_parse_related_dbnums: boolean
  cata_partial_parse?: boolean
  gen_model: boolean
  gen_mesh: boolean
  gen_spatial_tree: boolean
  apply_boolean_operation: boolean
  mesh_tol_ratio: number
  export_json: boolean
  export_parquet: boolean
  pipeline_db_mode: ManagedSiteDbMode
  runtime_db_mode: ManagedSiteDbMode
  config_path: string
  runtime_dir: string
  db_data_path: string
  db_port: number
  web_port: number
  viewer_port?: number | null
  bind_host: string
  public_base_url?: string | null
  associated_project?: string | null
  db_pid: number | null
  web_pid: number | null
  viewer_pid?: number | null
  viewer_url?: string | null
  parse_pid: number | null
  status: ManagedSiteStatus
  parse_status: ManagedSiteParseStatus
  last_error: string | null
  entry_url: string | null
  local_entry_url?: string | null
  public_entry_url?: string | null
  last_parse_started_at?: string | null
  last_parse_finished_at?: string | null
  last_parse_duration_ms?: number | null
  parse_plan: ManagedSiteParsePlan
  risk_level: ManagedSiteRiskLevel
  risk_reasons: string[]
  auto_deploy?: boolean
  deployment_submitted?: boolean
  deployment_error?: string | null
  deployment_task_id?: string | null
  created_at: string
  updated_at: string
}

export interface ManagedSiteProcessResource {
  pid: number | null
  running: boolean
  cpu_usage: number | null
  memory_bytes: number | null
}

export interface ManagedSiteResourceMetrics {
  db_process: ManagedSiteProcessResource
  web_process: ManagedSiteProcessResource
  viewer_process?: ManagedSiteProcessResource
  parse_process: ManagedSiteProcessResource
  runtime_dir_size_bytes: number
  data_dir_size_bytes: number
  runtime_dir_missing: boolean
  data_dir_missing: boolean
  last_parse_started_at: string | null
  last_parse_finished_at: string | null
  last_parse_duration_ms: number | null
}

export interface ManagedSiteRuntimeStatus {
  site_id: string
  status: ManagedSiteStatus
  parse_status: ManagedSiteParseStatus
  parse_plan: ManagedSiteParsePlan
  current_stage: string
  current_stage_label: string
  current_stage_detail: string | null
  db_running: boolean
  web_running: boolean
  viewer_running?: boolean
  parse_running: boolean
  db_pid: number | null
  web_pid: number | null
  viewer_pid?: number | null
  parse_pid: number | null
  sidecar_job_kind?: string | null
  sidecar_job_id?: string | null
  sidecar_job_status?: string | null
  db_port?: number
  web_port?: number
  auto_deploy?: boolean
  viewer_port?: number | null
  viewer_url?: string | null
  entry_url: string | null
  local_entry_url?: string | null
  public_entry_url?: string | null
  db_port_conflict?: boolean
  web_port_conflict?: boolean
  viewer_port_conflict?: boolean
  db_conflict_pids?: number[]
  web_conflict_pids?: number[]
  viewer_conflict_pids?: number[]
  last_error: string | null
  active_log_kind: string | null
  last_log_at: string | null
  recent_log_source: string | null
  recent_log_at: string | null
  last_key_log: string | null
  last_key_log_source: string | null
  recent_activity: ManagedSiteActivitySummary | null
  resources: ManagedSiteResourceMetrics | null
  risk_level: ManagedSiteRiskLevel
  warnings: string[]
  parse_health: ManagedSiteParseHealth
  web_status_ok?: boolean | null
  database_connected?: boolean | null
  surrealdb_connected?: boolean | null
  site_identity_ok?: boolean | null
}

export interface AdminResourceSummary {
  cpu_usage: number | null
  memory_usage: number | null
  disk_usage: number | null
  admin_runtime_size_bytes: number
  managed_data_size_bytes: number
  risk_level: ManagedSiteRiskLevel
  warnings: string[]
  updated_at: string
  message: string | null
}

export interface ManagedSiteActivitySummary {
  source: string
  label: string
  updated_at: string | null
  summary: string | null
}

export interface ManagedSiteLogsResponse {
  site_id: string
  parse_log: string[]
  generate_log?: string[]
  db_log: string[]
  web_log: string[]
  viewer_log?: string[]
  streams: ManagedSiteLogStreamSummary[]
}

export interface ManagedSiteLogStreamSummary {
  key: string
  label: string
  path: string
  exists: boolean
  has_content: boolean
  updated_at: string | null
  line_count: number
  last_line: string | null
  last_key_log: string | null
}

export type ManagedSitePreflightStatus = 'pass' | 'warning' | 'blocking'

export interface ManagedSitePreflightCheck {
  key: string
  label: string
  status: ManagedSitePreflightStatus
  message: string
  detail?: string | null
  action_hint?: string | null
  pids: number[]
}

export interface ManagedSitePreflightReport {
  site_id: string
  ready: boolean
  blocking_count: number
  warning_count: number
  updated_at: string
  checks: ManagedSitePreflightCheck[]
}

export interface ManagedSiteDeployValidationCheck {
  key: string
  label: string
  status: string
  message: string
  detail?: string | null
  url?: string | null
  bytes?: number | null
}

export interface ManagedSiteDeployValidationReport {
  site_id: string
  exists: boolean
  checked_at?: string | null
  blocking_count: number
  warning_count: number
  checks: ManagedSiteDeployValidationCheck[]
}

export interface ManagedSiteActionResponse {
  site_id: string
  action: string
  task_id?: string
}

export interface AppendManagedSiteDbFileRequest {
  db_file: string
  dbnum?: number | null
  stop_running?: boolean
}

export interface AppendManagedSiteDbFileResponse {
  site_id: string
  dbnum: number
  resolved_db_file?: string | null
  already_present: boolean
  stopped_site: boolean
  manual_db_nums: number[]
  site: ManagedProjectSite
  task_id?: string | null
}

export interface QuickDeploySiteRequest {
  /** 目标 db 文件：绝对路径 / 文件名 / 相对 project_path 的路径；仅传绝对路径时后端会自动推断 project_path。 */
  db_file: string
  /** 工程根目录；省略时要求 db_file 为绝对路径。 */
  project_path?: string
  /** E3D 项目名 / 站点显示名；省略时后端按工程目录名和 dbnum 生成。 */
  project_name?: string
  project_code?: number
  dbnum?: number
  auto_parse_related_dbnums?: boolean
  cata_partial_parse?: boolean
  gen_model?: boolean
  gen_mesh?: boolean
  gen_spatial_tree?: boolean
  start_site?: boolean
  web_port?: number
  /** true=等待管线结束；false=后台执行并立即返回 site_id。 */
  wait?: boolean
  force_recreate?: boolean
  pipeline_db_mode?: ManagedSiteDbMode
}

export interface QuickDeploySiteResponse {
  success: boolean
  site_id: string
  dbnum?: number | null
  resolved_db_file?: string | null
  parse_status: string
  generated: boolean
  entry_url?: string | null
  duration_ms: number
  parse_log_tail: string[]
  generate_log_tail: string[]
  warnings: string[]
  message?: string | null
}

export interface ManagedSiteReconcileRequest {
  cleanup_orphans?: boolean
}

export interface ManagedSiteReconcileResponse {
  site_id: string
  changed: boolean
  actions: string[]
  runtime: ManagedSiteRuntimeStatus
}

export interface CreateManagedSiteRequest {
  site_name?: string
  projects?: SiteProject[]
  project_name: string
  project_path: string
  project_code: number
  manual_db_nums?: number[]
  manual_db_files?: string[]
  generate_db_nums?: number[]
  generate_db_files?: string[]
  parse_db_types?: string[]
  force_rebuild_system_db?: boolean
  auto_parse_related_dbnums?: boolean
  cata_partial_parse?: boolean
  gen_model?: boolean
  gen_mesh?: boolean
  gen_spatial_tree?: boolean
  apply_boolean_operation?: boolean
  mesh_tol_ratio?: number
  export_json?: boolean
  export_parquet?: boolean
  pipeline_db_mode?: ManagedSiteDbMode
  runtime_db_mode?: ManagedSiteDbMode
  db_port?: number
  web_port?: number
  auto_deploy?: boolean
  bind_host?: string
  public_base_url?: string
  associated_project?: string
  db_user?: string
  db_password?: string
}

export interface UpdateManagedSiteRequest {
  site_name?: string
  projects?: SiteProject[]
  project_name?: string
  project_path?: string
  project_code?: number
  manual_db_nums?: number[]
  manual_db_files?: string[]
  generate_db_nums?: number[]
  generate_db_files?: string[]
  parse_db_types?: string[]
  force_rebuild_system_db?: boolean
  auto_parse_related_dbnums?: boolean
  cata_partial_parse?: boolean
  gen_model?: boolean
  gen_mesh?: boolean
  gen_spatial_tree?: boolean
  apply_boolean_operation?: boolean
  mesh_tol_ratio?: number
  export_json?: boolean
  export_parquet?: boolean
  pipeline_db_mode?: ManagedSiteDbMode
  runtime_db_mode?: ManagedSiteDbMode
  db_port?: number
  web_port?: number
  bind_host?: string
  public_base_url?: string
  associated_project?: string
  db_user?: string
  db_password?: string
}

export interface PreviewManagedSiteParsePlanRequest {
  site_id?: string
  site_name?: string
  projects?: SiteProject[]
  project_name: string
  project_path: string
  manual_db_nums?: number[]
  manual_db_files?: string[]
  generate_db_nums?: number[]
  generate_db_files?: string[]
  parse_db_types?: string[]
  force_rebuild_system_db?: boolean
  auto_parse_related_dbnums?: boolean
  cata_partial_parse?: boolean
  db_index_path?: string
  web_port: number
  bind_host?: string
  public_base_url?: string
  associated_project?: string
}

export interface SiteStats {
  total: number
  running: number
  error: number
  pending_parse: number
}

/** spec 004：同类型上一次任务的关键指标差值。 */
export interface SiteTaskMetricsDelta {
  prev_task_id: string
  duration_ms: number
  total_elements: number
  inst_relate: number
  closure_visited: number
}

/** spec 004：站点任务级性能指标（一行 = 一次 parse / generate 任务）。 */
export interface SiteTaskMetrics {
  task_id: string
  job_kind: 'parse' | 'generate' | string
  started_at: string
  finished_at?: string | null
  duration_ms: number
  success: boolean
  /** 阶段明细（closure / parse / generate / export，阶段可缺省）。 */
  stages: Record<string, unknown>
  delta?: SiteTaskMetricsDelta
}

export interface SiteTaskMetricsList {
  items: SiteTaskMetrics[]
}
