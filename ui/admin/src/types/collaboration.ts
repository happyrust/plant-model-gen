export type CollaborationTone = 'default' | 'success' | 'warning' | 'danger'
export type CollaborationSiteAvailability = 'online' | 'cached' | 'offline' | 'unknown'
export type CollaborationDiagnosticStatus = 'idle' | 'running' | 'success' | 'failed'

export interface CollaborationEnv {
  id: string
  name: string
  mqtt_host: string | null
  mqtt_port: number | null
  file_server_host: string | null
  location: string | null
  location_dbs: string | null
  reconnect_initial_ms: number | null
  reconnect_max_ms: number | null
  created_at: string
  updated_at: string
}

export interface CollaborationSite {
  id: string
  env_id: string
  name: string
  location: string | null
  http_host: string | null
  dbnums: string | null
  notes: string | null
  created_at: string
  updated_at: string
}

export interface CollaborationRuntimeStatus {
  status: string
  active: boolean
  env_id: string | null
  mqtt_connected: boolean | null
}

export interface CollaborationRuntimeConfig {
  mqtt_host: string | null
  mqtt_port: number | null
  file_server_host: string | null
  location: string | null
  location_dbs: number[]
  sync_live: boolean
  source: string | null
}

export interface CollaborationLogRecord {
  id: string
  task_id: string | null
  env_id: string | null
  source_env: string | null
  target_site: string | null
  site_id: string | null
  direction: string | null
  file_path: string | null
  file_size: number | null
  record_count: number | null
  status: string
  error_message: string | null
  notes: string | null
  started_at: string | null
  completed_at: string | null
  created_at: string
  updated_at: string
}

export interface CollaborationLogFilters {
  status: string
  direction: string
  target_site: string
  keyword: string
}

export interface CollaborationLogsResult {
  items: CollaborationLogRecord[]
  total: number
  limit: number
  offset: number
}

export interface CollaborationDailyStat {
  day: string
  total: number
  completed: number
  failed: number
  record_count: number
  total_bytes: number
}

export interface CollaborationFlowStat {
  env_id: string
  target_site: string
  direction: string
  total: number
  completed: number
  failed: number
  record_count: number
  total_bytes: number
}

export interface CollaborationSiteMetadataEntry {
  file_name: string
  file_path: string
  file_size: number
  file_hash: string | null
  record_count: number | null
  direction: string | null
  source_env: string | null
  download_url: string | null
  relative_path: string | null
  updated_at: string
}

export interface CollaborationMetadataFile {
  env_id: string | null
  env_name: string | null
  site_id: string | null
  site_name: string | null
  site_http_host: string | null
  generated_at: string
  entries: CollaborationSiteMetadataEntry[]
}

export interface CollaborationSiteMetadataResponse {
  status: string
  source: string
  fetched_at: string
  entry_count: number
  cache_path: string | null
  http_base: string | null
  local_base: string | null
  warnings: string[]
  env: {
    id: string
    name: string | null
    file_host: string | null
  }
  site: {
    id: string
    name: string
    host: string | null
  }
  metadata: CollaborationMetadataFile
}

export interface CollaborationSiteMetadataState {
  siteId: string
  state: 'loading' | 'ready' | 'error'
  source: string | null
  fetchedAt: string | null
  entryCount: number
  totalRecordCount: number
  latestUpdatedAt: string | null
  warningCount: number
  message: string | null
}

export interface CollaborationDiagnosticCheck {
  status: CollaborationDiagnosticStatus
  message: string
  checkedAt: string | null
  addr: string | null
  url: string | null
  code: number | null
  latencyMs: number | null
}

export interface CollaborationEnvDiagnostics {
  mqtt: CollaborationDiagnosticCheck
  http: CollaborationDiagnosticCheck
}

export interface CollaborationSiteDiagnostics {
  siteId: string
  siteName: string
  check: CollaborationDiagnosticCheck
}

export interface CollaborationDiagnosticsSummary {
  status: CollaborationDiagnosticStatus
  label: string
  detail: string
  checkedAt: string | null
}

export interface CollaborationRuntimeBindingState {
  label: string
  tone: CollaborationTone
  runtimeEnvLabel: string
  configSourceLabel: string
  relationLabel: string
  relationDetail: string
  lastActionMessage: string | null
  lastActionTone: CollaborationTone
  lastActionAt: string | null
}

export interface CollaborationRuntimeActionState {
  status: 'idle' | 'success' | 'failed'
  message: string
  at: string | null
}

export interface CollaborationRuntimeControlSummary {
  label: string
  tone: CollaborationTone
  detail: string
  runtimeEnvName: string
  configSource: string
  lastAction: CollaborationRuntimeActionState
}

export interface CollaborationActionResult {
  status: string
  message?: string
  id?: string
  env_id?: string
}

export interface CollaborationControlMessage {
  status: 'success' | 'failed' | 'idle'
  message: string
  at: string | null
}

export interface CollaborationEffectiveStateSummary {
  label: string
  tone: CollaborationTone
  runtimeEnvName: string
  runtimeEnvDetail: string
  configSource: string
  configSourceDetail: string
  relationDetail: string
  lastAction: CollaborationControlMessage
}

export interface CollaborationOverviewMetric {
  id: string
  label: string
  value: string
  detail: string
  tone: CollaborationTone
}

export interface CollaborationInsightsSummary {
  total7d: number
  total14d: number
  completed: number
  failed: number
  successRate: number
  totalRecords: number
  totalBytes: number
  alertCount: number
  busiestFlow: CollaborationFlowStat | null
  riskiestFlow: CollaborationFlowStat | null
  lastLogAt: string | null
  trend14d: CollaborationDailyStat[]
  topFailedFlows: CollaborationFlowStat[]
  recentFailures: CollaborationLogRecord[]
}

export interface CollaborationGroupListItem {
  id: string
  name: string
  location: string | null
  mqttSummary: string
  siteCount: number
  updatedAt: string
  isActive: boolean
  statusLabel: string
  statusTone: CollaborationTone
}

export interface CollaborationSiteCard {
  id: string
  name: string
  location: string | null
  httpHost: string | null
  dbnums: string | null
  dbnumList: number[]
  notes: string | null
  roleLabel: string
  availability: CollaborationSiteAvailability
  availabilityLabel: string
  connectionSummary: string
  metadataSourceLabel: string
  fileCount: number
  totalRecordCount: number
  latestUpdatedAt: string | null
  warningCount: number
  metadataMessage: string | null
  diagnosticStatus: CollaborationDiagnosticStatus
  diagnosticStatusLabel: string
  diagnosticTone: CollaborationTone
  diagnosticCheckedAt: string | null
  diagnosticMessage: string | null
  diagnosticUrl: string | null
  diagnosticCode: number | null
  diagnosticLatencyMs: number | null
  diagnosticPending: boolean
}

export interface CollaborationOption {
  value: string
  label: string
}

export interface CollaborationDiagnosticResponse {
  status: 'success' | 'failed'
  message?: string
  checked_at?: string | null
  addr?: string | null
  url?: string | null
  code?: number | null
  latency_ms?: number | null
}

export interface CollaborationControlResponse {
  status: 'success' | 'failed'
  message?: string
  id?: string | null
  env_id?: string | null
}

export interface CollaborationActionResponse {
  status: 'success' | 'failed'
  message?: string
  id?: string | null
  env_id?: string | null
}

export interface CreateCollaborationEnvRequest {
  name: string
  mqtt_host: string | null
  mqtt_port: number | null
  file_server_host: string | null
  location: string | null
  location_dbs: string | null
  reconnect_initial_ms: number | null
  reconnect_max_ms: number | null
}

export type UpdateCollaborationEnvRequest = CreateCollaborationEnvRequest

export interface CreateCollaborationSiteRequest {
  name: string
  location: string | null
  http_host: string | null
  dbnums: string | null
  notes: string | null
}

export type UpdateCollaborationSiteRequest = CreateCollaborationSiteRequest
