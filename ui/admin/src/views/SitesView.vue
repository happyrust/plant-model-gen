<script setup lang="ts">
import { onMounted } from 'vue'
import { useSitesStore } from '@/stores/sites'
import { usePolling } from '@/composables/usePolling'
import SiteStatsCards from '@/components/sites/SiteStatsCards.vue'
import SiteToolbar from '@/components/sites/SiteToolbar.vue'
import SiteDataTable from '@/components/sites/SiteDataTable.vue'

const sitesStore = useSitesStore()

const { start: startPolling } = usePolling(async () => {
  await sitesStore.fetchSites()
  await sitesStore.fetchStats()
}, 30000)

onMounted(async () => {
  await sitesStore.fetchSites()
  await sitesStore.fetchStats()
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
    <SiteToolbar />
    <SiteDataTable :sites="sitesStore.sites" :loading="sitesStore.loading" />
  </div>
</template>
