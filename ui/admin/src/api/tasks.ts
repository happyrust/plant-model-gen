import { apiGet, apiPost } from './client'
import type { TaskInfo } from '@/types/task'

export const tasksApi = {
  list: (params?: { status?: string; type?: string }) =>
    apiGet<TaskInfo[]>('/api/admin/tasks', { query: params as Record<string, unknown> | undefined }),

  get: (id: string) => apiGet<TaskInfo>(`/api/admin/tasks/${id}`),

  create: (payload: Record<string, unknown>) =>
    apiPost<TaskInfo>('/api/admin/tasks', payload),

  cancel: (id: string) => apiPost<{ task_id: string }>(`/api/admin/tasks/${id}/cancel`),

  retry: (id: string) => apiPost<TaskInfo>(`/api/admin/tasks/${id}/retry`),
}
