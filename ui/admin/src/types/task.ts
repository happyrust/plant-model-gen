export type TaskType =
  | 'DataGeneration'
  | 'SpatialTreeGeneration'
  | 'FullGeneration'
  | 'MeshGeneration'
  | 'ParsePdmsData'
  | 'GenerateGeometry'
  | 'BuildSpatialIndex'
  | 'BatchDatabaseProcess'
  | 'BatchGeometryGeneration'
  | 'DataExport'
  | 'DataImport'
  | 'DataParsingWizard'
  | 'RefnoModelGeneration'
  | 'ModelExport'
  | { Custom: string }

export type TaskStatus =
  | 'Pending'
  | 'Running'
  | 'Completed'
  | 'Failed'
  | 'Cancelled'

export type TaskPriority = 'Low' | 'Normal' | 'High' | 'Urgent'

export type LogLevel = 'Debug' | 'Info' | 'Warning' | 'Error' | 'Critical'

export interface TaskProgress {
  current_step: string
  total_steps: number
  current_step_number: number
  percentage: number
  processed_items: number
  total_items: number
  estimated_remaining_seconds: number | null
}

export interface LogEntry {
  timestamp: number
  level: LogLevel
  message: string
  error_code: string | null
  stack_trace: string | null
}

export interface ErrorDetails {
  error_type: string
  error_code: string | null
  failed_step: string
  detailed_message: string
  stack_trace: string | null
  suggested_solutions: string[]
  related_config: unknown | null
}

export interface DatabaseConfig {
  name: string
  manual_db_nums: number[]
  manual_refnos: string[]
  enabled_nouns: string[] | null
  excluded_nouns: string[] | null
  debug_limit_per_noun_type: number | null
  project_name: string
  project_path: string
  project_code: number
  mdb_name: string
  module: string
  db_type: string
  surreal_ns: number
  db_ip: string
  db_port: string
  db_user: string
  db_password: string
  gen_model: boolean
  gen_mesh: boolean
  gen_spatial_tree: boolean
  apply_boolean_operation: boolean
  mesh_tol_ratio: number
  room_keyword: string
  target_sesno: number | null
  meshes_path: string | null
  export_json: boolean
  export_parquet: boolean
}

export interface TaskInfo {
  id: string
  name: string
  task_type: TaskType
  status: TaskStatus
  config: DatabaseConfig
  created_at: number
  started_at: number | null
  completed_at: number | null
  progress: TaskProgress
  error: string | null
  error_details: ErrorDetails | null
  logs: LogEntry[]
  priority: TaskPriority
  dependencies: string[]
  estimated_duration: number | null
  actual_duration: number | null
  metadata: unknown | null
}

export const TASK_TYPE_LABELS: Record<string, string> = {
  DataGeneration: '数据生成',
  SpatialTreeGeneration: '空间树生成',
  FullGeneration: '完整生成',
  MeshGeneration: '网格生成',
  ParsePdmsData: 'PDMS 数据解析',
  GenerateGeometry: '几何生成',
  BuildSpatialIndex: '空间索引构建',
  BatchDatabaseProcess: '批量数据库处理',
  BatchGeometryGeneration: '批量几何生成',
  DataExport: '数据导出',
  DataImport: '数据导入',
  DataParsingWizard: '数据解析向导',
  RefnoModelGeneration: 'Refno 模型生成',
  ModelExport: '模型导出',
}

export const PRIORITY_LABELS: Record<TaskPriority, string> = {
  Low: '低',
  Normal: '普通',
  High: '高',
  Urgent: '紧急',
}

export function getTaskTypeLabel(t: TaskType): string {
  if (typeof t === 'string') return TASK_TYPE_LABELS[t] ?? t
  if (typeof t === 'object' && 'Custom' in t) return `自定义: ${t.Custom}`
  return String(t)
}
