import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { authApi } from '@/api/auth'
import { extractErrorMessage } from '@/api/client'
import type { AuthCredentials } from '@/types/api'

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
      token.value = session.token
      username.value = session.user?.username ?? ''
      role.value = session.user?.role ?? ''
      localStorage.setItem('admin_token', session.token)
    } catch (err: unknown) {
      loginError.value = extractErrorMessage(err)
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
      username.value = user?.username ?? ''
      role.value = user?.role ?? ''
    } catch {
      token.value = ''
      username.value = ''
      role.value = ''
      localStorage.removeItem('admin_token')
    }
  }

  return { token, username, role, loginError, loading, isAuthenticated, login, logout, fetchMe }
})
