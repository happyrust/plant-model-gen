import { ofetch } from 'ofetch'

const BASE_URL = import.meta.env.VITE_API_BASE ?? ''

interface ApiEnvelope<T> {
  success: boolean
  message: string
  data: T | null
}

export const rawApi = ofetch.create({
  baseURL: BASE_URL,
  headers: { 'Content-Type': 'application/json' },
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

async function unwrap<T>(promise: Promise<ApiEnvelope<T>>): Promise<T> {
  const envelope = await promise
  if (!envelope.success || envelope.data === null) {
    throw new Error(envelope.message || 'Request failed')
  }
  return envelope.data
}

export function apiGet<T>(url: string, opts?: { query?: Record<string, unknown> }): Promise<T> {
  return unwrap(rawApi<ApiEnvelope<T>>(url, { method: 'GET', ...opts }))
}

export function apiPost<T>(url: string, body?: Record<string, unknown>): Promise<T> {
  return unwrap(rawApi<ApiEnvelope<T>>(url, { method: 'POST', body }))
}

export function apiPut<T>(url: string, body?: Record<string, unknown>): Promise<T> {
  return unwrap(rawApi<ApiEnvelope<T>>(url, { method: 'PUT', body }))
}

export function apiDelete<T>(url: string): Promise<T> {
  return unwrap(rawApi<ApiEnvelope<T>>(url, { method: 'DELETE' }))
}

export function apiPostRaw<T>(url: string, body?: Record<string, unknown>): Promise<ApiEnvelope<T>> {
  return rawApi<ApiEnvelope<T>>(url, { method: 'POST', body })
}
