import { apiDelete, apiGet, apiPost, apiPut } from '@/api/client'
import type {
  RegistrySite,
  RegistrySiteExportConfigResult,
  RegistrySiteHealthcheckResult,
  RegistrySiteImportPayload,
  RegistrySiteMutationPayload,
  RegistrySiteQuery,
  RegistrySitesPageData,
  RegistrySiteTaskPayload,
  RegistrySiteTaskResult,
} from '@/types/registry'

export const registryApi = {
  list: (query?: RegistrySiteQuery) =>
    apiGet<RegistrySitesPageData>('/api/admin/registry/sites', {
      query: query as Record<string, unknown> | undefined,
    }),

  get: (id: string) => apiGet<RegistrySite>(`/api/admin/registry/sites/${id}`),

  create: (payload: RegistrySiteMutationPayload) =>
    apiPost<RegistrySite>(
      '/api/admin/registry/sites',
      payload as unknown as Record<string, unknown>,
    ),

  update: (id: string, payload: RegistrySiteMutationPayload) =>
    apiPut<RegistrySite>(
      `/api/admin/registry/sites/${id}`,
      payload as unknown as Record<string, unknown>,
    ),

  delete: (id: string) =>
    apiDelete<{ site_id: string; deleted: boolean }>(`/api/admin/registry/sites/${id}`),

  importDbOption: (payload: RegistrySiteImportPayload) =>
    apiPost<RegistrySite>(
      '/api/admin/registry/import-dboption',
      payload as unknown as Record<string, unknown>,
    ),

  healthcheck: (id: string) =>
    apiPost<RegistrySiteHealthcheckResult>(`/api/admin/registry/sites/${id}/healthcheck`),

  exportConfig: (id: string) =>
    apiGet<RegistrySiteExportConfigResult>(`/api/admin/registry/sites/${id}/export-config`),

  createTask: (id: string, payload: RegistrySiteTaskPayload) =>
    apiPost<RegistrySiteTaskResult>(
      `/api/admin/registry/sites/${id}/tasks`,
      payload as unknown as Record<string, unknown>,
    ),
}
