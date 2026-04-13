import { computed, ref } from 'vue'
import { defineStore } from 'pinia'

import { registryApi } from '@/api/registry'
import type {
  RegistrySite,
  RegistrySiteExportConfigResult,
  RegistrySiteHealthcheckResult,
  RegistrySiteImportPayload,
  RegistrySiteMutationPayload,
  RegistrySiteQuery,
  RegistrySiteTaskPayload,
  RegistrySiteTaskResult,
} from '@/types/registry'

export const useRegistryStore = defineStore('registry', () => {
  const sites = ref<RegistrySite[]>([])
  const loading = ref(false)
  const error = ref('')
  const total = ref(0)
  const page = ref(1)
  const perPage = ref(10)
  const pages = ref(1)
  const query = ref<RegistrySiteQuery>({
    page: 1,
    per_page: 10,
  })

  const stats = computed(() => ({
    total: total.value,
    running: sites.value.filter((site) => site.status === 'Running').length,
    failed: sites.value.filter((site) => site.status === 'Failed').length,
    offline: sites.value.filter((site) => site.status === 'Offline').length,
  }))

  async function fetchSites(overrides?: Partial<RegistrySiteQuery>) {
    loading.value = true
    error.value = ''

    const nextQuery = {
      ...query.value,
      ...overrides,
    }
    query.value = nextQuery

    try {
      const data = await registryApi.list(nextQuery)
      sites.value = data.items ?? []
      total.value = data.total ?? 0
      page.value = data.page ?? 1
      perPage.value = data.per_page ?? 10
      pages.value = data.pages ?? 1
    } catch (err: unknown) {
      error.value = err instanceof Error ? err.message : '加载注册表站点失败'
      throw err
    } finally {
      loading.value = false
    }
  }

  async function fetchSite(id: string) {
    return registryApi.get(id)
  }

  async function createSite(payload: RegistrySiteMutationPayload) {
    const site = await registryApi.create(payload)
    await fetchSites({ page: 1 })
    return site
  }

  async function updateSite(id: string, payload: RegistrySiteMutationPayload) {
    const site = await registryApi.update(id, payload)
    await fetchSites()
    return site
  }

  async function deleteSite(id: string) {
    const result = await registryApi.delete(id)
    const nextPage =
      sites.value.length === 1 && page.value > 1 ? page.value - 1 : page.value
    await fetchSites({ page: nextPage })
    return result
  }

  async function importSite(payload: RegistrySiteImportPayload) {
    const site = await registryApi.importDbOption(payload)
    await fetchSites({ page: 1 })
    return site
  }

  async function healthcheckSite(id: string): Promise<RegistrySiteHealthcheckResult> {
    const result = await registryApi.healthcheck(id)
    const index = sites.value.findIndex((site) => site.site_id === id)
    if (index !== -1) {
      sites.value[index] = result.item
    }
    return result
  }

  async function exportConfig(id: string): Promise<RegistrySiteExportConfigResult> {
    return registryApi.exportConfig(id)
  }

  async function createTask(
    id: string,
    payload: RegistrySiteTaskPayload,
  ): Promise<RegistrySiteTaskResult> {
    return registryApi.createTask(id, payload)
  }

  return {
    sites,
    loading,
    error,
    total,
    page,
    perPage,
    pages,
    query,
    stats,
    fetchSites,
    fetchSite,
    createSite,
    updateSite,
    deleteSite,
    importSite,
    healthcheckSite,
    exportConfig,
    createTask,
  }
})
