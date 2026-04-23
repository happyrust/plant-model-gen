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

export type ManagedSiteRiskLevel = 'normal' | 'warning' | 'critical'
export type ManagedSiteParseHealthStatus = ManagedSiteRiskLevel | 'unknown'
export type ManagedSiteParsePlanMode = 'Full' | 'Bootstrap' | 'RebuildSystem' | 'Selective' | 'FastReparse'

export interface ManagedSiteParseHealth {
  status: ManagedSiteParseHealthStatus
  label: string
  detail: string | null
}

export interface ManagedSiteParsePlan {
  mode: ManagedSiteParsePlanMode
  label: string
  detail: string
  includes_system_db_files: boolean
  included_db_files: string[]
}

export interface ManagedProjectSite {
  site_id: string
  project_name: string
  project_code: number
  project_path: string
  manual_db_nums: number[]
  parse_db_types: string[]
  force_rebuild_system_db: boolean
  config_path: string
  runtime_dir: string
  db_data_path: string
  db_port: number
  web_port: number
  bind_host: string
  public_base_url?: string | null
  associated_project?: string | null
  db_pid: number | null
  web_pid: number | null
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
  parse_running: boolean
  db_pid: number | null
  web_pid: number | null
  parse_pid: number | null
  db_port: number
  web_port: number
  entry_url: string | null
  local_entry_url?: string | null
  public_entry_url?: string | null
  db_port_conflict?: boolean
  web_port_conflict?: boolean
  db_conflict_pids?: number[]
  web_conflict_pids?: number[]
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
  db_log: string[]
  web_log: string[]
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

export interface CreateManagedSiteRequest {
  project_name: string
  project_path: string
  project_code: number
  manual_db_nums?: number[]
  parse_db_types?: string[]
  force_rebuild_system_db?: boolean
  db_port: number
  web_port: number
  bind_host?: string
  public_base_url?: string
  associated_project?: string
  db_user?: string
  db_password?: string
}

export interface UpdateManagedSiteRequest {
  project_name?: string
  project_path?: string
  project_code?: number
  manual_db_nums?: number[]
  parse_db_types?: string[]
  force_rebuild_system_db?: boolean
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
  project_name: string
  project_path: string
  manual_db_nums?: number[]
  parse_db_types?: string[]
  force_rebuild_system_db?: boolean
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
