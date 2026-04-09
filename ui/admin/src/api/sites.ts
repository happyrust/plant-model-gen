import { apiGet, apiPost, apiPut, apiDelete, apiPostRaw } from './client'
import type {
  ManagedProjectSite,
  ManagedSiteRuntimeStatus,
  ManagedSiteLogsResponse,
  CreateManagedSiteRequest,
  UpdateManagedSiteRequest,
} from '@/types/site'

export const sitesApi = {
  list: () => apiGet<ManagedProjectSite[]>('/api/admin/sites'),

  get: (id: string) => apiGet<ManagedProjectSite>(`/api/admin/sites/${id}`),

  create: (payload: CreateManagedSiteRequest) =>
    apiPost<ManagedProjectSite>('/api/admin/sites', payload as unknown as Record<string, unknown>),

  update: (id: string, payload: UpdateManagedSiteRequest) =>
    apiPut<ManagedProjectSite>(`/api/admin/sites/${id}`, payload as unknown as Record<string, unknown>),

  delete: (id: string) => apiDelete<{ site_id: string; deleted: boolean }>(`/api/admin/sites/${id}`),

  parse: (id: string) =>
    apiPostRaw<{ site_id: string; action: string }>(`/api/admin/sites/${id}/parse`),

  start: (id: string) =>
    apiPostRaw<{ site_id: string; action: string }>(`/api/admin/sites/${id}/start`),

  stop: (id: string) =>
    apiPost<ManagedProjectSite>(`/api/admin/sites/${id}/stop`),

  runtime: (id: string) =>
    apiGet<ManagedSiteRuntimeStatus>(`/api/admin/sites/${id}/runtime`),

  logs: (id: string) =>
    apiGet<ManagedSiteLogsResponse>(`/api/admin/sites/${id}/logs`),
}
