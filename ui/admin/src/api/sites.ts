import { apiGet, apiPost, apiPut, apiDelete } from './client'
import type { Site, SiteStats, SiteRuntime, SiteCreatePayload, SiteUpdatePayload } from '@/types/site'

export const sitesApi = {
  list: () => apiGet<Site[]>('/api/admin/sites'),

  stats: () => apiGet<SiteStats>('/api/admin/sites/stats'),

  get: (id: string) => apiGet<Site>(`/api/admin/sites/${id}`),

  create: (payload: SiteCreatePayload) =>
    apiPost<Site>('/api/admin/sites', payload as unknown as Record<string, unknown>),

  update: (id: string, payload: SiteUpdatePayload) =>
    apiPut<Site>(`/api/admin/sites/${id}`, payload as unknown as Record<string, unknown>),

  delete: (id: string) => apiDelete(`/api/admin/sites/${id}`),

  parse: (id: string) => apiPost(`/api/admin/sites/${id}/parse`),

  start: (id: string) => apiPost(`/api/admin/sites/${id}/start`),

  stop: (id: string) => apiPost(`/api/admin/sites/${id}/stop`),

  runtime: (id: string) => apiGet<SiteRuntime>(`/api/admin/sites/${id}/runtime`),

  logs: (id: string, lines?: number) =>
    apiGet<string[]>(`/api/admin/sites/${id}/logs`, {
      query: lines ? { lines } : undefined,
    }),
}
