import { ofetch } from 'ofetch'

const BASE_URL = import.meta.env.VITE_API_BASE ?? ''

export interface ApiEnvelope<T> {
  success: boolean
  message: string
  data: T | null
}

function getEnvelopeMessage(error: unknown): string | undefined {
  if (typeof error !== 'object' || error === null) return undefined
  const responseData = (error as { response?: { _data?: { message?: string } } }).response?._data
  if (responseData?.message) return responseData.message
  const directData = (error as { data?: { message?: string } }).data
  return directData?.message
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

async function unwrapEmpty(promise: Promise<ApiEnvelope<unknown>>): Promise<void> {
  const envelope = await promise
  if (!envelope.success) {
    throw new Error(envelope.message || 'Request failed')
  }
}

export function extractErrorMessage(error: unknown): string {
  return getEnvelopeMessage(error)
    || (error instanceof Error ? error.message : undefined)
    || 'Request failed'
}

export function apiGet<T>(url: string, opts?: { query?: Record<string, unknown> }): Promise<T> {
  return unwrap(rawApi<ApiEnvelope<T>>(url, { method: 'GET', ...opts }))
}

export function apiPost<T>(url: string, body?: Record<string, unknown>): Promise<T> {
  return unwrap(rawApi<ApiEnvelope<T>>(url, { method: 'POST', body }))
}

export function apiPostEmpty(url: string, body?: Record<string, unknown>): Promise<void> {
  return unwrapEmpty(rawApi<ApiEnvelope<unknown>>(url, { method: 'POST', body }))
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
