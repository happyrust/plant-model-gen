export type SiteEnvironment = 'dev' | 'staging' | 'prod'

export type SiteStatus =
  | 'stopped'
  | 'running'
  | 'error'
  | 'parsing'
  | 'pending'

export interface Site {
  id: string
  name: string
  environment: SiteEnvironment
  status: SiteStatus
  port: number
  bind_host: string
  db_nums: number[]
  manual_db_nums: number[]
  user: string
  password?: string
  created_at: string
  updated_at: string
}

export interface SiteStats {
  total: number
  running: number
  error: number
  pending_parse: number
}

export interface SiteRuntime {
  uptime_seconds: number
  memory_mb: number
  cpu_percent: number
  connections: number
  component_counts: Record<string, number>
  parse_progress?: { done: number; total: number }
}

export interface SiteCreatePayload {
  name: string
  environment: SiteEnvironment
  port: number
  bind_host: string
  db_nums?: number[]
  manual_db_nums?: number[]
  user: string
  password: string
}

export type SiteUpdatePayload = Partial<SiteCreatePayload>
