import { defineStore } from 'pinia'
import { ref, computed, reactive } from 'vue'
import { extractErrorMessage } from '@/api/client'
import { sitesApi } from '@/api/sites'
import type {
  ManagedProjectSite,
  SiteStats,
  CreateManagedSiteRequest,
  UpdateManagedSiteRequest,
} from '@/types/site'

export type SiteAction = 'parse' | 'start' | 'stop' | 'delete'
export interface SiteActionError {
  siteId: string
  action: SiteAction
  message: string
}

export const useSitesStore = defineStore('sites', () => {
  const sites = ref<ManagedProjectSite[]>([])
  const loading = ref(false)
  const error = ref('')
  const pendingActions = reactive(new Map<string, SiteAction>())
  const actionErrors = reactive(new Map<string, SiteActionError>())
  const latestActionError = ref<SiteActionError | null>(null)

  const stats = computed<SiteStats>(() => ({
    total: sites.value.length,
    running: sites.value.filter((s) => s.status === 'Running').length,
    error: sites.value.filter((s) => s.status === 'Failed').length,
    pending_parse: sites.value.filter((s) => s.parse_status === 'Pending').length,
  }))

  function getSiteAction(siteId: string): SiteAction | undefined {
    return pendingActions.get(siteId)
  }

  function isSiteActionPending(siteId: string): boolean {
    return pendingActions.has(siteId)
  }

  function getSiteActionError(siteId: string): SiteActionError | undefined {
    return actionErrors.get(siteId)
  }

  function clearSiteActionError(siteId: string) {
    actionErrors.delete(siteId)
    if (latestActionError.value?.siteId === siteId) {
      latestActionError.value = null
    }
  }

  async function withAction(siteId: string, action: SiteAction, fn: () => Promise<void>) {
    if (pendingActions.has(siteId)) return
    pendingActions.set(siteId, action)
    clearSiteActionError(siteId)
    try {
      await fn()
    } catch (err: unknown) {
      const message = extractErrorMessage(err)
      const payload = { siteId, action, message }
      actionErrors.set(siteId, payload)
      latestActionError.value = payload
      throw err
    } finally {
      pendingActions.delete(siteId)
    }
  }

  async function fetchSites() {
    loading.value = true
    error.value = ''
    try {
      sites.value = await sitesApi.list()
    } catch (err: unknown) {
      error.value = err instanceof Error ? err.message : 'Failed to fetch sites'
    } finally {
      loading.value = false
    }
  }

  async function createSite(payload: CreateManagedSiteRequest) {
    const site = await sitesApi.create(payload)
    sites.value.push(site)
    return site
  }

  async function updateSite(id: string, payload: UpdateManagedSiteRequest) {
    const updated = await sitesApi.update(id, payload)
    const idx = sites.value.findIndex((s) => s.site_id === id)
    if (idx !== -1) sites.value[idx] = updated
    return updated
  }

  async function deleteSite(id: string) {
    await withAction(id, 'delete', async () => {
      await sitesApi.delete(id)
      sites.value = sites.value.filter((s) => s.site_id !== id)
    })
  }

  async function parseSite(id: string) {
    await withAction(id, 'parse', async () => {
      await sitesApi.parse(id)
      await fetchSites()
    })
  }

  async function startSite(id: string) {
    await withAction(id, 'start', async () => {
      await sitesApi.start(id)
      await fetchSites()
    })
  }

  async function stopSite(id: string) {
    await withAction(id, 'stop', async () => {
      await sitesApi.stop(id)
      await fetchSites()
    })
  }

  return {
    sites, stats, loading, error,
    pendingActions, actionErrors, latestActionError,
    getSiteAction, isSiteActionPending, getSiteActionError, clearSiteActionError,
    fetchSites, createSite, updateSite, deleteSite,
    parseSite, startSite, stopSite,
  }
})
