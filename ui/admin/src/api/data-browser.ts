import { apiGet } from '@/api/client'

export interface DataBrowserTable {
  name: string
}

export interface DataBrowserSiteContext {
  site_id: string
  site_name: string
  project_name: string
  status: string
  runtime_db_mode: 'file' | 'ws'
  db_port: number
}

export interface DataBrowserTablesResponse {
  tables: DataBrowserTable[]
  total: number
  site?: DataBrowserSiteContext
}

export interface DataBrowserRecordsResponse {
  table: string
  columns: string[]
  records: Record<string, unknown>[]
  total: number
  page: number
  per_page: number
  sort: string
  dir: 'asc' | 'desc'
  site?: DataBrowserSiteContext
}

export interface DataBrowserConnectionCredential {
  username: string
  password: string
  role: string
  can_write: boolean
}

export interface DataBrowserConnectionResponse {
  site: DataBrowserSiteContext
  endpoint: string
  namespace: string
  database: string
  mode: 'reader' | 'editor'
  credential: DataBrowserConnectionCredential
  local_only: boolean
}

export type DataBrowserConnectionMode = 'reader' | 'editor'

export interface DataBrowserRecordParams {
  page?: number
  per_page?: number
  sort?: string
  dir?: 'asc' | 'desc'
}

export const dataBrowserApi = {
  tables: () => apiGet<DataBrowserTablesResponse>('/api/admin/data-browser/tables'),

  tablesForSite: (siteId: string) =>
    apiGet<DataBrowserTablesResponse>(`/api/admin/sites/${encodeURIComponent(siteId)}/data-browser/tables`),

  connectionForSite: (siteId: string, mode: DataBrowserConnectionMode = 'reader') =>
    apiGet<DataBrowserConnectionResponse>(
      `/api/admin/sites/${encodeURIComponent(siteId)}/data-browser/connection`,
      { query: { mode } },
    ),

  records: (table: string, params: DataBrowserRecordParams = {}) => {
    const query = new URLSearchParams()
    if (params.page) query.set('page', String(params.page))
    if (params.per_page) query.set('per_page', String(params.per_page))
    if (params.sort) query.set('sort', params.sort)
    if (params.dir) query.set('dir', params.dir)
    const suffix = query.toString()
    return apiGet<DataBrowserRecordsResponse>(
      `/api/admin/data-browser/tables/${encodeURIComponent(table)}/records${suffix ? `?${suffix}` : ''}`,
    )
  },

  recordsForSite: (siteId: string, table: string, params: DataBrowserRecordParams = {}) => {
    const query = new URLSearchParams()
    if (params.page) query.set('page', String(params.page))
    if (params.per_page) query.set('per_page', String(params.per_page))
    if (params.sort) query.set('sort', params.sort)
    if (params.dir) query.set('dir', params.dir)
    const suffix = query.toString()
    return apiGet<DataBrowserRecordsResponse>(
      `/api/admin/sites/${encodeURIComponent(siteId)}/data-browser/tables/${encodeURIComponent(table)}/records${suffix ? `?${suffix}` : ''}`,
    )
  },
}
