import type { DatabaseConfig, TaskPriority, TaskType } from '@/types/task'

export type RegistrySiteStatus =
  | 'Configuring'
  | 'Deploying'
  | 'Running'
  | 'Failed'
  | 'Stopped'
  | 'Offline'
  | string

export interface RegistrySite {
  id?: string | null
  site_id: string
  name: string
  description?: string | null
  project_name: string
  project_path?: string | null
  project_code?: number | null
  frontend_url?: string | null
  backend_url?: string | null
  bind_host: string
  bind_port?: number | null
  status: RegistrySiteStatus
  url?: string | null
  health_url?: string | null
  owner?: string | null
  env?: string | null
  region?: string | null
  notes?: string | null
  last_seen_at?: string | null
  last_health_check?: string | null
  created_at?: string | null
  updated_at?: string | null
  config: DatabaseConfig
}

export interface RegistrySitesPageData {
  items: RegistrySite[]
  total: number
  page: number
  per_page: number
  pages: number
}

export interface RegistrySiteQuery {
  q?: string
  status?: string
  region?: string
  page?: number
  per_page?: number
  env?: string
  owner?: string
  project_name?: string
  sort?: string
}

export interface RegistrySiteMutationPayload {
  site_id?: string
  name: string
  description?: string | null
  region?: string | null
  env?: string | null
  project_name?: string | null
  project_path?: string | null
  project_code?: number | null
  frontend_url?: string | null
  backend_url?: string | null
  bind_host?: string | null
  bind_port?: number | null
  owner?: string | null
  health_url?: string | null
  notes?: string | null
  selected_projects?: string[]
  config: DatabaseConfig
}

export interface RegistrySiteImportPayload {
  path?: string
  name?: string
  description?: string
  env?: string
  owner?: string
  region?: string
  site_id?: string
  frontend_url?: string
  backend_url?: string
  bind_host?: string
  bind_port?: number
  health_url?: string
}

export interface RegistrySiteHealthcheckResult {
  healthy: boolean
  item: RegistrySite
}

export interface RegistrySiteExportConfigResult {
  name: string
  config: DatabaseConfig
}

export interface RegistrySiteTaskPayload {
  task_name?: string
  task_type?: TaskType
  priority?: TaskPriority
  config_override?: DatabaseConfig | null
}

export interface RegistrySiteTaskResult {
  task_id: string
  message: string
}

export interface RegistrySiteForm {
  site_id: string
  name: string
  description: string
  region: string
  env: string
  project_name: string
  project_path: string
  project_code: number
  frontend_url: string
  backend_url: string
  bind_host: string
  bind_port: number
  owner: string
  health_url: string
  notes: string
  config_json: string
}
