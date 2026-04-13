import { apiGet, apiPost } from './client'
import type { AuthCredentials, AuthSession, AuthUser } from '@/types/api'

export const authApi = {
  login: (creds: AuthCredentials) =>
    apiPost<AuthSession>('/api/admin/auth/login', creds as unknown as Record<string, unknown>),

  logout: () => apiPost('/api/admin/auth/logout'),

  me: () => apiGet<AuthUser>('/api/admin/auth/me'),
}
