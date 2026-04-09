import { defineStore } from 'pinia'
import { ref } from 'vue'
import { sitesApi } from '@/api/sites'
import type { Site, SiteStats, SiteCreatePayload, SiteUpdatePayload } from '@/types/site'

export const useSitesStore = defineStore('sites', () => {
  const sites = ref<Site[]>([])
  const stats = ref<SiteStats>({ total: 0, running: 0, error: 0, pending_parse: 0 })
  const loading = ref(false)
  const error = ref('')

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

  async function fetchStats() {
    try {
      stats.value = await sitesApi.stats()
    } catch {
      // stats are non-critical; derive from local list as fallback
      stats.value = {
        total: sites.value.length,
        running: sites.value.filter((s) => s.status === 'running').length,
        error: sites.value.filter((s) => s.status === 'error').length,
        pending_parse: sites.value.filter((s) => s.status === 'pending').length,
      }
    }
  }

  async function createSite(payload: SiteCreatePayload) {
    const site = await sitesApi.create(payload)
    sites.value.push(site)
    return site
  }

  async function updateSite(id: string, payload: SiteUpdatePayload) {
    const updated = await sitesApi.update(id, payload)
    const idx = sites.value.findIndex((s) => s.id === id)
    if (idx !== -1) sites.value[idx] = updated
    return updated
  }

  async function deleteSite(id: string) {
    await sitesApi.delete(id)
    sites.value = sites.value.filter((s) => s.id !== id)
  }

  async function parseSite(id: string) {
    await sitesApi.parse(id)
    await fetchSites()
  }

  async function startSite(id: string) {
    await sitesApi.start(id)
    await fetchSites()
  }

  async function stopSite(id: string) {
    await sitesApi.stop(id)
    await fetchSites()
  }

  return {
    sites, stats, loading, error,
    fetchSites, fetchStats, createSite, updateSite, deleteSite,
    parseSite, startSite, stopSite,
  }
})
