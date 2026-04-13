import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { authApi } from '@/api/auth'
import type { AuthCredentials, AuthSession, AuthUser } from '@/types/api'

function resolveSessionUser(session: AuthSession): AuthUser {
  return {
    username: session.user?.username ?? session.username ?? '',
    role: session.user?.role ?? session.role ?? '',
  }
}

export const useAuthStore = defineStore('auth', () => {
  const token = ref(localStorage.getItem('admin_token') ?? '')
  const username = ref('')
  const role = ref('')
  const loginError = ref('')
  const loading = ref(false)

  const isAuthenticated = computed(() => !!token.value)

  async function login(creds: AuthCredentials) {
    loading.value = true
    loginError.value = ''
    try {
      const session = await authApi.login(creds)
      const user = resolveSessionUser(session)
      token.value = session.token
      username.value = user.username
      role.value = user.role
      localStorage.setItem('admin_token', session.token)
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : 'Login failed'
      loginError.value = message
      throw err
    } finally {
      loading.value = false
    }
  }

  async function logout() {
    try {
      await authApi.logout()
    } finally {
      token.value = ''
      username.value = ''
      role.value = ''
      localStorage.removeItem('admin_token')
    }
  }

  async function fetchMe() {
    if (!token.value) return
    try {
      const user = await authApi.me()
      username.value = user.username
      role.value = user.role
    } catch {
      token.value = ''
      localStorage.removeItem('admin_token')
    }
  }

  return { token, username, role, loginError, loading, isAuthenticated, login, logout, fetchMe }
})
