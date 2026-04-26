import { apiGet, apiPost, apiPut, apiDelete, apiPostRaw } from './client'
import type {
  AdminResourceSummary,
  ManagedProjectSite,
  ManagedSiteRuntimeStatus,
  ManagedSiteLogsResponse,
  ManagedSiteParsePlan,
  CreateManagedSiteRequest,
  PreviewManagedSiteParsePlanRequest,
  UpdateManagedSiteRequest,
} from '@/types/site'

export type ManagedSiteLogKind = 'parse' | 'db' | 'web'

export interface TailLogResponse {
  kind: ManagedSiteLogKind
  path: string
  total_lines: number
  returned_lines: number
  truncated: boolean
  limit: number
  lines: string[]
}

export interface PortCheckResult {
  port: number
  host: string | null
  in_use: boolean
  pids: number[]
}

export const sitesApi = {
  resourceSummary: () => apiGet<AdminResourceSummary>('/api/admin/resources/summary'),

  /**
   * D4 / Sprint D · 端口占用预检
   *
   * Drawer 的 db_port / web_port onBlur 时调用，<300ms 反馈是否被本机
   * 其他进程占用，避免提交后才暴露冲突。
   */
  checkPort: (port: number, host?: string) => {
    const params = new URLSearchParams({ port: String(port) })
    if (host) params.set('host', host)
    return apiGet<PortCheckResult>(`/api/admin/ports/check?${params.toString()}`)
  },

  list: () => apiGet<ManagedProjectSite[]>('/api/admin/sites'),

  get: (id: string) => apiGet<ManagedProjectSite>(`/api/admin/sites/${id}`),

  create: (payload: CreateManagedSiteRequest) =>
    apiPost<ManagedProjectSite>('/api/admin/sites', payload as unknown as Record<string, unknown>),

  previewParsePlan: (payload: PreviewManagedSiteParsePlanRequest) =>
    apiPost<ManagedSiteParsePlan>(
      '/api/admin/sites/preview-parse-plan',
      payload as unknown as Record<string, unknown>,
    ),

  update: (id: string, payload: UpdateManagedSiteRequest) =>
    apiPut<ManagedProjectSite>(`/api/admin/sites/${id}`, payload as unknown as Record<string, unknown>),

  delete: (id: string) => apiDelete<{ site_id: string; deleted: boolean }>(`/api/admin/sites/${id}`),

  parse: (id: string) =>
    apiPostRaw<{ site_id: string; action: string }>(`/api/admin/sites/${id}/parse`),

  start: (id: string) =>
    apiPostRaw<{ site_id: string; action: string }>(`/api/admin/sites/${id}/start`),

  stop: (id: string) =>
    apiPost<ManagedProjectSite>(`/api/admin/sites/${id}/stop`),

  restart: (id: string) =>
    apiPostRaw<{ site_id: string; action: string }>(`/api/admin/sites/${id}/restart`),

  runtime: (id: string) =>
    apiGet<ManagedSiteRuntimeStatus>(`/api/admin/sites/${id}/runtime`),

  logs: (id: string) =>
    apiGet<ManagedSiteLogsResponse>(`/api/admin/sites/${id}/logs`),

  /**
   * D5 / Sprint D · 单类日志的分页尾部
   *
   * 默认 limit=200，详情页"加载更多"按钮按 2 倍递增至上限 5000。
   * 后端会钳制 limit 到 [1, 5000]。
   */
  tailLog: (id: string, kind: ManagedSiteLogKind, limit = 200) =>
    apiGet<TailLogResponse>(`/api/admin/sites/${id}/logs/${kind}?limit=${limit}`),

  /**
   * D5 / Sprint D · 单类日志的全量下载链接
   *
   * 返回浏览器原生下载流程使用的 URL（admin auth 由 cookie / Bearer 头承载，
   * 调用方需保证页面已登录）。
   */
  logDownloadUrl: (id: string, kind: ManagedSiteLogKind) =>
    `/api/admin/sites/${id}/logs/${kind}/download`,
}
