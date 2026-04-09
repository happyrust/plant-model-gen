import { apiGet, apiPost } from './client'
import type { Task, SubTask } from '@/types/task'

export const tasksApi = {
  list: (params?: { status?: string; type?: string }) =>
    apiGet<Task[]>('/api/admin/tasks', { query: params as Record<string, unknown> | undefined }),

  get: (id: string) => apiGet<Task>(`/api/admin/tasks/${id}`),

  create: (payload: Partial<Task>) =>
    apiPost<Task>('/api/admin/tasks', payload as unknown as Record<string, unknown>),

  cancel: (id: string) => apiPost(`/api/admin/tasks/${id}/cancel`),

  retry: (id: string) => apiPost(`/api/admin/tasks/${id}/retry`),

  subtasks: (id: string) =>
    apiGet<SubTask[]>(`/api/admin/tasks/${id}/subtasks`),

  logs: (id: string) => apiGet<string[]>(`/api/admin/tasks/${id}/logs`),
}
