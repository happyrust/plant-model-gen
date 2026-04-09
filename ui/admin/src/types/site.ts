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

export interface ManagedProjectSite {
  site_id: string
  project_name: string
  project_code: number
  project_path: string
  manual_db_nums: number[]
  config_path: string
  runtime_dir: string
  db_data_path: string
  db_port: number
  web_port: number
  bind_host: string
  db_pid: number | null
  web_pid: number | null
  parse_pid: number | null
  status: ManagedSiteStatus
  parse_status: ManagedSiteParseStatus
  last_error: string | null
  entry_url: string | null
  created_at: string
  updated_at: string
}

export interface ManagedSiteRuntimeStatus {
  site_id: string
  status: ManagedSiteStatus
  parse_status: ManagedSiteParseStatus
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
  last_error: string | null
  active_log_kind: string | null
  last_log_at: string | null
  recent_log_source: string | null
  recent_log_at: string | null
  last_key_log: string | null
  last_key_log_source: string | null
  recent_activity: ManagedSiteActivitySummary | null
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
  db_port: number
  web_port: number
  bind_host?: string
  db_user?: string
  db_password?: string
}

export interface UpdateManagedSiteRequest {
  project_name?: string
  project_path?: string
  project_code?: number
  manual_db_nums?: number[]
  db_port?: number
  web_port?: number
  bind_host?: string
  db_user?: string
  db_password?: string
}

export interface SiteStats {
  total: number
  running: number
  error: number
  pending_parse: number
}
