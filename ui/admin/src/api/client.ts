import { ofetch } from 'ofetch'

const BASE_URL = import.meta.env.VITE_API_BASE ?? ''

export const api = ofetch.create({
  baseURL: BASE_URL,
  headers: {
    'Content-Type': 'application/json',
  },
  onRequest({ options }) {
    const token = localStorage.getItem('admin_token')
    if (token) {
      const headers = new Headers(options.headers)
      headers.set('Authorization', `Bearer ${token}`)
      options.headers = headers
    }
  },
  onResponseError({ response }) {
    if (response.status === 401) {
      localStorage.removeItem('admin_token')
      window.location.hash = '#/login'
    }
  },
})

export function apiGet<T>(url: string, opts?: { query?: Record<string, unknown> }) {
  return api<T>(url, { method: 'GET', ...opts })
}

export function apiPost<T>(url: string, body?: Record<string, unknown>) {
  return api<T>(url, { method: 'POST', body })
}

export function apiPut<T>(url: string, body?: Record<string, unknown>) {
  return api<T>(url, { method: 'PUT', body })
}

export function apiDelete<T>(url: string) {
  return api<T>(url, { method: 'DELETE' })
}
