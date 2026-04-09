import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { sitesApi } from '@/api/sites'
import type {
  ManagedProjectSite,
  SiteStats,
  CreateManagedSiteRequest,
  UpdateManagedSiteRequest,
} from '@/types/site'

export const useSitesStore = defineStore('sites', () => {
  const sites = ref<ManagedProjectSite[]>([])
  const loading = ref(false)
  const error = ref('')

  const stats = computed<SiteStats>(() => ({
    total: sites.value.length,
    running: sites.value.filter((s) => s.status === 'Running').length,
    error: sites.value.filter((s) => s.status === 'Failed').length,
    pending_parse: sites.value.filter((s) => s.parse_status === 'Pending').length,
  }))

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
    await sitesApi.delete(id)
    sites.value = sites.value.filter((s) => s.site_id !== id)
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
    fetchSites, createSite, updateSite, deleteSite,
    parseSite, startSite, stopSite,
  }
})
