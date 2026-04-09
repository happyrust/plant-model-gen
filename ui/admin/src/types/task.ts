export type TaskType = 'parse' | 'gen_model' | 'export'

export type TaskStatus =
  | 'pending'
  | 'running'
  | 'completed'
  | 'failed'
  | 'cancelled'

export type TaskPriority = 'low' | 'medium' | 'high'

export interface ParseConfig {
  mode: 'full' | 'by_dbnum' | 'by_ref'
  db_nums?: number[]
  ref_range?: { start: string; end: string }
  max_concurrency: number
}

export interface GenModelConfig {
  generate_model: boolean
  generate_mesh: boolean
  generate_spatial_tree: boolean
  generate_boolean_ops: boolean
  mesh_tolerance?: number
  max_concurrency: number
  export_web_package: boolean
}

export interface ExportConfig {
  format: string
  max_concurrency: number
}

export type TaskConfig = ParseConfig | GenModelConfig | ExportConfig

export interface Task {
  id: string
  name: string
  type: TaskType
  status: TaskStatus
  priority: TaskPriority
  description?: string
  config: TaskConfig
  progress_percent: number
  created_at: string
  started_at?: string
  completed_at?: string
  site_id: string
}

export interface SubTask {
  id: string
  task_id: string
  name: string
  status: TaskStatus
  progress_percent: number
  duration_ms?: number
  error?: string
}
