<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useSitesStore } from '@/stores/sites'
import { usePolling } from '@/composables/usePolling'
import SiteStatsCards from '@/components/sites/SiteStatsCards.vue'
import SiteToolbar from '@/components/sites/SiteToolbar.vue'
import SiteDataTable from '@/components/sites/SiteDataTable.vue'
import SiteDrawer from '@/components/sites/SiteDrawer.vue'

const sitesStore = useSitesStore()

const drawerOpen = ref(false)
const editingSiteId = ref<string | null>(null)
const searchQuery = ref('')
const statusFilter = ref('')

const filteredSites = computed(() => {
  let list = sitesStore.sites
  if (searchQuery.value) {
    const q = searchQuery.value.toLowerCase()
    list = list.filter((s) =>
      s.project_name.toLowerCase().includes(q) || s.site_id.toLowerCase().includes(q)
    )
  }
  if (statusFilter.value) {
    list = list.filter((s) => s.status === statusFilter.value)
  }
  return list
})

function openCreateDrawer() {
  editingSiteId.value = null
  drawerOpen.value = true
}

function handleFilter(search: string, status: string) {
  searchQuery.value = search
  statusFilter.value = status
}

function handleDrawerSaved() {
  drawerOpen.value = false
  editingSiteId.value = null
  sitesStore.fetchSites()
}

const { start: startPolling } = usePolling(async () => {
  await sitesStore.fetchSites()
}, 30000)

onMounted(async () => {
  await sitesStore.fetchSites()
  startPolling()
})
</script>

<template>
  <div class="space-y-6">
    <div>
      <h2 class="text-2xl font-semibold tracking-tight">站点管理</h2>
      <p class="text-sm text-muted-foreground">管理和监控所有项目站点</p>
    </div>
    <SiteStatsCards :stats="sitesStore.stats" />
    <SiteToolbar @open-drawer="openCreateDrawer" @filter="handleFilter" />
    <SiteDataTable :sites="filteredSites" :loading="sitesStore.loading" />
    <SiteDrawer
      :open="drawerOpen"
      :site-id="editingSiteId"
      @close="drawerOpen = false"
      @saved="handleDrawerSaved"
    />
  </div>
</template>
