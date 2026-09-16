import { apiGet, apiPost, apiPostEmpty } from './client'
import type { AuthCredentials, AuthSession } from '@/types/api'

export const authApi = {
  login: (creds: AuthCredentials) =>
    apiPost<AuthSession>('/api/admin/auth/login', creds as unknown as Record<string, unknown>),

  logout: () => apiPostEmpty('/api/admin/auth/logout'),

  me: () => apiGet<AuthSession['user']>('/api/admin/auth/me'),
}
